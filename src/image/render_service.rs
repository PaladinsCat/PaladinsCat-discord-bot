//! Queue + cache + recovery wrapper — mirrors TS `render-service.ts`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde_json::Value;

use crate::cache::RenderCache;
use crate::image::match_renderer::MatchRenderer;
use crate::image::render_queue::{BoundedWorkQueue, QueueSnapshot};

#[derive(Debug, Clone)]
pub struct ImageServiceConfig {
    pub concurrency: usize,
    pub queue_limit: usize,
    pub timeout_ms: u64,
    pub cache_bytes: usize,
    pub cache_ttl_secs: u64,
}

impl Default for ImageServiceConfig {
    fn default() -> Self {
        Self {
            concurrency: 1,
            queue_limit: 10,
            // Match the TypeScript production budget. The command-level 12s
            // timeout remains the hang boundary and recycles a stalled browser.
            timeout_ms: 20_000,
            cache_bytes: 32 * 1024 * 1024,
            cache_ttl_secs: 600,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServiceSnapshot {
    pub queue: QueueSnapshot,
    pub cache_entries: u64,
    pub cache_bytes: u64,
    pub deduplicated: usize,
    pub render_retries: usize,
    pub browser_recoveries: usize,
    pub render_attempt_timeout_ms: u64,
}

#[derive(Debug)]
struct ServiceStats {
    deduplicated: usize,
    render_retries: usize,
    browser_recoveries: usize,
}

impl Default for ServiceStats {
    fn default() -> Self {
        Self {
            deduplicated: 0,
            render_retries: 0,
            browser_recoveries: 0,
        }
    }
}

pub struct ImageService {
    renderer: Arc<MatchRenderer>,
    cache: RenderCache,
    queue: BoundedWorkQueue<Vec<u8>>,
    in_flight_matches: StdMutex<HashMap<String, Arc<tokio::sync::Mutex<Option<Vec<u8>>>>>>,
    render_attempt_timeout_ms: u64,
    stats: StdMutex<ServiceStats>,
}

impl ImageService {
    pub fn new(renderer: Arc<MatchRenderer>, config: ImageServiceConfig) -> Self {
        let render_attempt_timeout_ms = std::cmp::max(
            1,
            std::cmp::min(6000, (config.timeout_ms as f64 * 0.4) as u64),
        );
        Self {
            renderer,
            cache: RenderCache::new(config.cache_bytes, config.cache_ttl_secs),
            queue: BoundedWorkQueue::new(
                // The canonical exporter uses one shared CDP page. More queue
                // permits only make later requests spend their budget waiting
                // on `render_lock`; serialize here and report the wait.
                1,
                config.queue_limit,
                config.timeout_ms,
                "Render",
            ),
            in_flight_matches: StdMutex::new(HashMap::new()),
            render_attempt_timeout_ms,
            stats: StdMutex::new(ServiceStats::default()),
        }
    }

    pub async fn render_match(
        &self,
        record: &Value,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let match_id = value_id(record["match"].get("match_id"));
        let cache_key = format!(
            "match:{}:summary:v{}",
            match_id,
            self.renderer.template_version()
        );

        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(decode_b64(&cached));
        }

        let result = self
            .queue
            .add(match_id.clone(), || async {
                self.render_with_recovery(|| async { self.renderer.render(record).await })
                    .await
            })
            .await;
        let result = self.finish_queued_render(result).await?;

        self.cache.set(cache_key, encode_b64(&result)).await;
        Ok(result)
    }

    /// Return a completed match render without invoking the backend or browser.
    /// This preserves the cache-first path for repeated match commands.
    pub async fn cached_match(&self, match_id: &str) -> Option<Vec<u8>> {
        let cache_key = format!(
            "match:{}:summary:v{}",
            match_id,
            self.renderer.template_version()
        );
        self.cache
            .get(&cache_key)
            .await
            .map(|cached| decode_b64(&cached))
            .filter(|png| !png.is_empty())
    }

    pub async fn render_web_match(
        &self,
        match_id: &str,
        url: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let cache_key = format!(
            "match:{}:summary:v{}",
            match_id,
            self.renderer.template_version()
        );
        if let Some(cached) = self.cached_match(match_id).await {
            return Ok(cached);
        }
        let result = self
            .queue
            .add(match_id.to_string(), || async {
                self.render_with_recovery(|| async { self.renderer.render_web_match(url).await })
                    .await
            })
            .await;
        let result = self.finish_queued_render(result).await?;
        self.cache.set(cache_key, encode_b64(&result)).await;
        Ok(result)
    }

    pub async fn render_loadout(
        &self,
        record: &Value,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let player_id = value_id(record["player"].get("id"));
        let loadout_id = value_id(record["loadout"].get("id"));
        let updated_at = record["loadout"]["updated_at"].as_str().unwrap_or(
            record["loadout"]["fetched_at"]
                .as_str()
                .unwrap_or("unknown"),
        );
        let cache_key = format!(
            "loadout:{}:{}:{}:v{}",
            player_id,
            loadout_id,
            updated_at,
            self.renderer.loadout_template_version()
        );

        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(decode_b64(&cached));
        }

        let result = self
            .queue
            .add(cache_key.clone(), || async {
                self.render_with_recovery(|| async { self.renderer.render_loadout(record).await })
                    .await
            })
            .await;
        let result = self.finish_queued_render(result).await?;

        self.cache.set(cache_key, encode_b64(&result)).await;
        Ok(result)
    }

    pub async fn match_by_id<F>(
        &self,
        match_id: String,
        load: F,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnOnce() -> Box<
            dyn std::future::Future<
                    Output = Result<Value, Box<dyn std::error::Error + Send + Sync>>,
                > + Send,
        >,
    {
        let cache_key = format!(
            "match:{}:summary:v{}",
            match_id,
            self.renderer.template_version()
        );
        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(decode_b64(&cached));
        }

        let holder = self.get_or_create_in_flight(&match_id);
        {
            let guard = holder.lock().await;
            if let Some(result) = guard.as_ref() {
                return Ok(result.clone());
            }
        }

        // SAFETY: `load()` returns a `impl Future` which is always `Unpin`
        let record = unsafe {
            let f = std::pin::Pin::new_unchecked(load());
            f.await?
        };
        let result = self
            .render_with_recovery(|| async { self.renderer.render(&record).await })
            .await?;

        {
            let mut guard = holder.lock().await;
            *guard = Some(result.clone());
        }

        self.cache.set(cache_key, encode_b64(&result)).await;
        Ok(result)
    }

    pub async fn warm(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.renderer.warm().await
    }

    pub async fn close(&self) {
        self.renderer.close().await;
    }

    /// Discard a renderer left mid-request by a caller-level timeout.
    /// The command timeout cancels its future before `render_with_recovery`
    /// can observe an error, so it must explicitly reset Chromium.
    pub async fn recycle(&self) {
        self.renderer.recycle().await;
    }

    pub fn snapshot(&self) -> ServiceSnapshot {
        let stats = self.stats.lock().unwrap();
        ServiceSnapshot {
            queue: self.queue.snapshot(),
            cache_entries: self.cache.entry_count(),
            cache_bytes: self.cache.approximate_bytes(),
            deduplicated: stats.deduplicated,
            render_retries: stats.render_retries,
            browser_recoveries: stats.browser_recoveries,
            render_attempt_timeout_ms: self.render_attempt_timeout_ms,
        }
    }

    async fn finish_queued_render(
        &self,
        result: Result<Vec<u8>, crate::image::render_queue::QueueFullError>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                // Queue timeout drops the renderer future before its own
                // recovery loop can observe the failure. Reset the shared CDP
                // page so the next command never inherits that pending call.
                if error.is_work_timeout() {
                    self.renderer.recycle().await;
                    self.stats.lock().unwrap().browser_recoveries += 1;
                }
                Err(Box::new(error))
            }
        }
    }

    async fn render_with_recovery<F, Fut>(
        &self,
        mut render: F,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnMut() -> Fut,
        Fut:
            std::future::Future<Output = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>>,
    {
        for attempt in 0..2 {
            match render().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    self.renderer.recycle().await;
                    let mut s = self.stats.lock().unwrap();
                    s.browser_recoveries += 1;
                    if attempt == 0 {
                        s.render_retries += 1;
                    } else {
                        drop(s);
                        return Err(e);
                    }
                }
            }
        }
        Err("Render recovery exhausted".into())
    }

    async fn render_with_dedup<F, Fut>(
        &self,
        match_id: &str,
        render: F,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnOnce() -> Fut,
        Fut:
            std::future::Future<Output = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>>,
    {
        let holder = self.get_or_create_in_flight(match_id);
        {
            let guard = holder.lock().await;
            if let Some(result) = guard.as_ref() {
                self.stats.lock().unwrap().deduplicated += 1;
                return Ok(result.clone());
            }
        }

        let result = render().await?;
        {
            let mut guard = holder.lock().await;
            *guard = Some(result.clone());
        }
        Ok(result)
    }

    fn get_or_create_in_flight(&self, match_id: &str) -> Arc<tokio::sync::Mutex<Option<Vec<u8>>>> {
        let mut map = self.in_flight_matches.lock().unwrap();
        match map.get(match_id) {
            Some(h) => Arc::clone(h),
            None => {
                let holder = Arc::new(tokio::sync::Mutex::new(None));
                map.insert(match_id.to_string(), Arc::clone(&holder));
                holder
            }
        }
    }
}

