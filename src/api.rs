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
/// Mirrors TS: PaladinsCatApi with service token auth.
#[derive(Clone)]
pub struct ApiClient {
    inner: HttpClient,
    inner_slow: HttpClient,
    base: String,
    /// Service token for /players/discord endpoint (mirrors TS: serviceToken)
    service_token: Option<String>,
    #[allow(dead_code)] // Used by health server preview endpoints for cache-backed requests
    response_cache: Cache<String, serde_json::Value>,
}

/// Encode a path segment for use in URLs.
fn encode(s: &str) -> String {
    percent_encode(s.as_bytes(), NON_ALPHANUMERIC).to_string()
}

/// Clamp a value to the given range.
fn clamp(val: usize, min: usize, max: usize) -> usize {
    val.max(min).min(max)
}

/// Map lobby scope string to (tierMin, tierMax) — mirrors ranked-lobby.ts.
fn lobby_scope_to_tiers(scope: &str) -> Option<(u32, u32)> {
    match scope {
        "bronze-gold" => Some((1, 15)),
        "platinum" => Some((16, 26)),
        "diamond" => Some((21, 26)),
        _ => None, // "global" or unknown → no tier filters
    }
}

impl ApiClient {
    /// Create new client pointing to PaladinsCat API.
    ///
    /// Mirrors TS: PaladinsCatApi constructor.
    /// `service_token` is used for authenticated endpoints (mirrors TS: X-PaladinsCat-Service-Token).
    pub fn new(base: &str, service_token: Option<&str>) -> Self {
        Self {
            inner: HttpClient::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("build reqwest client"),
            inner_slow: HttpClient::builder()
                .timeout(Duration::from_secs(125))
                .build()
                .expect("build slow reqwest client"),
            base: base.to_string(),
            service_token: service_token.map(str::to_string),
            response_cache: Cache::builder()
                .time_to_live(Duration::from_secs(600))
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Send a GET request with exponential backoff on 429 (rate limited).
    /// Retries up to 3 times with 500ms, 1s, 2s delays.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, reqwest::Error> {
        self.get_json_impl(&self.inner, url).await
    }

    /// Send a GET request with slow timeout (125s) — used for match endpoints.
    async fn get_json_slow(&self, url: &str) -> Result<serde_json::Value, reqwest::Error> {
        self.get_json_impl(&self.inner_slow, url).await
    }

    async fn get_json_impl(
        &self,
        client: &HttpClient,
        url: &str,
    ) -> Result<serde_json::Value, reqwest::Error> {
        // Check cache first
        if let Some(cached) = self.response_cache.get(url).await {
            return Ok(cached);
        }

        let delays = [500u64, 1000, 2000];

        for (attempt, &delay_ms) in delays.iter().enumerate() {
            let mut req = client.get(url);
            if let Some(token) = &self.service_token {
                req = req
                    .header(reqwest::header::USER_AGENT, "PaladinsCatDiscordBot/0.1")
                    .header("X-PaladinsCat-Service-Token", token.as_str());
            }
            match req.send().await {
                Ok(resp) => {
                    if resp.status().as_u16() == 429 && attempt + 1 < delays.len() {
                        tracing::warn!(url, delay_ms, "Rate limited (429), backing off");
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    resp.error_for_status_ref()?;
                    let val: serde_json::Value = resp.json().await?;
                    // Cache successful response
                    self.response_cache
                        .insert(url.to_string(), val.clone())
                        .await;
                    return Ok(val);
                }
                Err(e) => {
                    if e.is_timeout() {
                        // A second full request can triple a slow command's
                        // wall time while providing no extra information.
                        tracing::warn!(url, "Request timed out");
                    }
                    return Err(e);
                }
            }
        }
        unreachable!("request loop returns on its final attempt")
    }

    /// Fetch enriched player profile via authenticated /players/discord endpoint.
    ///
    /// Mirrors TS: discordPlayer(input) → GET /players/discord?player=<input>.
    /// This endpoint resolves the player AND returns enriched data including
    /// Hi-Rez profile info (gamertag, peak rank, headroom).
    /// Requires PALADINSCAT_SERVICE_TOKEN header.
    pub async fn discord_player(&self, name: &str) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/players/discord?player={}", self.base, encode(name));
        let val = self.get_json(&url).await?;
        Ok(val)
    }

    /// Return the default player saved for a Discord user.
    pub async fn saved_discord_player(
        &self,
        discord_user_id: &str,
    ) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!(
            "{}/players/discord/saved-player?discordUserId={}",
            self.base,
            encode(discord_user_id)
        );
        let value = self.get_json(&url).await?;
        Ok(value.get("player").cloned().unwrap_or(value))
    }

