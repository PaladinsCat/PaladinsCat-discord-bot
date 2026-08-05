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

    /// Resolve a player name or numeric ID to the backend player profile.
    ///
    /// Mirrors the canonical backend-rust contract: `name` is resolved to a
    /// numeric player ID via `/players/search`, then the profile is fetched
    /// from `/players/{id}` (the profile object is unwrapped from the
    /// `{"player": {...}}` envelope so commands can read `id`/`name` directly).
    pub async fn player(&self, name: &str) -> Result<serde_json::Value, reqwest::Error> {
        // Resolve name -> ID via canonical search endpoint (or numeric passthrough).
        let player_id = match self.resolve_player_id(name).await {
            Ok(id) => id,
            Err(e) => return Err(e),
        };
        if player_id.is_empty() {
            return Ok(serde_json::json!({"error": "player not found"}));
        }

        let url = format!("{}/players/{}", self.base, encode(&player_id));
        let val = self.get_json(&url).await?;
        // Unwrap the `{"player": {...}}` envelope.
        match val.get("player") {
            Some(inner) if inner.is_object() => Ok(inner.clone()),
            _ => Ok(val),
        }
    }

    /// Resolve a player name or numeric ID to a canonical numeric player ID.
    ///
    /// Numeric inputs pass through unchanged. Names are resolved via the
    /// `/players/search?name=` endpoint (first hit). Returns an empty string when
    /// no player matches, mirroring the "not found" contract.
    async fn resolve_player_id(&self, input: &str) -> Result<String, reqwest::Error> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Ok(trimmed.to_string());
        }
        let search_url = format!("{}/players/search?name={}&limit=1", self.base, encode(trimmed));
        let val = self.get_json(&search_url).await?;
        match val.as_array().and_then(|arr| arr.first()) {
            Some(row) => Ok(row
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()),
            None => Ok(String::new()),
        }
    }

    /// Get match details by ID.
    ///
    /// Canonical backend-rust contract is `/matches/{id}`, which returns a
    /// `{"count": N, "matches": [{"match": {...}}]}` envelope. This unwraps to
    /// the inner match object so commands can read `map`/`duration`/etc.
    pub async fn match_info(&self, match_id: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/matches/{}", self.base, encode(match_id));
        let val = self.get_json(&url).await?;
        match val.get("matches").and_then(|m| m.as_array()).and_then(|a| a.first()) {
            Some(wrapper) => {
                if let Some(m) = wrapper.get("match") {
                    return Ok(m.clone());
                }
                Ok(wrapper.clone())
            }
            _ => Ok(val),
        }
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
    ///
    /// Canonical backend-rust route is `/players/{id}/matches` (returns a bare array).
    pub async fn player_history(&self, player_id: &str, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/players/{}/matches?limit={}", self.base, encode(player_id), limit);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Check player live match status.
    ///
    /// Canonical backend-rust route is `/matches/live/{player_id}`, which returns
    /// `{"match": {...}, "players": [...]}` when in-game (or `{"message": ...}` /
    /// `{"match": null}` when not). This normalises to an object with an `in_game`
    /// boolean so the `current` command can render it.
    pub async fn live_match(&self, player: &str) -> Result<serde_json::Value, reqwest::Error> {
        let player_id = self.resolve_player_id(player).await?;
        let url = format!("{}/matches/live/{}", self.base, encode(&player_id));
        let val = self.get_json(&url).await?;
        let in_game = val
            .get("match")
            .map(|m| !m.is_null())
            .unwrap_or(false);
        let mut out = val;
        if let Some(obj) = out.as_object_mut() {
            obj.insert("in_game".to_string(), serde_json::json!(in_game));
        }
        Ok(out)
    }

    /// Get player champion loadouts.
    ///
    /// Backend returns `{"loadouts": [...], "freshness": {...}}`; this unwraps the
    /// `loadouts` array so commands can iterate it directly.
    pub async fn loadouts(&self, player_id: &str) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/players/{}/loadouts", self.base, encode(player_id));
        let val: serde_json::Value = self.get_json(&url).await?;
        match val.get("loadouts").and_then(|v| v.as_array()) {
            Some(arr) => Ok(arr.to_vec()),
            None => match &val {
                serde_json::Value::Array(arr) => Ok(arr.to_vec()),
                _ => Ok(vec![val]),
            },
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
    ///
    /// Canonical backend-rust route is `/matches/compositions`, which returns
    /// `{"total": N, "data": [...]}`. This unwraps the `data` array.
    pub async fn ranked_compositions(&self, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/matches/compositions?limit={}", self.base, limit);
        let val: serde_json::Value = self.get_json(&url).await?;
        match val.get("data").and_then(|v| v.as_array()) {
            Some(arr) => Ok(arr.to_vec()),
            None => match &val {
                serde_json::Value::Array(arr) => Ok(arr.to_vec()),
                _ => Ok(vec![val]),
            },
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