fn encode_b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_b64(s: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .unwrap_or_default()
}

fn value_id(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(value)) => value.clone(),
        Some(serde_json::Value::Number(value)) => value.to_string(),
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_ts_render_budget() {
        let config = ImageServiceConfig::default();
        assert_eq!(config.concurrency, 1);
        assert_eq!(config.queue_limit, 10);
        assert_eq!(config.timeout_ms, 20_000);
        assert_eq!(config.cache_bytes, 32 * 1024 * 1024);
        assert_eq!(config.cache_ttl_secs, 600);
    }

    #[test]
    fn encode_b64_does_not_panic_on_high_bytes() {
        // Regression: the old hand-rolled encoder indexed a 64-char table with
        // unmasked 8-bit values (e.g. (a<<4)|(b>>4) up to 255), panicking with
        // "index out of bounds" on real PNG bytes. Must round-trip cleanly.
        for len in 0..300 {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let encoded = encode_b64(&data);
            let decoded = decode_b64(&encoded);
            assert_eq!(decoded, data, "round-trip failed for len {len}");
        }
    }

    #[test]
    fn cache_ids_accept_json_strings_and_numbers() {
        assert_eq!(
            value_id(Some(&serde_json::json!(1281335238u64))),
            "1281335238"
        );
        assert_eq!(
            value_id(Some(&serde_json::json!("1281335238"))),
            "1281335238"
        );
    }
}
