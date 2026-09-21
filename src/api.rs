//! Call PaladinsCat /v1 endpoints and normalize backend JSON for bot commands.
//!
//! Normal and slow clients attach optional service tokens; only the static champion roster is cached.
//! GET rate-limit retries are bounded; mutations use explicit PUT, POST, or DELETE requests.
//! refs: doc: documents/05-operations/runbooks/discord-bot.md

use crate::service_auth::ServiceTokenProvider;
use moka::future::Cache;
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use reqwest::Client as HttpClient;
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Debug)]
/// Represent an API failure with an optional HTTP status, user-facing message, and optional backend
/// error code.
/// Transport and service-auth failures may have no status; response failures preserve the status.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
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
            "http://backend:3005/api/v1/players/716515038/matches?limit=1&offset=0"
        );
    }

    #[test]
    fn request_destination_cannot_escape_configured_backend() {
        let client = ApiClient::new("http://backend:3005/api", None);
        for url in [
            "http://attacker.invalid/api/v1/players",
            "http://backend:3006/api/v1/players",
            "http://backend:3005/api/v1/../../admin",
            "http://user@backend:3005/api/v1/players",
            "https://backend:3005/api/v1/players",
        ] {
            assert!(client.request_url(url).is_err(), "{url}");
        }
        let url = client
            .request_url("http://backend:3005/api/v1/players?player=https%3A%2F%2Fattacker.invalid")
            .unwrap();
        assert_eq!(url.host_str(), Some("backend"));
        assert_eq!(url.path(), "/api/v1/players");
        let public_http = ApiClient::new("http://example.com/api", None);
        assert!(public_http
            .request_url("http://example.com/api/v1/players")
            .is_err());
        let public_https = ApiClient::new("https://example.com/api", None);
        assert!(public_https
            .request_url("https://example.com/api/v1/players")
            .is_ok());
    }

    #[tokio::test]
    async fn backend_redirect_is_not_followed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let responder = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        let client = ApiClient::new(&format!("http://{address}"), None);
        let error = client
            .get_json(&format!("{}/players", client.base))
            .await
            .unwrap_err();
        assert_eq!(error.status, Some(302));
        responder.await.unwrap();
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
/// URL path segments and GET query values are percent-encoded. GET responses to 429 receive bounded backoff.
/// Mirrors TS: PaladinsCatApi with service token auth.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
#[derive(Clone)]
/// Own normal (15 s) and slow (125 s) HTTP clients, a normalized /v1 base URL, optional service
/// identity, and a ten-minute champion-list cache.
/// GET retries only HTTP 429, at most three attempts with 500 ms and 1 s waits; profile and match
/// reads are not cached here.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct ApiClient {
    inner: HttpClient,
    inner_slow: HttpClient,
    base: String,
    /// Short-lived Keycloak service identity; private key remains external to the repo.
    /// refs: none
    service_auth: Option<Arc<ServiceTokenProvider>>,
    /// Short-lived cache only for the static champion roster used by autocomplete.
    /// refs: none
    response_cache: Cache<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
/// Carry loadout JSON rows, the backend refresh flag, and an optional refresh error.
/// A refresh error can accompany a successful HTTP response and existing loadouts.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct LoadoutsResponse {
    pub loadouts: Vec<serde_json::Value>,
    pub refreshed: bool,
    pub refresh_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
/// Carry optional queue, champion, and win-status strings plus a zero-based usize offset.
/// Only nonempty optional values become encoded history query parameters; the caller supplies the
/// limit separately.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct HistoryFilters {
    pub queue_id: Option<String>,
    pub champion_id: Option<String>,
    pub win_status: Option<String>,
    pub offset: usize,
}

/// Encode a path segment for use in URLs.
/// refs: none
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
        "{}/players/{}/matches?limit=1&offset=0",
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
/// refs: none
fn clamp(val: usize, min: usize, max: usize) -> usize {
    val.max(min).min(max)
}

