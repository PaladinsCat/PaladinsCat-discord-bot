//! Coordinate queued rendering, PNG caching, and browser recovery.
//!
//! Cache keys include template versions; the shared browser uses one queue permit.
//! Failed attempts can recycle Chromium; queued callers share in-flight results.
//! refs: doc: documents/05-operations/runbooks/discord-bot.md

use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde_json::Value;

use crate::cache::RenderCache;
use crate::image::match_renderer::MatchRenderer;
use crate::image::render_queue::{BoundedWorkQueue, QueueSnapshot};

#[derive(Debug, Clone)]
/// Configure queue limit, execution timeout in milliseconds, and cache byte/TTL budgets.
/// ImageService uses one queue permit for its shared CDP page.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct ImageServiceConfig {
    pub queue_limit: usize,
    pub timeout_ms: u64,
    pub cache_bytes: usize,
    pub cache_ttl_secs: u64,
}

impl Default for ImageServiceConfig {
    fn default() -> Self {
        Self {
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
/// Report queue state (including deduplication), cached entry/byte estimates, render retries, browser
/// recoveries, and per-attempt timeout milliseconds.
/// The service and queue counters are read from separate locks and are not one atomic snapshot.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct ServiceSnapshot {
    pub queue: QueueSnapshot,
    pub cache_entries: u64,
    pub cache_bytes: u64,
    pub render_retries: usize,
    pub browser_recoveries: usize,
    pub render_attempt_timeout_ms: u64,
}

#[derive(Debug, Default)]
struct ServiceStats {
    render_retries: usize,
    browser_recoveries: usize,
}

/// Coordinate a shared renderer, PNG cache, bounded queue, and recovery counters.
/// Queued render entry points cache successful PNGs and share in-flight work by key.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct ImageService {
    renderer: Arc<MatchRenderer>,
    cache: RenderCache,
    queue: BoundedWorkQueue<Vec<u8>>,
    render_attempt_timeout_ms: u64,
    stats: StdMutex<ServiceStats>,
}

impl ImageService {
    /// Create an image service from a renderer and config.
    ///
    /// Force one queue permit for the shared CDP page and set attempt timeout to clamp(40% of queue
    /// timeout, 1, 6000) milliseconds. Allocate caches/counters without starting Chromium.
    ///
    /// I/O: `Arc<MatchRenderer>`, `ImageServiceConfig` -> `ImageService`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub fn new(renderer: Arc<MatchRenderer>, config: ImageServiceConfig) -> Self {
        let render_attempt_timeout_ms = ((config.timeout_ms as f64 * 0.4) as u64).clamp(1, 6000);
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
            render_attempt_timeout_ms,
            stats: StdMutex::new(ServiceStats::default()),
        }
    }

    /// Render a match scoreboard record to PNG bytes (queued + cached).
    ///
    /// Use match.match_id plus template version for caching; on a miss admit keyed work and render
    /// with recovery, then cache success. Queue admission, execution, and exhausted recovery errors
    /// propagate.
    ///
    /// Rendering retries once after an error; each failed attempt recycles Chromium.
    ///
    /// I/O: `&Value` (record) -> `Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
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
            return Ok(cached);
        }

        let result = self
            .queue
            .add(match_id.clone(), || async {
                self.render_with_recovery(|| async { self.renderer.render(record).await })
                    .await
            })
            .await;
        let result = self.finish_queued_render(result).await?;

        self.cache.set(cache_key, result.clone()).await;
        Ok(result)
    }

    /// Return a completed match render without invoking the backend or browser.
    /// This preserves the cache-first path for repeated match commands.
    ///
    /// I/O: `&str` (match id) -> `Option<Vec<u8>>`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub async fn cached_match(&self, match_id: &str) -> Option<Vec<u8>> {
        let cache_key = format!(
            "match:{}:summary:v{}",
            match_id,
            self.renderer.template_version()
        );
        self.cache
            .get(&cache_key)
            .await
            .filter(|png| !png.is_empty())
    }

    /// Render a loadout card record to PNG bytes (queued + cached).
    ///
    /// Key by player/loadout ID, updated_at or fetched_at, and template version; queue cache misses
    /// and render with recovery. Cache successful PNGs; queue and exhausted recovery errors
    /// propagate.
    ///
    /// Rendering retries once after an error; each failed attempt recycles Chromium.
    ///
    /// I/O: `&Value` (record) -> `Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
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
            return Ok(cached);
        }

        let result = self
            .queue
            .add(cache_key.clone(), || async {
                self.render_with_recovery(|| async { self.renderer.render_loadout(record).await })
                    .await
            })
            .await;
        let result = self.finish_queued_render(result).await?;

        self.cache.set(cache_key, result.clone()).await;
        Ok(result)
    }

    /// Warm up the underlying renderer.
    ///
    /// Delegate browser startup to the renderer and propagate startup/discovery/CDP errors; no
    /// render is cached.
    ///
    /// I/O: () -> `Result<(), Box<dyn std::error::Error + Send + Sync>>`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub async fn warm(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.renderer.warm().await
    }

    /// Discard a renderer left mid-request by a caller-level timeout.
    /// The command timeout cancels its future before `render_with_recovery`
    /// can observe an error, so it must explicitly reset Chromium.
    ///
    /// Delegate browser reset to the renderer; service cache and counters remain allocated.
    ///
    /// I/O: `&ImageService` (self) -> `()`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub async fn recycle(&self) {
        self.renderer.recycle().await;
    }

    /// Get a snapshot of the service state.
    ///
    /// Read service counters under a mutex and obtain separate queue/cache estimates; no network or
    /// rendering is triggered.
    ///
    /// I/O: () -> `ServiceSnapshot`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub fn snapshot(&self) -> ServiceSnapshot {
        let stats = self.stats.lock().unwrap();
        ServiceSnapshot {
            queue: self.queue.snapshot(),
            cache_entries: self.cache.entry_count(),
            cache_bytes: self.cache.approximate_bytes(),
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
        assert_eq!(config.queue_limit, 10);
        assert_eq!(config.timeout_ms, 20_000);
        assert_eq!(config.cache_bytes, 32 * 1024 * 1024);
        assert_eq!(config.cache_ttl_secs, 600);
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
