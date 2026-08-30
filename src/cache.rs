//! In-memory render cache — replaces render-cache.ts
//!
//! Uses moka for async-safe LRU caching with TTL eviction.

use moka::future::Cache;
use std::time::Duration;

#[derive(Clone)]
pub struct RenderCache {
    inner: Cache<String, Vec<u8>>,
}

impl RenderCache {
    pub fn new(max_bytes: usize, ttl_secs: u64) -> Self {
        Self {
            inner: Cache::builder()
                .time_to_live(Duration::from_secs(ttl_secs))
                .weigher(|key: &String, value: &Vec<u8>| {
                    key.len().saturating_add(value.len()).min(u32::MAX as usize) as u32
                })
                .max_capacity(max_bytes as u64)
                .build(),
        }
    }

    #[allow(dead_code)] // Used by health server for cache stats reporting
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.get(key).await
    }

    #[allow(dead_code)] // Used by health server for cache stats reporting
    pub async fn set(&self, key: String, value: Vec<u8>) {
        self.inner.insert(key, value).await;
    }

    /// Approximate number of cached entries.
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Current weighted cache size in bytes, including key bytes.
    pub fn approximate_bytes(&self) -> u64 {
        self.inner.weighted_size()
    }
}

#[cfg(test)]
mod tests {
    use super::RenderCache;

    #[tokio::test]
    async fn stores_png_bytes_without_base64_round_trip() {
        let cache = RenderCache::new(1024, 60);
        let png = b"\x89PNG\r\n\x1a\nraw-cache-fixture".to_vec();
        cache.set("match:1".into(), png.clone()).await;
        assert_eq!(cache.get("match:1").await, Some(png));
    }
}