/// Map lobby scope string to (tierMin, tierMax) — mirrors ranked-lobby.ts.
/// refs: none
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
    ///
    /// Normalize base to one trailing /v1 and allocate clients/cache without sending HTTP; client
    /// construction panics if reqwest initialization fails.
    ///
    /// I/O: `&str` (base), `Option<ServiceTokenProvider>` -> `ApiClient`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub fn new(base: &str, service_auth: Option<ServiceTokenProvider>) -> Self {
        Self {
            inner: HttpClient::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()
                .expect("build reqwest client"),
            inner_slow: HttpClient::builder()
                .redirect(reqwest::redirect::Policy::none())
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
    /// refs: none
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ApiError> {
        self.get_json_impl(&self.inner, url).await
    }

    /// Send a GET request with slow timeout (125s) — used for match endpoints.
    /// refs: none
    async fn get_json_slow(&self, url: &str) -> Result<serde_json::Value, ApiError> {
        self.get_json_impl(&self.inner_slow, url).await
    }

    /// Keep all request authority in trusted configuration; user values may only
    /// affect paths/queries within this backend's API namespace.
    fn request_url(&self, value: &str) -> Result<reqwest::Url, ApiError> {
        let invalid = || ApiError {
            status: None,
            message: "The PaladinsCat request destination is not allowed.".into(),
            code: Some("INVALID_API_DESTINATION".into()),
        };
        let mut base_url = reqwest::Url::parse(&self.base).map_err(|_| invalid())?;
        let candidate = reqwest::Url::parse(value).map_err(|_| invalid())?;
        // HTTP is limited to loopback tests and the documented private Compose service.
        let private_http = base_url.scheme() == "http"
            && (matches!(
                base_url.host_str(),
                Some("127.0.0.1" | "[::1]" | "localhost")
            ) || (matches!(base_url.host_str(), Some("backend" | "backend-rust-api"))
                && base_url.port() == Some(3005))
                || (base_url.host_str() == Some("paladinscat-backend")
                    && base_url.port() == Some(3001)));
        if !(base_url.scheme() == "https" || private_http)
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || candidate.origin() != base_url.origin()
            || !candidate.username().is_empty()
            || candidate.password().is_some()
            || candidate.fragment().is_some()
            || !(candidate.path() == base_url.path()
                || candidate
                    .path()
                    .starts_with(&format!("{}/", base_url.path())))
        {
            return Err(invalid());
        }
        base_url.set_path(candidate.path());
        base_url.set_query(candidate.query());
        Ok(base_url)
    }

    async fn post_empty(&self, url: &str) -> Result<serde_json::Value, ApiError> {
        let mut request = self.inner.post(self.request_url(url)?);
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
        let destination = self.request_url(url)?;

        for (attempt, &delay_ms) in delays.iter().enumerate() {
            let mut req = client.get(destination.clone());
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
    ///
    /// Send an encoded player query and return the raw enriched JSON. Authentication, transport,
    /// non-success status, and JSON failures return ApiError.
    ///
    /// I/O: `&str` (player input) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /players/discord
    pub async fn discord_player(&self, name: &str) -> Result<serde_json::Value, ApiError> {
        let url = format!("{}/players/discord?player={}", self.base, encode(name));
        let val = self.get_json(&url).await?;
        Ok(val)
    }

    /// Return the default player saved for a Discord user.
    ///
    /// GET the encoded user/slot mapping and unwrap its player field, falling back to the whole
    /// response. HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (discord user id), `&str` (slot) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /players/discord/saved-player
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
    ///
    /// PUT user/player/slot JSON and unwrap player from the response. This persists the backend
    /// mapping; HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (discord user id), `&str` (player id), `&str` (slot) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: PUT /players/discord/saved-player
    pub async fn save_discord_player(
        &self,
        discord_user_id: &str,
        player_id: &str,
        slot: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let url = format!("{}/players/discord/saved-player", self.base);
        let mut req = self
            .inner
            .put(self.request_url(&url)?)
            .json(&serde_json::json!({
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

    /// Delete the saved default player for a Discord user/slot.
    ///
    /// DELETE the encoded user/slot mapping; absent or nonnumeric deleted counts become zero.
    /// HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (discord user id), `&str` (slot) -> `Result<usize, ApiError>` (rows removed)
    /// refs: endpoints: DELETE /players/discord/saved-player
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
        let mut request = self.inner.delete(self.request_url(&url)?);
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
    ///
    /// Resolve the input, GET the profile with ratings, and unwrap an object-valued player field.
    /// Resolution or HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (name or id) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /players/search, GET /players/{id}
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

    /// Resolve an exact name or numeric ID through the backend identity owner.
    /// I/O: player input -> typed resolved JSON or error; no local search fallback.
    /// Ambiguous identities require an ID; an older backend response is rejected.
    /// refs: endpoints: GET /players/search?exact=true
    pub async fn resolve_player(&self, input: &str) -> Result<serde_json::Value, ApiError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(player_not_found(trimmed));
        }
        let url = format!(
            "{}/players/search?name={}&exact=true",
            self.base,
            encode(trimmed)
        );
        let result = self.get_json(&url).await?;
        match result.get("state").and_then(serde_json::Value::as_str) {
            Some("resolved" | "unverified_id") if json_id(result.get("id")).is_some() => Ok(result),
            Some("ambiguous") => Err(ApiError {
                status: Some(409),
                code: Some("PLAYER_AMBIGUOUS".to_owned()),
                message: "Multiple players share this name. Use a player ID.".to_owned(),
            }),
            _ => Err(player_not_found(trimmed)),
        }
    }

    async fn resolve_player_id(&self, input: &str) -> Result<String, ApiError> {
        let resolved = self.resolve_player(input).await?;
        json_id(resolved.get("id")).ok_or_else(|| player_not_found(input.trim()))
    }

    /// Fetch a complete match record and enrich player rows for rendering.
    ///
    /// Fetch /matches/batch and optional facts concurrently with the normal 15-second client.
    /// On a batch miss/error, use /matches/{id} with the 125-second client and retry missing facts.
    /// Preserve the first complete matches entry (match, players, bans), or return the raw response.
    /// Promote non-null profile-snapshot fields and best-effort /players/bulk moderation into players;
    /// attach fact-player rows as facts (empty on absence). Supplemental failures are tolerated;
    /// required fallback HTTP/auth/JSON failures return ApiError. Arbitrary successful JSON is retained.
    ///
    /// I/O: `&str` (match id) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /matches/batch, GET /matches/{id}, GET /matches/fact/{match_id}, GET /players/bulk
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
    ///
    /// GET /champions only on a ten-minute cache miss; accept strings or object name fields and
    /// skip other rows. Non-array JSON yields an empty vector; HTTP/auth/JSON failures return
    /// ApiError.
    ///
    /// I/O: () -> `Result<Vec<String>, ApiError>`
    /// refs: endpoints: GET /champions
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
    /// refs: endpoints: GET /champions
    #[allow(dead_code)] // Kept for potential future use
    /// Get the full champion list as raw JSON.
    ///
    /// Fetch uncached /champions JSON; authentication, transport, non-success status, and JSON
    /// failures return ApiError.
    ///
    /// I/O: () -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /champions
    pub async fn champions(&self) -> Result<serde_json::Value, ApiError> {
        let url = format!("{}/champions", self.base);
        self.get_json(&url).await
    }

    /// Resolve a champion name (case-insensitive) to its ID from the champion list.
    ///
    /// Fetch champions over HTTP; return None if no matching name has an ID. HTTP/auth/JSON
    /// failures return ApiError.
    ///
    /// I/O: `&str` (name) -> `Result<Option<String>, ApiError>`
    /// refs: endpoints: GET /champions
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
    /// Uses slow client (125s timeout) — large history sets can be slow.
    ///
    /// Encode nonempty filters and pass limit/offset without local clamping; an array is returned
    /// directly and other JSON is wrapped as one row. HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (player id), `usize` (limit), `&HistoryFilters` -> `Result<Vec<serde_json::Value>, ApiError>`
    /// refs: endpoints: GET /players/{id}/matches
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
    /// backend-owned three-minute history TTL. Freshness is backend-owned; the
    /// read carries no refresh flag so the developer-API guard is not tripped.
    ///
    /// Request limit=1 using the slow client; return the first array row, None
    /// for null/empty arrays, or the single JSON value. HTTP/auth/JSON failures
    /// return ApiError.
    ///
    /// I/O: `&str` (player id) -> `Result<Option<serde_json::Value>, ApiError>`
    /// refs: endpoints: GET /players/{id}/matches
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

    /// Get a player's champion roster.
    ///
    /// GET the encoded player roster; non-array JSON yields an empty vector. HTTP/auth/JSON
    /// failures return ApiError.
    ///
    /// I/O: `&str` (player id) -> `Result<Vec<serde_json::Value>, ApiError>`
    /// refs: endpoints: GET /players/{id}/champions
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

    /// Get ranked leaderboard rows for a category (class / champion-elo / performance).
    ///
    /// Map class to class, champion to champion-elo, and other categories to performance; request
    /// ten rows in queue 486 with nonempty encoded filters. HTTP/auth/JSON failures return
    /// ApiError.
    ///
    /// I/O: `&str` (category), `Option<&str>` (metric), `Option<&str>` (role), `Option<&str>` (champion id) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /players/leaderboard/class, GET /players/leaderboard/champion-elo, GET /players/leaderboard/performance
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

    /// Get live activity: presence and match-overview fetched in parallel.
    ///
    /// Fail if either concurrent slow-client GET fails; otherwise return an object containing
    /// presence and overview. HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: () -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /stats/presence, GET /matches/overview
    pub async fn activity(&self) -> Result<serde_json::Value, ApiError> {
        let presence_url = format!("{}/stats/presence?view=activity-v4", self.base);
        let overview_url = format!("{}/matches/overview?view=activity-v3", self.base);
        let (presence, overview) = tokio::try_join!(
            self.get_json_slow(&presence_url),
            self.get_json_slow(&overview_url)
        )?;
        Ok(serde_json::json!({"presence":presence,"overview":overview}))
    }

    /// Get the Hi-Rez API status (`/system/hirez-status`).
    ///
    /// Return the raw status JSON over HTTP; authentication, transport, non-success status, and
    /// JSON failures return ApiError.
    ///
    /// I/O: () -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /system/hirez-status
    pub async fn status(&self) -> Result<serde_json::Value, ApiError> {
        self.get_json(&format!("{}/system/hirez-status", self.base))
            .await
    }

    /// Check player live match status.
    ///
    /// Mirrors TS: liveMatch(input) → resolvePlayer → GET /live/players/{id}.
    /// Returns object with `in_game` boolean.
    ///
    /// Resolve the player and GET live JSON; object responses receive in_game=true exactly when
    /// match exists and is non-null. Resolution or HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (player) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /players/search, GET /live/players/{player_id}
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
    ///
    /// Delegate to the non-refreshing structured loadout read and return only its rows;
    /// HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (player id) -> `Result<Vec<serde_json::Value>, ApiError>`
    /// refs: endpoints: GET /players/{id}/loadouts
    pub async fn loadouts(&self, player_id: &str) -> Result<Vec<serde_json::Value>, ApiError> {
        Ok(self.loadouts_response(player_id).await?.loadouts)
    }

    /// Get a player's loadouts with `refresh=false` as a structured response.
    ///
    /// GET with refresh=false; accept a loadouts array, a top-level array, or wrap other JSON as
    /// one row. Missing refreshed is false; refresh_error stays separate from HTTP/auth/JSON
    /// ApiError failures.
    ///
    /// I/O: `&str` (player id) -> `Result<LoadoutsResponse, ApiError>`
    /// refs: endpoints: GET /players/{id}/loadouts
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
    ///
    /// POST an empty refresh request, potentially refreshing backend loadout storage. Missing
    /// loadouts yields an empty vector; backend refresh_error can accompany success, while
    /// HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (player id) -> `Result<LoadoutsResponse, ApiError>`
    /// refs: endpoints: POST /players/{id}/loadouts/refresh
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
    ///
    /// Route: GET /champions/{slug}/page-data?tierMin={}&tierMax={}
    ///
    /// GET the encoded champion slug with optional lobby tier bounds; HTTP/auth/JSON failures
    /// return ApiError.
    ///
    /// I/O: `&str` (slug), `&str` (scope) -> `Result<serde_json::Value, ApiError>`
    /// refs: endpoints: GET /champions/{id}/page-data
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
    ///
    /// Return array rows or wrap non-array JSON as one row. HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `usize` (limit) -> `Result<Vec<serde_json::Value>, ApiError>`
    /// refs: endpoints: GET /stats/maps
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
    ///
    /// Prefer the data array, then a top-level array, otherwise wrap JSON as one row.
    /// HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `usize` (limit) -> `Result<Vec<serde_json::Value>, ApiError>`
    /// refs: endpoints: GET /matches/compositions
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
    ///
    /// Apply optional lobby tier bounds and return array rows or wrap other JSON as one row.
    /// HTTP/auth/JSON failures return ApiError.
    ///
    /// I/O: `&str` (scope), `usize` (limit) -> `Result<Vec<serde_json::Value>, ApiError>`
    /// refs: endpoints: GET /stats/items
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
