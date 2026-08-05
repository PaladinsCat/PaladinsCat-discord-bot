//! PaladinsCat API client — replaces api.ts
//!
//! reqwest wrapper for PaladinsCat API endpoints.
//! Mirrors api.ts: player, match, champion, history lookups.

use moka::future::Cache;
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use reqwest::Client as HttpClient;
use std::time::Duration;

/// API client wrapper — stores base URL separately from reqwest client.
/// All path parameters are percent-encoded. Responses to 429 get exponential backoff.
#[derive(Clone)]
pub struct ApiClient {
    inner: HttpClient,
    base: String,
    #[allow(dead_code)] // Used by health server preview endpoints for cache-backed requests
    response_cache: Cache<String, serde_json::Value>,
}

/// Encode a path segment for use in URLs.
fn encode(s: &str) -> String {
    percent_encode(s.as_bytes(), NON_ALPHANUMERIC).to_string()
}

impl ApiClient {
    /// Create new client pointing to PaladinsCat API.
    pub fn new(base: &str) -> Self {
        Self {
            inner: HttpClient::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("build reqwest client"),
            base: base.to_string(),
            response_cache: Cache::builder()
                .time_to_live(Duration::from_secs(600))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Send a GET request with exponential backoff on 429 (rate limited).
    /// Retries up to 3 times with 500ms, 1s, 2s delays.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, reqwest::Error> {
        // Check cache first
        if let Some(cached) = self.response_cache.get(url).await {
            return Ok(cached);
        }

        let mut last_err: Option<reqwest::Error> = None;
        let delays = [500u64, 1000, 2000];

        for &delay_ms in &delays {
            match self.inner.get(url).send().await {
                Ok(resp) => {
                    if resp.status().as_u16() == 429 {
                        tracing::warn!(url, delay_ms, "Rate limited (429), backing off");
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    resp.error_for_status_ref()?;
                    let val: serde_json::Value = resp.json().await?;
                    // Cache successful response
                    self.response_cache.insert(url.to_string(), val.clone()).await;
                    return Ok(val);
                }
                Err(e) => {
                    if e.is_timeout() {
                        tracing::warn!(url, delay_ms, "Request timeout, retrying");
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.expect("loop runs at least once"))
    }

    /// Lookup player by name.
    pub async fn player(&self, name: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/player/{}", self.base, encode(name));
        self.get_json(&url).await
    }

    /// Get match details by ID.
    pub async fn match_info(&self, match_id: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/match/{}", self.base, encode(match_id));
        self.get_json(&url).await
    }

    /// Get all champion names.
    pub async fn champion_names(&self) -> Result<Vec<String>, reqwest::Error> {
        let url = format!("{}/champions", self.base);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => {
                Ok(arr
                    .iter()
                    .filter_map(|v| match v {
                        serde_json::Value::String(s) => Some(s.clone()),
                        serde_json::Value::Object(o) => o.get("name").and_then(|n| n.as_str().map(str::to_string)),
                        _ => None,
                    })
                    .collect())
            }
            _ => Ok(vec![]),
        }
    }

    /// Get all champions list.
    #[allow(dead_code)] // Kept for potential future use
    pub async fn champions(&self) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/champions", self.base);
        self.get_json(&url).await
    }

    /// Get player match history.
    pub async fn player_history(&self, player_id: &str, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/players/{}/history?limit={}", self.base, encode(player_id), limit);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Check player live match status.
    pub async fn live_match(&self, player: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/players/{}/current", self.base, encode(player));
        self.get_json(&url).await
    }

    /// Get player champion loadouts.
    pub async fn loadouts(&self, player_id: &str) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/players/{}/loadouts", self.base, encode(player_id));
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Get champion page data for stats.
    pub async fn champion_page_data(&self, slug: &str, scope: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/champions/{}/page-data?scope={}", self.base, encode(slug), encode(scope));
        self.get_json(&url).await
    }

    /// Get ranked map stats.
    pub async fn ranked_maps(&self, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/stats/maps?limit={}", self.base, limit);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Get ranked composition stats.
    pub async fn ranked_compositions(&self, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/stats/compositions?limit={}", self.base, limit);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Get ranked item stats.
    pub async fn ranked_items(&self, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/stats/items?limit={}", self.base, limit);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }
}
