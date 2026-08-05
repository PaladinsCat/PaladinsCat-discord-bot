//! PaladinsCat API client — replaces api.ts
//!
//! reqwest wrapper for PaladinsCat API endpoints.
//! Mirrors api.ts: player, match, champion, history lookups.

use reqwest::Client as HttpClient;

/// API client wrapper — stores base URL separately from reqwest client.
#[derive(Clone)]
pub struct ApiClient {
    inner: HttpClient,
    base: String,
}

impl ApiClient {
    /// Create new client pointing to PaladinsCat API.
    pub fn new(base: &str) -> Self {
        Self {
            inner: HttpClient::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("build reqwest client"),
            base: base.to_string(),
        }
    }

    /// Lookup player by name.
    pub async fn player(&self, name: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/player/{}", self.base, name);
        self.inner.get(&url).send().await?.json().await
    }

    /// Get match details by ID.
    pub async fn match_info(&self, match_id: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/match/{}", self.base, match_id);
        self.inner.get(&url).send().await?.json().await
    }

    /// Get all champion names.
    pub async fn champion_names(&self) -> Result<Vec<String>, reqwest::Error> {
        let url = format!("{}/champions", self.base);
        let val: serde_json::Value = self.inner.get(&url).send().await?.json().await?;
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
    pub async fn champions(&self) -> Result<serde_json::Value, reqwest::Error> {
        self.inner.get(&format!("{}/champions", self.base)).send().await?.json().await
    }

    /// Get player match history.
    pub async fn player_history(&self, player_id: &str, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/players/{}/history?limit={}", self.base, player_id, limit);
        let val: serde_json::Value = self.inner.get(&url).send().await?.json().await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Check player live match status.
    pub async fn live_match(&self, player: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/players/{}/current", self.base, player);
        self.inner.get(&url).send().await?.json().await
    }

    /// Get player champion loadouts.
    pub async fn loadouts(&self, player_id: &str) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/players/{}/loadouts", self.base, player_id);
        let val: serde_json::Value = self.inner.get(&url).send().await?.json().await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Get champion page data for stats.
    pub async fn champion_page_data(&self, slug: &str, scope: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/champions/{}/page-data?scope={}", self.base, slug, scope);
        self.inner.get(&url).send().await?.json().await
    }

    /// Get ranked map stats.
    pub async fn ranked_maps(&self, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/stats/maps?limit={}", self.base, limit);
        let val: serde_json::Value = self.inner.get(&url).send().await?.json().await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Get ranked composition stats.
    pub async fn ranked_compositions(&self, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/stats/compositions?limit={}", self.base, limit);
        let val: serde_json::Value = self.inner.get(&url).send().await?.json().await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Get ranked item stats.
    pub async fn ranked_items(&self, limit: usize) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!("{}/stats/items?limit={}", self.base, limit);
        let val: serde_json::Value = self.inner.get(&url).send().await?.json().await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }
}