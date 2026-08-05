//! In-memory render cache — replaces render-cache.ts
//!
//! Uses moka for async-safe LRU caching with TTL eviction.

use moka::future::Cache;
use std::time::Duration;

#[derive(Clone)]
pub struct RenderCache {
    inner: Cache<String, String>,
}

impl RenderCache {
    pub fn new(_max_bytes: usize, ttl_secs: u64) -> Self {
        Self {
            inner: Cache::builder()
                .time_to_live(Duration::from_secs(ttl_secs))
                .max_capacity(10_000)
                .build(),
        }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        self.inner.get(key).await
    }

    pub async fn set(&self, key: String, value: String) {
        self.inner.insert(key, value).await;
    }
}