    /// Persist the authoritative player ID resolved by `/players/discord`.
    pub async fn save_discord_player(
        &self,
        discord_user_id: &str,
        player_id: &str,
    ) -> Result<serde_json::Value, reqwest::Error> {
        let url = format!("{}/players/discord/saved-player", self.base);
        let mut req = self.inner.put(url).json(&serde_json::json!({
            "discordUserId": discord_user_id,
            "playerId": player_id,
        }));
        if let Some(token) = &self.service_token {
            req = req.header("X-PaladinsCat-Service-Token", token);
        }
        let value = req
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(value.get("player").cloned().unwrap_or(value))
    }

    /// Resolve player name/ID to numeric ID and fetch profile.
    /// Used by history, loadout, current commands to get player ID.
    pub async fn player(&self, name: &str) -> Result<serde_json::Value, reqwest::Error> {
        let player_id = match self.resolve_player_id(name).await {
            Ok(id) => id,
            Err(e) => return Err(e),
        };
        if player_id.is_empty() {
            return Ok(serde_json::json!({"error": "player not found"}));
        }
        let url = format!(
            "{}/players/{}?include=ratings",
            self.base,
            encode(&player_id)
        );
        let val = self.get_json(&url).await?;
        match val.get("player") {
            Some(inner) if inner.is_object() => Ok(inner.clone()),
            _ => Ok(val),
        }
    }

