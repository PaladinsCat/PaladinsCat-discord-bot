//! Cache rendered PNG bytes in process with moka.
//!
//! Clones share TTL and byte-weighted storage; keys contribute to entry weights.
//! The cache performs no HTTP requests and has no durable backing store.
//! refs: doc: documents/05-operations/runbooks/discord-bot.md

use moka::future::Cache;
use std::time::Duration;

#[derive(Clone)]
/// Share an asynchronous moka cache of String keys and Vec<u8> image bytes across clones.
/// TTL and a byte-weighted capacity bound retention; each entry weight includes both key and value lengths.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct RenderCache {
    inner: Cache<String, Vec<u8>>,
}

impl RenderCache {
    /// Create an in-memory cache with a byte budget and TTL.
    ///
    /// I/O: `usize` (max bytes), `u64` (ttl secs) -> `RenderCache`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
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
    /// Return the cached value for a key, or None if absent/expired.
    ///
    /// I/O: `&str` (key) -> `Option<Vec<u8>>`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.get(key).await
    }

    #[allow(dead_code)] // Used by health server for cache stats reporting
    /// Store a value under a key (evicts to stay within the byte budget).
    ///
    /// I/O: `String` (key), `Vec<u8>` (value) -> ()
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub async fn set(&self, key: String, value: Vec<u8>) {
        self.inner.insert(key, value).await;
    }

    /// Approximate number of cached entries.
    ///
    /// I/O: () -> `u64`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Current weighted cache size in bytes, including key bytes.
    ///
    /// I/O: () -> `u64`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
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
