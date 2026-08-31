//! PaladinsCat API client — replaces api.ts
//!
//! reqwest wrapper for PaladinsCat API endpoints.
//! Mirrors api.ts: player, match, champion, history lookups.

use crate::service_auth::ServiceTokenProvider;
use moka::future::Cache;
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use reqwest::Client as HttpClient;
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Debug)]
/// Define ApiError.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub struct ApiError {
    pub status: Option<u16>,
    pub message: String,
    pub code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_error_preserves_safe_message_and_code() {
        let error = response_error(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"message":"Player not found","code":"PLAYER_NOT_FOUND"}"#,
        );
        assert_eq!(error.status, Some(404));
        assert_eq!(error.message, "Player not found");
        assert_eq!(error.code.as_deref(), Some("PLAYER_NOT_FOUND"));
    }

    #[test]
    fn nested_backend_error_preserves_message_and_code() {
        let error = response_error(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"Invalid player","code":"BAD_PLAYER"}}"#,
        );
        assert_eq!(error.message, "Invalid player");
        assert_eq!(error.code.as_deref(), Some("BAD_PLAYER"));
    }

    #[test]
    fn malformed_backend_error_uses_safe_fallback() {
        assert_eq!(
            response_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "nope").message,
            "The PaladinsCat service request failed."
        );
    }

    #[test]
    fn player_ids_accept_backend_strings_and_numbers() {
        assert_eq!(
            json_id(Some(&serde_json::json!("123"))).as_deref(),
            Some("123")
        );
        assert_eq!(
            json_id(Some(&serde_json::json!(123))).as_deref(),
            Some("123")
        );
    }

    #[test]
    fn match_players_receive_the_shared_public_tag_counts() {
        let mut player = serde_json::Map::new();
        merge_public_moderation(
            &mut player,
            &serde_json::json!({
                "sus_count": 5,
                "automatic_afk_count": 4,
                "wall_shooter_count": 5,
                "hypercarry_count": 6
            }),
        );
        assert_eq!(player["sus_count"], 5);
        assert_eq!(player["automatic_afk_count"], 4);
        assert_eq!(player["wall_shooter_count"], 5);
        assert_eq!(player["hypercarry_count"], 6);
    }

    #[test]
    fn api_base_is_consolidated_on_v1() {
        assert_eq!(
            ApiClient::new("http://backend:3005", None).base,
            "http://backend:3005/v1"
        );
        assert_eq!(
            ApiClient::new("http://backend:3005/api", None).base,
            "http://backend:3005/api/v1"
        );
        assert_eq!(
            ApiClient::new("http://backend:3005/api/v1/", None).base,
            "http://backend:3005/api/v1"
        );
    }

    #[test]
    fn latest_player_match_forces_one_row_history_read_through() {
        assert_eq!(
            latest_player_match_url("http://backend:3005/api/v1", "716515038"),
            "http://backend:3005/api/v1/players/716515038/matches?limit=1&offset=0&refresh=true"
        );
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ApiError {}
impl From<reqwest::Error> for ApiError {
    fn from(error: reqwest::Error) -> Self {
        Self {
            status: error.status().map(|s| s.as_u16()),
            message: "The PaladinsCat service request failed.".into(),
            code: None,
        }
    }
}
fn response_error(status: reqwest::StatusCode, body: &str) -> ApiError {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let details = value
        .as_ref()
        .and_then(|v| v.get("error"))
        .filter(|v| v.is_object())
        .or(value.as_ref());
    let message = details
        .and_then(|v| v.get("message").or_else(|| v.get("error")))
        .and_then(|v| v.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or("The PaladinsCat service request failed.")
        .to_owned();
    let code = details.and_then(|v| v.get("code").and_then(|v| v.as_str()).map(str::to_owned));
    ApiError {
        status: Some(status.as_u16()),
        message,
        code,
    }
}

fn player_not_found(input: &str) -> ApiError {
    ApiError {
        status: Some(404),
        message: format!("Player “{}” was not found", input),
        code: None,
    }
}

/// API client wrapper — stores base URL separately from reqwest client.
/// All path parameters are percent-encoded. Responses to 429 get exponential backoff.
/// Mirrors TS: PaladinsCatApi with service token auth.
#[derive(Clone)]
/// Define ApiClient.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub struct ApiClient {
    inner: HttpClient,
    inner_slow: HttpClient,
    base: String,
    /// Short-lived Keycloak service identity; private key remains external to the repo.
    service_auth: Option<Arc<ServiceTokenProvider>>,
    /// Short-lived cache only for the static champion roster used by autocomplete.
    response_cache: Cache<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
/// Define LoadoutsResponse.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub struct LoadoutsResponse {
    pub loadouts: Vec<serde_json::Value>,
    pub refreshed: bool,
    pub refresh_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
/// Define HistoryFilters.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
pub struct HistoryFilters {
    pub queue_id: Option<String>,
    pub champion_id: Option<String>,
    pub win_status: Option<String>,
    pub offset: usize,
}

/// Encode a path segment for use in URLs.
fn encode(s: &str) -> String {
    percent_encode(s.as_bytes(), NON_ALPHANUMERIC).to_string()
}

fn json_id(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(|value| match value {
        serde_json::Value::String(id) => Some(id.clone()),
        serde_json::Value::Number(id) => Some(id.to_string()),
        _ => None,
    })
}

fn latest_player_match_url(base: &str, player_id: &str) -> String {
    format!(
        "{}/players/{}/matches?limit=1&offset=0&refresh=true",
        base,
        encode(player_id)
    )
}

const PUBLIC_MODERATION_FIELDS: [&str; 20] = [
    "cheater",
    "sus_count",
    "dropper",
    "dropper_vote_count",
    "afk_wintrade",
    "afk_wintrade_vote_count",
    "boosted",
    "boosted_match_count",
    "alt_account",
    "alt_account_vote_count",
    "automatic_afk_count",
    "wall_shooter_count",
    "master_feeding_count",
    "tank_diff_count",
    "support_diff_count",
    "dps_diff_count",
    "flank_diff_count",
    "noob_count",
    "hypercarry_count",
    "verified",
];

fn merge_public_moderation(
    player: &mut serde_json::Map<String, serde_json::Value>,
    moderation: &serde_json::Value,
) {
    for field in PUBLIC_MODERATION_FIELDS {
        if let Some(value) = moderation.get(field).filter(|value| !value.is_null()) {
            player.insert(field.to_string(), value.clone());
        }
    }
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
    /// `service_auth` supplies short-lived client-credentials bearer tokens.
    pub fn new(base: &str, service_auth: Option<ServiceTokenProvider>) -> Self {
        Self {
            inner: HttpClient::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("build reqwest client"),
            inner_slow: HttpClient::builder()
                .timeout(Duration::from_secs(125))
                .build()
                .expect("build slow reqwest client"),
            base: {
                let base = base.trim_end_matches('/');
                if base.ends_with("/v1") {
                    base.to_owned()
                } else {
                    format!("{base}/v1")
                }
            },
            service_auth: service_auth.map(Arc::new),
            response_cache: Cache::builder()
                .time_to_live(Duration::from_secs(600))
                .max_capacity(10_000)
                .build(),
        }
    }

    async fn bearer(&self) -> Result<Option<String>, ApiError> {
        if let Some(provider) = self.service_auth.as_ref() {
            return provider.token().await.map(Some).map_err(|_error| ApiError {
                status: None,
                message: "The PaladinsCat service authentication failed.".into(),
                code: None,
            });
        }
        Ok(None)
    }

    /// Send a GET request with exponential backoff on 429 (rate limited).
    /// Retries up to 3 times with 500ms, 1s, 2s delays.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ApiError> {
        self.get_json_impl(&self.inner, url).await
    }

    /// Send a GET request with slow timeout (125s) — used for match endpoints.
    async fn get_json_slow(&self, url: &str) -> Result<serde_json::Value, ApiError> {
        self.get_json_impl(&self.inner_slow, url).await
    }

    async fn post_empty(&self, url: &str) -> Result<serde_json::Value, ApiError> {
        let mut request = self.inner.post(url);
        if let Some(token) = self.bearer().await? {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(response_error(status, &response.text().await?));
        }
        Ok(response.json().await?)
    }

    async fn get_json_impl(
        &self,
        client: &HttpClient,
        url: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let delays = [500u64, 1000, 2000];

        for (attempt, &delay_ms) in delays.iter().enumerate() {
            let mut req = client.get(url);
            req = req.header(reqwest::header::USER_AGENT, "PaladinsCatDiscordBot/0.1");
            if let Some(token) = self.bearer().await? {
                req = req.bearer_auth(token);
            }
            match req.send().await {
                Ok(resp) => {
                    if resp.status().as_u16() == 429 && attempt + 1 < delays.len() {
                        tracing::warn!(url, delay_ms, "Rate limited (429), backing off");
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let body = resp.text().await?;
                        return Err(response_error(status, &body));
                    }
                    let val: serde_json::Value = resp.json().await?;
                    return Ok(val);
                }
                Err(e) => {
                    if e.is_timeout() {
                        // A second full request can triple a slow command's
                        // wall time while providing no extra information.
                        tracing::warn!(url, "Request timed out");
                    }
                    return Err(e.into());
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
    /// Requires the configured Keycloak service identity.
    pub async fn discord_player(&self, name: &str) -> Result<serde_json::Value, ApiError> {
        let url = format!("{}/players/discord?player={}", self.base, encode(name));
        let val = self.get_json(&url).await?;
        Ok(val)
    }

    /// Return the default player saved for a Discord user.
    pub async fn saved_discord_player(
        &self,
        discord_user_id: &str,
        slot: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let url = format!(
            "{}/players/discord/saved-player?discordUserId={}&slot={}",
            self.base,
            encode(discord_user_id),
            encode(slot)
        );
        let value = self.get_json(&url).await?;
        Ok(value.get("player").cloned().unwrap_or(value))
    }

    /// Persist the authoritative player ID resolved by `/players/discord`.
    pub async fn save_discord_player(
        &self,
        discord_user_id: &str,
        player_id: &str,
        slot: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let url = format!("{}/players/discord/saved-player", self.base);
        let mut req = self.inner.put(url).json(&serde_json::json!({
            "discordUserId": discord_user_id,
            "playerId": player_id,
            "slot": slot,
        }));
        if let Some(token) = self.bearer().await? {
            req = req.bearer_auth(token);
        }
        let response = req.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(response_error(status, &response.text().await?));
        }
        let value = response.json::<serde_json::Value>().await?;
        Ok(value.get("player").cloned().unwrap_or(value))
    }

    /// Implement forget_discord_player.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn forget_discord_player(
        &self,
        discord_user_id: &str,
        slot: &str,
    ) -> Result<usize, ApiError> {
        let url = format!(
            "{}/players/discord/saved-player?discordUserId={}&slot={}",
            self.base,
            encode(discord_user_id),
            encode(slot)
        );
        let mut request = self.inner.delete(url);
        if let Some(token) = self.bearer().await? {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(response_error(status, &response.text().await?));
        }
        Ok(response
            .json::<serde_json::Value>()
            .await?
            .get("deleted")
            .and_then(|value| value.as_u64())
            .unwrap_or_default() as usize)
    }

    /// Resolve player name/ID to numeric ID and fetch profile.
    /// Used by history, loadout, current commands to get player ID.
    pub async fn player(&self, name: &str) -> Result<serde_json::Value, ApiError> {
        let resolved = self.resolve_player(name).await?;
        let player_id = json_id(resolved.get("id")).unwrap_or_default();
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
    pub async fn resolve_player(&self, input: &str) -> Result<serde_json::Value, ApiError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(player_not_found(trimmed));
        }
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Ok(serde_json::json!({"id": trimmed, "name": trimmed}));
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

        let mut result = exact
            .or_else(|| rows.first())
            .cloned()
            .ok_or_else(|| player_not_found(trimmed))?;
        let Some(id) = json_id(result.get("id")) else {
            return Err(player_not_found(trimmed));
        };
        let Some(object) = result.as_object_mut() else {
            return Err(player_not_found(trimmed));
        };
        object.insert("id".to_string(), serde_json::Value::String(id));
        Ok(result)
    }

    async fn resolve_player_id(&self, input: &str) -> Result<String, ApiError> {
        let resolved = self.resolve_player(input).await?;
        json_id(resolved.get("id")).ok_or_else(|| player_not_found(input.trim()))
    }

    /// Get match details by ID.
    ///
    /// Mirrors TS: match(id).
    /// - Primary: GET /matches/{id} with 125s timeout (slow client).
    /// - Parallel: GET /matches/fact/{id} (best-effort, 15s timeout).
    /// - Returns hydrated match object with facts merged into players.
    /// - Envelope: {"count": N, "matches": [{"match": {...}}]} → inner match object.
    pub async fn match_info(&self, match_id: &str) -> Result<serde_json::Value, ApiError> {
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
                    .is_some_and(|matches| !matches.is_empty()) =>
            {
                payload
            }
            _ => {
                let payload = self.get_json_slow(&match_url).await?;
                if fact_result.is_none() {
                    fact_result = self.get_json(&fact_url).await.ok();
                }
                payload
            }
        };

        // Preserve the complete MatchRecord. The renderer requires the sibling
        // `match`, `players`, and `bans` fields; unwrapping only `.match`
        // produced an empty custom scoreboard and led to the web-page workaround.
        let mut record = val
            .get("matches")
            .and_then(|m| m.as_array())
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_else(|| std::mem::take(&mut val));

        let player_ids = record
            .get("players")
            .and_then(|players| players.as_array())
            .into_iter()
            .flatten()
            .filter_map(|player| json_id(player.get("player_id")))
            .filter(|id| id.parse::<u64>().is_ok_and(|id| id > 0))
            .collect::<Vec<_>>();
        let moderation_by_id = if player_ids.is_empty() {
            HashMap::new()
        } else {
            let bulk_url = format!("{}/players/bulk?ids={}", self.base, player_ids.join(","));
            self.get_json(&bulk_url)
                .await
                .ok()
                .and_then(|payload| {
                    payload
                        .get("players")
                        .and_then(|rows| rows.as_array())
                        .cloned()
                })
                .unwrap_or_default()
                .into_iter()
                .filter_map(|row| json_id(row.get("id")).map(|id| (id, row)))
                .collect::<HashMap<_, _>>()
        };

        if let Some(obj) = record.as_object_mut() {
            // Mirror TS hydrateMatchPlayer: promote the joined profile snapshot
            // fields used by the standalone scoreboard into each player row.
            if let Some(players) = obj.get_mut("players").and_then(|v| v.as_array_mut()) {
                for player in players {
                    let Some(player_obj) = player.as_object_mut() else {
                        continue;
                    };
                    let snapshot = player_obj
                        .get("profile_snapshot")
                        .and_then(|v| v.as_object())
                        .cloned()
                        .unwrap_or_default();
                    for (target, source) in [
                        ("final_match_level", "level"),
                        ("tier", "kbm_tier"),
                        ("kbm_tier", "kbm_tier"),
                        ("kbm_rank", "kbm_rank"),
                        ("queue_elo", "queue_elo"),
                        ("cheater", "cheater"),
                        ("sus_count", "sus_count"),
                        ("verified", "verified"),
                    ] {
                        if let Some(value) = snapshot.get(source).filter(|v| !v.is_null()) {
                            player_obj.insert(target.to_string(), value.clone());
                        }
                    }
                    let player_id = json_id(player_obj.get("player_id"));
                    if let Some(moderation) = player_id
                        .as_ref()
                        .and_then(|player_id| moderation_by_id.get(player_id))
                    {
                        merge_public_moderation(player_obj, moderation);
                    }
                }
            }
            obj.insert(
                "facts".to_string(),
                fact_result
                    .and_then(|facts| facts.get("players").cloned())
                    .unwrap_or_else(|| serde_json::json!([])),
            );
        }

        Ok(record)
    }

    /// Get all champion names.
    pub async fn champion_names(&self) -> Result<Vec<String>, ApiError> {
        let url = format!("{}/champions", self.base);
        let val: serde_json::Value = match self.response_cache.get(&url).await {
            Some(cached) => cached,
            None => {
                let value = self.get_json(&url).await?;
                self.response_cache.insert(url, value.clone()).await;
                value
            }
        };
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
    /// Implement champions.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn champions(&self) -> Result<serde_json::Value, ApiError> {
        let url = format!("{}/champions", self.base);
        self.get_json(&url).await
    }

    /// Implement champion_id.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn champion_id(&self, name: &str) -> Result<Option<String>, ApiError> {
        let value = self.champions().await?;
        Ok(value.as_array().and_then(|rows| {
            rows.iter().find_map(|row| {
                row.get("name")
                    .and_then(|value| value.as_str())
                    .filter(|value| value.eq_ignore_ascii_case(name))
                    .and_then(|_| json_id(row.get("id")))
            })
        }))
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
        filters: &HistoryFilters,
    ) -> Result<Vec<serde_json::Value>, ApiError> {
        let mut url = format!(
            "{}/players/{}/matches?limit={}&offset={}",
            self.base,
            encode(player_id),
            limit,
            filters.offset
        );
        for (key, value) in [
            ("queueId", filters.queue_id.as_deref()),
            ("championId", filters.champion_id.as_deref()),
            ("winStatus", filters.win_status.as_deref()),
        ] {
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                url.push_str(&format!("&{key}={}", encode(value)));
            }
        }
        let val: serde_json::Value = self.get_json_slow(&url).await?;
        match &val {
            serde_json::Value::Array(arr) => Ok(arr.to_vec()),
            _ => Ok(vec![val]),
        }
    }

    /// Return the newest match observed for a player after applying the
    /// backend-owned three-minute history TTL. `refresh=true` makes the
    /// read-through contract explicit; the backend performs no Hi-Rez call
    /// while fresh and synchronously persists an expired refresh.
    pub async fn latest_player_match(
        &self,
        player_id: &str,
    ) -> Result<Option<serde_json::Value>, ApiError> {
        let value = self
            .get_json_slow(&latest_player_match_url(&self.base, player_id))
            .await?;
        Ok(match value {
            serde_json::Value::Array(rows) => rows.into_iter().next(),
            serde_json::Value::Null => None,
            row => Some(row),
        })
    }

    /// Implement player_champions.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn player_champions(
        &self,
        player_id: &str,
    ) -> Result<Vec<serde_json::Value>, ApiError> {
        let value = self
            .get_json(&format!(
                "{}/players/{}/champions",
                self.base,
                encode(player_id)
            ))
            .await?;
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    /// Implement leaderboard.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn leaderboard(
        &self,
        category: &str,
        metric: Option<&str>,
        role: Option<&str>,
        champion_id: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        let path = match category {
            "class" => "class",
            "champion" => "champion-elo",
            _ => "performance",
        };
        let mut url = format!(
            "{}/players/leaderboard/{path}?limit=10&queueId=486",
            self.base
        );
        for (key, value) in [
            ("metric", metric),
            ("role", role),
            ("championId", champion_id),
        ] {
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                url.push_str(&format!("&{key}={}", encode(value)));
            }
        }
        self.get_json_slow(&url).await
    }

    /// Implement activity.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn activity(&self) -> Result<serde_json::Value, ApiError> {
        let presence_url = format!("{}/stats/presence?view=activity-v4", self.base);
        let overview_url = format!("{}/matches/overview?view=activity-v3", self.base);
        let (presence, overview) = tokio::try_join!(
            self.get_json_slow(&presence_url),
            self.get_json_slow(&overview_url)
        )?;
        Ok(serde_json::json!({"presence":presence,"overview":overview}))
    }

    /// Implement status.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn status(&self) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("{}/system/hirez-status", self.base))
            .await
    }

    /// Check player live match status.
    ///
    /// Mirrors TS: liveMatch(input) → resolvePlayer → GET /live/players/{id}.
    /// Returns object with `in_game` boolean.
    pub async fn live_match(&self, player: &str) -> Result<serde_json::Value, ApiError> {
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
    pub async fn loadouts(&self, player_id: &str) -> Result<Vec<serde_json::Value>, ApiError> {
        Ok(self.loadouts_response(player_id).await?.loadouts)
    }

    /// Implement loadouts_response.
    ///
    /// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
    ///
    pub async fn loadouts_response(&self, player_id: &str) -> Result<LoadoutsResponse, ApiError> {
        let url = format!(
            "{}/players/{}/loadouts?refresh=false",
            self.base,
            encode(player_id)
        );
        let val: serde_json::Value = self.get_json(&url).await?;
        let loadouts = match val.get("loadouts").and_then(|v| v.as_array()) {
            Some(arr) => arr.to_vec(),
            None => match &val {
                serde_json::Value::Array(arr) => arr.to_vec(),
                _ => vec![val.clone()],
            },
        };
        Ok(LoadoutsResponse {
            loadouts,
            refreshed: val
                .get("refreshed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            refresh_error: val
                .get("refresh_error")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        })
    }

    /// Mirrors the TS explicit refresh endpoint; the backend owns its guard.
    pub async fn refresh_loadouts(&self, player_id: &str) -> Result<LoadoutsResponse, ApiError> {
        let url = format!(
            "{}/players/{}/loadouts/refresh",
            self.base,
            encode(player_id)
        );
        let val: serde_json::Value = self.post_empty(&url).await?;
        let loadouts = val
            .get("loadouts")
            .and_then(|v| v.as_array())
            .map(|rows| rows.to_vec())
            .unwrap_or_default();
        Ok(LoadoutsResponse {
            loadouts,
            refreshed: val
                .get("refreshed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            refresh_error: val
                .get("refresh_error")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        })
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
    ) -> Result<serde_json::Value, ApiError> {
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
    pub async fn ranked_maps(&self, limit: usize) -> Result<Vec<serde_json::Value>, ApiError> {
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
    ) -> Result<Vec<serde_json::Value>, ApiError> {
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
    ) -> Result<Vec<serde_json::Value>, ApiError> {
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
