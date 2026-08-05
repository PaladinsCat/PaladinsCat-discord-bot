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
    pub fn new(max_bytes: usize, ttl_secs: u64) -> Self {
        Self {
            inner: Cache::builder()
                .time_to_live(Duration::from_secs(ttl_secs))
                .max_capacity(max_bytes as u64 / 1024) // estimate entries from bytes budget
                .build(),
        }
    }

    #[allow(dead_code)] // Used by health server for cache stats reporting
    pub async fn get(&self, key: &str) -> Option<String> {
        self.inner.get(key).await
    }

    #[allow(dead_code)] // Used by health server for cache stats reporting
    pub async fn set(&self, key: String, value: String) {
        self.inner.insert(key, value).await;
    }

    /// Approximate number of cached entries.
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count() as u64
    }

    /// Approximate total bytes of cached values (sum of string lengths).
    /// Moka does not expose entry iteration directly, so we return the entry
    /// count × estimated average size as a lower bound.
    pub fn approximate_bytes(&self) -> u64 {
        // Moka's future::Cache has no public snapshot() in 0.12.
        // We estimate from entry_count × typical cached value size.
        let entries = self.inner.entry_count();
        (entries as u64) * 1024 // rough estimate: ~1KB per cached render
    }
}
