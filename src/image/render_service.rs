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
            concurrency: 2,
            queue_limit: 10,
            timeout_ms: 8000,
            cache_bytes: 50 * 1024 * 1024,
            cache_ttl_secs: 300,
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
        Self { deduplicated: 0, render_retries: 0, browser_recoveries: 0 }
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
        let render_attempt_timeout_ms = std::cmp::max(1, std::cmp::min(6000, (config.timeout_ms as f64 * 0.4) as u64));
        Self {
            renderer,
            cache: RenderCache::new(config.cache_bytes, config.cache_ttl_secs),
            queue: BoundedWorkQueue::new(config.concurrency, config.queue_limit, config.timeout_ms, "Render"),
            in_flight_matches: StdMutex::new(HashMap::new()),
            render_attempt_timeout_ms,
            stats: StdMutex::new(ServiceStats::default()),
        }
    }

    pub async fn render_match(&self, record: &Value) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let match_id = record["match"]["match_id"].as_str().unwrap_or("unknown");
        let cache_key = format!("match:{}:summary:v{}", match_id, self.renderer.template_version());

        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(decode_b64(&cached));
        }

        let result = self.render_with_dedup(match_id, || async {
            self.render_with_recovery(|| async {
                self.renderer.render(record).await
            }).await
        }).await?;

        self.cache.set(cache_key, encode_b64(&result)).await;
        Ok(result)
    }

    pub async fn render_web_match(
        &self,
        match_id: &str,
        url: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let cache_key = format!("match:{}:summary:v{}", match_id, self.renderer.template_version());
        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(decode_b64(&cached));
        }
        let result = self.render_with_dedup(match_id, || async {
            self.render_with_recovery(|| async { self.renderer.render_web_match(url).await }).await
        }).await?;
        self.cache.set(cache_key, encode_b64(&result)).await;
        Ok(result)
    }

    pub async fn render_loadout(&self, record: &Value) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let player_id = record["player"]["id"].as_str().unwrap_or("unknown");
        let loadout_id = record["loadout"]["id"].as_str().unwrap_or("unknown");
        let updated_at = record["loadout"]["updated_at"]
            .as_str()
            .unwrap_or(record["loadout"]["fetched_at"].as_str().unwrap_or("unknown"));
        let cache_key = format!("loadout:{}:{}:{}:v{}", player_id, loadout_id, updated_at, self.renderer.loadout_template_version());

        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(decode_b64(&cached));
        }

        let result = self.render_with_recovery(|| async {
            self.renderer.render_loadout(record).await
        }).await?;

        self.cache.set(cache_key, encode_b64(&result)).await;
        Ok(result)
    }

    pub async fn match_by_id<F>(&self, match_id: String, load: F)
        -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnOnce() -> Box<dyn std::future::Future<Output = Result<Value, Box<dyn std::error::Error + Send + Sync>>> + Send>,
    {
        let cache_key = format!("match:{}:summary:v{}", match_id, self.renderer.template_version());
        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(decode_b64(&cached));
        }

        let holder = self.get_or_create_in_flight(&match_id);
        {
            let guard = holder.lock().await;
            if let Some(result) = guard.as_ref() { return Ok(result.clone()); }
        }

        // SAFETY: `load()` returns a `impl Future` which is always `Unpin`
        let record = unsafe {
            let f = std::pin::Pin::new_unchecked(load());
            f.await?
        };
        let result = self.render_with_recovery(|| async {
            self.renderer.render(&record).await
        }).await?;

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

    async fn render_with_recovery<F, Fut>(&self, mut render: F)
        -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>>,
    {
        for attempt in 0..2 {
            match render().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if attempt == 0 {
                        self.renderer.recycle().await;
                        let mut s = self.stats.lock().unwrap();
                        s.browser_recoveries += 1;
                        s.render_retries += 1;
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Err("Render recovery exhausted".into())
    }

    async fn render_with_dedup<F, Fut>(&self, match_id: &str, render: F)
        -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>>,
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
    let t = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut r = String::with_capacity(bytes.len() / 3 * 4 + 4);
    let mut i = 0;
    while i + 2 < bytes.len() {
        let (a, b, c) = (bytes[i] as u32, bytes[i+1] as u32, bytes[i+2] as u32);
        r.push(t[(((a>>2)&63) as usize)] as char);
        r.push(t[(((a<<4)|(b>>4)) as u8 as usize)] as char);
        r.push(t[(((b<<2)|(c>>6)) as u8 as usize)] as char);
        r.push(t[((c&63) as usize)] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let a = bytes[i] as u32;
        r.push(t[(((a>>2)&63) as usize)] as char);
        r.push(t[((a<<4) as u8 as usize)] as char);
        r.push('='); r.push('=');
    } else if rem == 2 {
        let (a, b) = (bytes[i] as u32, bytes[i+1] as u32);
        r.push(t[(((a>>2)&63) as usize)] as char);
        r.push(t[(((a<<4)|(b>>4)) as u8 as usize)] as char);
        r.push(t[((b<<2) as u8 as usize)] as char);
        r.push('=');
    }
    r
}

fn decode_b64(s: &str) -> Vec<u8> {
    let lookup: [u8; 256] = {
        let mut t = [255u8; 256];
        for i in b'A'..=b'Z' { t[i as usize] = (i - b'A') as u8; }
        for i in b'a'..=b'z' { t[i as usize] = (i - b'a' + 26) as u8; }
        for i in b'0'..=b'9' { t[i as usize] = (i - b'0' + 52) as u8; }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut i = 0;
    while i + 3 < bytes.len() {
        let a = lookup[bytes[i] as usize];
        let b = lookup[bytes[i+1] as usize];
        let c = lookup[bytes[i+2] as usize];
        let d = lookup[bytes[i+3] as usize];
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
        out.push((c << 6) | d);
        i += 4;
    }
    out
}