    /// Resolve a player name or numeric ID to a canonical numeric player ID.
    ///
    /// Mirrors TS: resolvePlayer(input).
    /// - Numeric inputs pass through unchanged.
    /// - Names resolved via /players/search?name=...&limit=5.
    /// - Exact match (case-insensitive) preferred; fallback to first result.
    /// - Returns empty string when no player matches.
    async fn resolve_player_id(&self, input: &str) -> Result<String, reqwest::Error> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Ok(trimmed.to_string());
        }

        // TS: searchPlayers(input, 5) — limit=5
        let search_url = format!(
            "{}/players/search?name={}&limit=5",
            self.base,
            encode(trimmed)
        );
        let val = self.get_json(&search_url).await?;
        let rows = match val.as_array() {
            Some(arr) => arr.to_vec(),
            _ => vec![],
        };

        // TS: exact match first (case-insensitive), then first result
        let exact = rows.iter().find(|row| {
            row.get("name")
                .and_then(|v| v.as_str())
                .map(|n| n.eq_ignore_ascii_case(trimmed))
                .unwrap_or(false)
        });

        match exact {
            Some(row) => Ok(row
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()),
            None => {
                // Fallback to first result — TS: exact ?? rows[0]
                match rows.first() {
                    Some(row) => Ok(row
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()),
                    None => Ok(String::new()),
                }
            }
        }
    }

    /// Get match details by ID.
    ///
    /// Mirrors TS: match(id).
    /// - Primary: GET /matches/{id} with 125s timeout (slow client).
    /// - Parallel: GET /matches/fact/{id} (best-effort, 15s timeout).
    /// - Returns hydrated match object with facts merged into players.
    /// - Envelope: {"count": N, "matches": [{"match": {...}}]} → inner match object.
    pub async fn match_info(&self, match_id: &str) -> Result<serde_json::Value, reqwest::Error> {
        let encoded = encode(match_id);

        // Match the legacy local-only fast path: query the durable read model
        // first, and invoke the slow requested-match pipeline only on a miss.
        let batch_url = format!("{}/matches/batch?ids={}", self.base, encoded);
        let match_url = format!("{}/matches/{}", self.base, encoded);
        let fact_url = format!("{}/matches/fact/{}", self.base, encoded);

        let (batch_result, mut fact_result) = tokio::join!(self.get_json(&batch_url), async {
            self.get_json(&fact_url).await.ok()
        });

        let mut val = match batch_result {
            Ok(payload)
                if payload
                    .get("matches")
                    .and_then(|matches| matches.as_array())
                    .is_some_and(|matches| !matches.is_empty()) => payload,
            _ => {
                let payload = self.get_json_slow(&match_url).await?;
                if fact_result.is_none() {
                    fact_result = self.get_json(&fact_url).await.ok();
                }
                payload
            }
        };

        // Unwrap matches[0].match envelope — TS: payload.matches?.[0]
        let inner_match = val
            .get("matches")
            .and_then(|m| m.as_array())
            .and_then(|a| a.first())
            .map(|wrapper| {
                wrapper
                    .get("match")
                    .cloned()
                    .unwrap_or_else(|| wrapper.clone())
            })
            .unwrap_or_else(|| std::mem::take(&mut val));

        // Hydrate with facts — TS: merge facts.players into match
        if let Some(facts) = fact_result {
            if let Some(mut obj) = inner_match.as_object().cloned() {
                if let Some(fact_players) = facts.get("players").and_then(|v| v.as_array()) {
                    obj.insert("facts".to_string(), serde_json::json!(fact_players));
                }
                return Ok(serde_json::Value::Object(obj));
            }
        }

        Ok(inner_match)
    }

    /// Get all champion names.
    pub async fn champion_names(&self) -> Result<Vec<String>, reqwest::Error> {
        let url = format!("{}/champions", self.base);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr
                .iter()
                .filter_map(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Object(o) => {
                        o.get("name").and_then(|n| n.as_str().map(str::to_string))
                    }
                    _ => None,
                })
                .collect()),
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
    /// Mirrors TS: playerHistoryById(playerId, limit).
    /// Route: GET /players/{id}/matches?limit={}
    /// Uses slow client (30s timeout) — large history sets can be slow.
    pub async fn player_history(
        &self,
        player_id: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let url = format!(
            "{}/players/{}/matches?limit={}",
            self.base,
            encode(player_id),
            limit
        );
        let val: serde_json::Value = self.get_json_slow(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Check player live match status.
    ///
    /// Mirrors TS: liveMatch(input) → resolvePlayer → GET /live/players/{id}.
    /// Returns object with `in_game` boolean.
    pub async fn live_match(&self, player: &str) -> Result<serde_json::Value, reqwest::Error> {
        let player_id = self.resolve_player_id(player).await?;
        // TS: GET /live/players/{id}
        let url = format!("{}/live/players/{}", self.base, encode(&player_id));
        let val = self.get_json(&url).await?;
        let in_game = val.get("match").map(|m| !m.is_null()).unwrap_or(false);
        let mut out = val;
        if let Some(obj) = out.as_object_mut() {
            obj.insert("in_game".to_string(), serde_json::json!(in_game));
        }
        Ok(out)
    }

    /// Get player champion loadouts.
    ///
    /// Mirrors TS: playerLoadoutsById(playerId).
    /// Route: GET /players/{id}/loadouts
    /// Backend returns {"loadouts": [...], "freshness": {...}}; unwraps loadouts array.
    pub async fn loadouts(
        &self,
        player_id: &str,
    ) -> Result<Vec<serde_json::Value>, reqwest::Error> {
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
    ///
    /// Mirrors TS: championPageData(idOrSlug, scope).
    /// - scope maps to tierMin/tierMax via lobby_scope_to_tiers().
    /// - "global" or unknown scope → no tier filter (no query params).
    /// Route: GET /champions/{slug}/page-data?tierMin={}&tierMax={}
    pub async fn champion_page_data(
        &self,
        slug: &str,
        scope: &str,
    ) -> Result<serde_json::Value, reqwest::Error> {
        let q = if let Some((tier_min, tier_max)) = lobby_scope_to_tiers(scope) {
            format!("?tierMin={}&tierMax={}", tier_min, tier_max)
        } else {
            String::new()
        };
        let url = format!("{}/champions/{}/page-data{}", self.base, encode(slug), q);
        self.get_json(&url).await
    }

    /// Get ranked map stats.
    ///
    /// Mirrors TS: rankedMaps(limit=100).
    /// Route: GET /stats/maps?queueId=486&limit={} (clamped 1-100)
    pub async fn ranked_maps(
        &self,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let clamped = clamp(limit, 1, 100);
        // TS: queueId=486 is the ranked queue
        let url = format!("{}/stats/maps?queueId=486&limit={}", self.base, clamped);
        let val: serde_json::Value = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Get ranked composition stats.
    ///
    /// Mirrors TS: rankedCompositions(limit=5).
    /// Route: GET /matches/compositions?sortBy=count&order=desc&limit={} (clamped 1-25)
    /// Backend returns {"total": N, "data": [...]} — unwraps data array.
    pub async fn ranked_compositions(
        &self,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let clamped = clamp(limit, 1, 25);
        let url = format!(
            "{}/matches/compositions?sortBy=count&order=desc&limit={}",
            self.base, clamped
        );
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
    ///
    /// Mirrors TS: rankedItems(scope, limit=20).
    /// Route: GET /stats/items?mode=ranked&limit={} (clamped 1-50).
    /// "global" scope → no tier filter appended.
    pub async fn ranked_items(
        &self,
        scope: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, reqwest::Error> {
        let clamped = clamp(limit, 1, 50);
        let tiers = lobby_scope_to_tiers(scope)
            .map(|(min, max)| format!("&tierMin={min}&tierMax={max}"))
            .unwrap_or_default();
        let url = format!(
            "{}/stats/items?mode=ranked&limit={}{}",
            self.base, clamped, tiers
        );
        let val = self.get_json(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }
}
