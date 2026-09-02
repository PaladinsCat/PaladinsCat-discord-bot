//! Slash command handlers — dispatches InteractionCreate events.
//! refs: none

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::Event;
use twilight_http::Client as HttpClient;
use twilight_model::application::interaction::{
    application_command::{CommandData, CommandDataOption, CommandOptionValue},
    Interaction, InteractionData, InteractionType,
};
use twilight_model::channel::message::component::{
    ActionRow, Button, ButtonStyle, Component, SelectMenu, SelectMenuOption, SelectMenuType,
};
use twilight_model::channel::message::embed::Embed;
use twilight_model::channel::message::MessageFlags;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use unicode_normalization::UnicodeNormalization;

use crate::api::{ApiClient, ApiError, HistoryFilters};
use crate::cache::RenderCache;
use crate::embeds;
use crate::image::ImageService;

/// Loadout session — maps a token to player/loadout data with an expiration.
/// refs: none
#[derive(Clone)]
struct LoadoutSession {
    user_id: String,
    player: Value,
    loadouts: Vec<Value>,
    expires_at: u64,
}

struct PlayerInput {
    query: String,
    resolved: Option<Value>,
}

/// 5-minute TTL for loadout sessions.
/// refs: none
const LOADOUT_SESSION_TTL_SECS: u64 = 5 * 60;
const IMAGE_COOLDOWN_MS: i64 = 10 * 1000;

/// Maximum time to wait for an image render before falling back to an embed.
/// The interaction is deferred first, so this bounds only how long the user
/// waits for the image (or the embed fallback), not Discord's 3s ACK window.
/// refs: none
const RENDER_TIMEOUT: Duration = Duration::from_secs(22);

/// Module-level session store.  Shared between command and component handlers.
/// refs: none
static LOADOUT_SESSIONS: LazyLock<RwLock<HashMap<String, LoadoutSession>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static HISTORY_SESSIONS: LazyLock<RwLock<HashMap<String, HistorySession>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static IMAGE_COOLDOWNS: LazyLock<RwLock<HashMap<String, i64>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static USER_RATE_LIMITS: LazyLock<RwLock<HashMap<String, RateWindow>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static GUILD_RATE_LIMITS: LazyLock<RwLock<HashMap<String, RateWindow>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static WEBHOOK_HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(20))
        .build()
        .expect("build Discord webhook client")
});

struct Handler {
    api: Arc<ApiClient>,
    _cache: Arc<RenderCache>,
    http: Arc<HttpClient>,
    app_id: twilight_model::id::Id<twilight_model::id::marker::ApplicationMarker>,
    web_url: String,
    image_service: Option<Arc<ImageService>>,
    discord_cache: Arc<InMemoryCache>,
}

#[derive(Clone)]
struct HistorySession {
    user_id: String,
    player_id: String,
    player_name: String,
    filters: HistoryFilters,
    page: usize,
    expires_at: u64,
}

#[derive(Clone)]
struct RateWindow {
    started: Instant,
    count: u32,
}

/// Main event dispatcher — routes gateway events to command handlers.
/// refs: none
#[allow(clippy::too_many_arguments)]
/// Dispatch a Discord gateway event to the matching command handler.
///
/// I/O: `Event`, `Arc<ApiClient>`, `Arc<RenderCache>`, `Arc<HttpClient>`, `String` (web url), `Option<Arc<ImageService>>`, `Option<Id<ApplicationMarker>>`, `Option<Id<GuildMarker>>`, `Arc<AtomicBool>`, `Arc<InMemoryCache>`, `bool` (social enabled) -> ()
/// refs: none
pub async fn handle_event(
    event: Event,
    api: Arc<ApiClient>,
    render_cache: Arc<RenderCache>,
    http: Arc<HttpClient>,
    web_url: String,
    image_service: Option<Arc<ImageService>>,
    registration_app_id: Option<
        twilight_model::id::Id<twilight_model::id::marker::ApplicationMarker>,
    >,
    development_guild_id: Option<twilight_model::id::Id<twilight_model::id::marker::GuildMarker>>,
    registration_started: Arc<AtomicBool>,
    discord_cache: Arc<InMemoryCache>,
    social_commands_enabled: bool,
) {
    discord_cache.update(&event);
    match event {
        Event::Ready(ready) => {
            tracing::info!("Gateway connected");
            if let Some(app_id) = claim_registration(&registration_started, registration_app_id) {
                let guild_ids: Vec<_> = ready.guilds.iter().map(|guild| guild.id).collect();
                match crate::register::register_commands(
                    &http,
                    app_id,
                    development_guild_id,
                    &guild_ids,
                    social_commands_enabled,
                )
                .await
                {
                    Ok(result) => {
                        tracing::info!(scope = %result.scope, registered = result.registered, cleared = result.cleared_guild_scopes, failed = result.failed_guild_scopes, "Commands registered")
                    }
                    Err(error) => tracing::error!(%error, "Command registration failed"),
                }
            }
        }
        Event::InteractionCreate(interaction_box) => {
            let interaction = (*interaction_box).0;
            match interaction.kind {
                InteractionType::ApplicationCommand => {
                    let h = Arc::new(Handler {
                        api: api.clone(),
                        _cache: render_cache.clone(),
                        http: http.clone(),
                        app_id: interaction.application_id,
                        web_url: web_url.clone(),
                        image_service: image_service.clone(),
                        discord_cache: discord_cache.clone(),
                    });
                    tokio::spawn(async move { h.handle_command(interaction).await });
                }
                InteractionType::ApplicationCommandAutocomplete => {
                    let h = Arc::new(Handler {
                        api: api.clone(),
                        _cache: render_cache.clone(),
                        http: http.clone(),
                        app_id: interaction.application_id,
                        web_url: web_url.clone(),
                        image_service: image_service.clone(),
                        discord_cache: discord_cache.clone(),
                    });
                    tokio::spawn(async move { h.handle_autocomplete(interaction).await });
                }
                InteractionType::MessageComponent => {
                    let h = Arc::new(Handler {
                        api: api.clone(),
                        _cache: render_cache.clone(),
                        http: http.clone(),
                        app_id: interaction.application_id,
                        web_url: web_url.clone(),
                        image_service: image_service.clone(),
                        discord_cache: discord_cache.clone(),
                    });
                    tokio::spawn(async move { h.handle_component(interaction).await });
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn claim_registration(
    started: &AtomicBool,
    app_id: Option<twilight_model::id::Id<twilight_model::id::marker::ApplicationMarker>>,
) -> Option<twilight_model::id::Id<twilight_model::id::marker::ApplicationMarker>> {
    app_id.filter(|_| !started.swap(true, Ordering::AcqRel))
}

// ——— Helpers to extract user_id from interaction ———

fn extract_user_id(interaction: &Interaction) -> Option<String> {
    interaction
        .member
        .as_ref()
        .and_then(|m| m.user.as_ref().map(|u| u.id.get().to_string()))
        .or_else(|| interaction.user.as_ref().map(|u| u.id.get().to_string()))
}

// ——— Session helpers ———

/// Prune expired sessions.  Call before inserting a new session.
/// refs: none
fn prune_sessions() {
    let now = chrono::Utc::now().timestamp() as u64;
    let mut sessions = LOADOUT_SESSIONS.write().unwrap();
    sessions.retain(|_, s| s.expires_at > now);
    HISTORY_SESSIONS
        .write()
        .unwrap()
        .retain(|_, session| session.expires_at > now);
}

/// Clone a session out of the store, dropping the lock immediately.
/// refs: none
fn get_session(token: &str) -> Option<LoadoutSession> {
    let sessions = LOADOUT_SESSIONS.read().unwrap();
    sessions.get(token).cloned()
}

/// Remove a session by token.
/// refs: none
fn remove_session(token: &str) -> bool {
    let mut sessions = LOADOUT_SESSIONS.write().unwrap();
    sessions.remove(token).is_some()
}

/// Insert a session into the store.
/// refs: none
fn insert_session(token: &str, session: LoadoutSession) {
    let mut sessions = LOADOUT_SESSIONS.write().unwrap();
    sessions.insert(token.to_string(), session);
}

fn claim_image_cooldown(user_id: &str) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut cooldowns = IMAGE_COOLDOWNS.write().unwrap();
    cooldowns.retain(|_, expires_at| *expires_at > now);
    if let Some(expires_at) = cooldowns
        .get(user_id)
        .filter(|expires_at| **expires_at > now)
    {
        let remaining = (*expires_at - now + 999) / 1000;
        return Err(format!("Image cooldown: try again in {remaining}s."));
    }
    cooldowns.insert(user_id.to_string(), now + IMAGE_COOLDOWN_MS);
    Ok(())
}

fn claim_window(
    store: &RwLock<HashMap<String, RateWindow>>,
    key: String,
    limit: u32,
    window: Duration,
) -> bool {
    let now = Instant::now();
    let mut windows = store.write().unwrap();
    windows.retain(|_, value| now.duration_since(value.started) < window);
    let entry = windows.entry(key).or_insert(RateWindow {
        started: now,
        count: 0,
    });
    if now.duration_since(entry.started) >= window {
        *entry = RateWindow {
            started: now,
            count: 0,
        };
    }
    if entry.count >= limit {
        return false;
    }
    entry.count += 1;
    true
}

fn claim_command_rate(interaction: &Interaction) -> bool {
    let user_ok = extract_user_id(interaction)
        .is_none_or(|user| claim_window(&USER_RATE_LIMITS, user, 8, Duration::from_secs(10)));
    let guild_ok = interaction.guild_id.is_none_or(|guild| {
        claim_window(
            &GUILD_RATE_LIMITS,
            guild.get().to_string(),
            40,
            Duration::from_secs(10),
        )
    });
    user_ok && guild_ok
}

fn valid_match_id(id: &str) -> bool {
    (6..=20).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit())
}

fn match_id_from_history_row(row: &Value) -> Option<String> {
    value_id(row.get("match_id")).filter(|id| valid_match_id(id))
}

impl Handler {
    async fn handle_command(&self, interaction: Interaction) {
        let Some(cmd_data) = extract_command_data(&interaction.data) else {
            return self
                .send_response(
                    interaction.id,
                    &interaction.token,
                    InteractionResponse {
                        kind: InteractionResponseType::Pong,
                        data: None,
                    },
                )
                .await;
        };

        if cmd_data.name == "help" {
            self.help(&interaction).await;
            return;
        }
        if cmd_data.name == "privacy" {
            self.privacy(&interaction).await;
            return;
        }
        if !claim_command_rate(&interaction) {
            self.reply_ephemeral_text(
                &interaction,
                "Too many bot commands at once. Try again in a few seconds.",
            )
            .await;
            return;
        }

        // Match the TS lifecycle: acknowledge every potentially slow command
        // before doing any backend lookup. Otherwise Discord invalidates the
        // interaction token while the Rust handler is still waiting on I/O.
        self.defer_response(
            &interaction,
            matches!(cmd_data.name.as_str(), "save" | "forget"),
        )
        .await;

        match cmd_data.name.as_str() {
            "player" | "profile" => self.player(&interaction, &cmd_data.options).await,
            "match" => self.match_cmd(&interaction, &cmd_data.options).await,
            "history" => self.history(&interaction, &cmd_data.options).await,
            "current" => self.current(&interaction, &cmd_data.options).await,
            "loadout" => self.loadout(&interaction, &cmd_data.options).await,
            "champion" => self.champion(&interaction, &cmd_data.options).await,
            "maps" | "composition" | "items" => {
                self.stats(&interaction, &cmd_data.name, &cmd_data.options)
                    .await
            }
            "save" => self.save(&interaction, &cmd_data.options).await,
            "forget" => self.forget(&interaction, &cmd_data.options).await,
            "champions" => self.player_champions(&interaction, &cmd_data.options).await,
            "leaderboard" => self.leaderboard(&interaction, &cmd_data.options).await,
            "activity" => self.activity(&interaction).await,
            "status" => self.status(&interaction).await,
            "random" => self.random(&interaction, &cmd_data.options).await,
            "teams" => self.teams(&interaction).await,
            "Paladins Profile" | "Paladins History" | "Paladins Current" => {
                self.user_context(&interaction, cmd_data).await
            }
            other => {
                tracing::debug!(command = other, "unknown command");
                self.reply_text(&interaction, "Unknown command. Use `/help`.")
                    .await;
            }
        }
    }

    async fn handle_autocomplete(&self, interaction: Interaction) {
        let Some(cmd_data) = extract_command_data(&interaction.data) else {
            return;
        };
        let Some(focused) = cmd_data.options.iter().find_map(|opt| {
            if opt.name == "champion" {
                if let CommandOptionValue::Focused(query, _) = &opt.value {
                    return Some(query.as_str());
                }
            }
            None
        }) else {
            return;
        };

        let names = self.api.champion_names().await.unwrap_or_default();
        let choices = crate::autocomplete::champion_autocomplete_choices(&names, focused);

        let data = InteractionResponseData {
            choices: Some(
                choices
                    .into_iter()
                    .map(|(name, id)| {
                        twilight_model::application::command::CommandOptionChoice {
                            name,
                            name_localizations: None,
                            value: twilight_model::application::command::CommandOptionChoiceValue::String(id),
                        }
                    })
                    .collect(),
            ),
            ..Default::default()
        };

        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::ApplicationCommandAutocompleteResult,
                data: Some(data),
            },
        )
        .await;
    }

    // ——— Component interaction handler ———

    async fn handle_component(&self, interaction: Interaction) {
        let data = match &interaction.data {
            Some(InteractionData::MessageComponent(mc)) => mc,
            _ => return,
        };

        let custom_id = &data.custom_id;
        if let Some(payload) = custom_id.strip_prefix("forget:") {
            let mut parts = payload.split(':');
            let owner = parts.next().unwrap_or_default();
            let slot = parts.next().unwrap_or_default();
            if extract_user_id(&interaction).as_deref() != Some(owner) {
                self.reply_ephemeral_text(
                    &interaction,
                    "Only the account owner can confirm this deletion.",
                )
                .await;
                return;
            }
            self.defer_update(&interaction).await;
            match self.api.forget_discord_player(owner, slot).await {
                Ok(0) => {
                    self.send_webhook_text(
                        &interaction.token,
                        "No saved player existed in that slot.".into(),
                    )
                    .await
                }
                Ok(count) => {
                    self.send_webhook_text(
                        &interaction.token,
                        format!("Deleted {count} saved player link(s)."),
                    )
                    .await
                }
                Err(error) => {
                    self.send_webhook_text(
                        &interaction.token,
                        api_error_message(&error, "Failed to delete the saved player link."),
                    )
                    .await
                }
            }
            return;
        }
        if let Some(history_token) = custom_id.strip_prefix("history-match:") {
            let Some(session) = HISTORY_SESSIONS.read().unwrap().get(history_token).cloned() else {
                self.reply_ephemeral_text(
                    &interaction,
                    "This history menu expired. Run `/history` again.",
                )
                .await;
                return;
            };
            if extract_user_id(&interaction).as_deref() != Some(session.user_id.as_str()) {
                self.reply_ephemeral_text(
                    &interaction,
                    "Only the player who opened this history can select a match.",
                )
                .await;
                return;
            }
            let Some(match_id) = data.values.first().filter(|id| valid_match_id(id)) else {
                self.reply_ephemeral_text(&interaction, "That match is no longer available.")
                    .await;
                return;
            };
            if let Err(message) = claim_image_cooldown(&session.user_id) {
                self.reply_ephemeral_text(&interaction, message).await;
                return;
            }
            tracing::info!(match_id, "history match selected");
            self.send_response(
                interaction.id,
                &interaction.token,
                history_match_loading_response(match_id),
            )
            .await;
            let url = format!("{}/matches/{match_id}", self.web_url);
            let components = history_back_component(history_token);
            if let Some(images) = &self.image_service {
                if let Some(png) = images.cached_match(match_id).await {
                    let filename = format!("paladinscat-match-{match_id}.png");
                    let description = format!("Paladins match {match_id}");
                    self.send_webhook_image_with_components(
                        &url,
                        png,
                        &filename,
                        Some(&description),
                        &components,
                        &interaction.token,
                    )
                    .await;
                    return;
                }
            }
            match self.api.match_info(match_id).await {
                Ok(value) => {
                    let record = value.get("match").unwrap_or(&value);
                    let map = clean_value(record.get("map"), "Unknown map");
                    let queue = clean_value(
                        record.get("queue_name").or_else(|| record.get("queue_id")),
                        "Unknown queue",
                    );
                    let embed = embeds::simple_embed(
                        &format!("Match {match_id} · {map}"),
                        &format!("**{queue}**\n[Open full match]({url})"),
                        Some(&url),
                    );
                    if let Some(png) = self.render_match_scoreboard(&value).await {
                        let filename = format!("paladinscat-match-{match_id}.png");
                        let description = format!("Paladins match {match_id}");
                        self.send_webhook_image_with_components(
                            &url,
                            png,
                            &filename,
                            Some(&description),
                            &components,
                            &interaction.token,
                        )
                        .await;
                        return;
                    }
                    self.send_webhook(&embed, &components, &interaction.token)
                        .await;
                }
                Err(error) => {
                    self.send_webhook_text(
                        &interaction.token,
                        api_error_message(&error, "Failed to fetch that match"),
                    )
                    .await
                }
            }
            return;
        }
        if let Some(payload) = custom_id.strip_prefix("history:") {
            let mut parts = payload.split(':');
            let token = parts.next().unwrap_or_default();
            let action = parts.next().unwrap_or_default();
            let Some(mut session) = HISTORY_SESSIONS.read().unwrap().get(token).cloned() else {
                self.reply_ephemeral_text(
                    &interaction,
                    "This history menu expired. Run `/history` again.",
                )
                .await;
                return;
            };
            if extract_user_id(&interaction).as_deref() != Some(session.user_id.as_str()) {
                self.reply_ephemeral_text(
                    &interaction,
                    "Only the player who opened this history can page it.",
                )
                .await;
                return;
            }
            session.page = match action {
                "next" => session.page.saturating_add(1),
                "prev" => session.page.saturating_sub(1),
                _ => session.page,
            };
            session.filters.offset = session.page * 10;
            self.defer_update(&interaction).await;
            match self
                .api
                .player_history(&session.player_id, 11, &session.filters)
                .await
            {
                Ok(mut rows) => {
                    let has_next = rows.len() > 10;
                    rows.truncate(10);
                    HISTORY_SESSIONS
                        .write()
                        .unwrap()
                        .insert(token.to_string(), session.clone());
                    let embed =
                        embeds::build_history_payload(&session.player_name, &rows, &self.web_url);
                    self.send_webhook(
                        &embed,
                        &history_components(token, session.page, has_next, &rows),
                        &interaction.token,
                    )
                    .await;
                }
                Err(error) => {
                    self.send_webhook_text(
                        &interaction.token,
                        api_error_message(&error, "Failed to fetch match history"),
                    )
                    .await
                }
            }
            return;
        }

        // Loadout selection.
        let Some(token) = custom_id.strip_prefix("loadout:") else {
            return;
        };

        // Clone the session out, dropping the lock immediately.
        let session = match get_session(token) {
            Some(s) => s,
            None => {
                tracing::debug!(token, "loadout session not found");
                self.reply_ephemeral_text(
                    &interaction,
                    "This loadout menu expired. Run `/loadout` again.",
                )
                .await;
                return;
            }
        };

        // Verify the user matches.
        let interaction_user_id = extract_user_id(&interaction);
        if let Some(uid) = &interaction_user_id {
            if uid != &session.user_id {
                tracing::warn!(
                    uid,
                    session_user = session.user_id,
                    "user mismatch on loadout selection"
                );
                self.reply_ephemeral_text(
                    &interaction,
                    "Only the player who opened this menu can choose its loadout.",
                )
                .await;
                return;
            }
        }

        // Check expiry.
        let now = chrono::Utc::now().timestamp() as u64;
        if now >= session.expires_at {
            remove_session(token);
            self.reply_ephemeral_text(
                &interaction,
                "This loadout menu expired. Run `/loadout` again.",
            )
            .await;
            return;
        }

        // Extract the selected loadout ID from the value.
        let Some(selected_value) = data.values.first() else {
            self.reply_ephemeral_text(
                &interaction,
                "That saved loadout is no longer available. Run `/loadout` again.",
            )
            .await;
            return;
        };
        let loadout_id = selected_value.as_str();

        // Find the selected loadout.
        let selected = session
            .loadouts
            .iter()
            .find(|lo| value_id(lo.get("id")) == Some(loadout_id.to_string()));

        let Some(selected) = selected else {
            self.reply_ephemeral_text(
                &interaction,
                "That saved loadout is no longer available. Run `/loadout` again.",
            )
            .await;
            return;
        };

        if let Err(message) = claim_image_cooldown(&session.user_id) {
            self.reply_ephemeral_text(&interaction, message).await;
            return;
        }
        // Delete session only after the selection has been accepted.
        remove_session(token);
        // Preserve the TS component behaviour: replace the select-menu reply.
        self.defer_update(&interaction).await;

        let record = serde_json::json!({ "player": session.player, "loadout": selected });
        match &self.image_service {
            Some(images) => match images.render_loadout(&record).await {
                Ok(png) => {
                    let (filename, description) =
                        loadout_attachment_metadata(&session.player, selected);
                    self.send_webhook_image(
                        "",
                        png,
                        &filename,
                        Some(&description),
                        &interaction.token,
                    )
                    .await
                }
                Err(error) => {
                    tracing::warn!(%error, "loadout render failed");
                    self.send_webhook(
                        &embeds::simple_embed(
                            "Loadout",
                            "The loadout image could not be rendered.",
                            None,
                        ),
                        &[],
                        &interaction.token,
                    )
                    .await;
                }
            },
            None => {
                self.send_webhook(
                    &embeds::simple_embed("Loadout", "The loadout renderer is unavailable.", None),
                    &[],
                    &interaction.token,
                )
                .await
            }
        }
    }

    // ——— Command implementations ———

    async fn help(&self, interaction: &Interaction) {
        let description = [
            "`/save` remember your default Paladins player",
            "`/forget` delete one or all saved player links",
            "`/privacy` stored-data and deletion details",
            "`/profile` profile, rank, record and performance",
            "`/match [id]` match image by ID or your saved player's latest",
            "`/history` recent matches",
            "`/current` current live match",
            "`/champions` per-player champion statistics",
            "`/leaderboard` class, champion and performance rankings",
            "`/activity` observed player activity",
            "`/status` Paladins service status",
            "`/loadout` choose and render a saved champion deck",
            "`/champion` database-backed ranked statistics by lobby tier",
            "`/maps` statistics for every ranked map",
            "`/composition` five most-played ranked team compositions",
            "`/items` ranked item usage and win rate by lobby tier",
            "",
            "Player options are optional after `/save`; use slots for alternate accounts.",
        ]
        .join("\n");
        let embed = embeds::simple_embed("PaladinsCat commands", &description, None);
        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::ChannelMessageWithSource,
                data: Some(InteractionResponseData {
                    embeds: Some(vec![embed]),
                    flags: Some(MessageFlags::EPHEMERAL),
                    ..Default::default()
                }),
            },
        )
        .await;
    }

    async fn privacy(&self, interaction: &Interaction) {
        self.reply_ephemeral_text(
            interaction,
            "PaladinsCat stores only your Discord user ID, the resolved Paladins player ID, the selected slot, and timestamps for `/save`. Public match/profile data comes from PaladinsCat. Use `/forget` to delete one slot or all saved links immediately.",
        )
        .await;
    }

    async fn save(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        if let Some(n) = opt_string(opts, "player") {
            let slot = opt_string(opts, "slot").unwrap_or_else(|| "primary".into());
            match self.api.discord_player(&n).await {
                Ok(resolved) => {
                    let player = resolved.get("player").unwrap_or(&resolved);
                    let Some(player_id) = value_id(player.get("id")) else {
                        self.reply_text(interaction, "Failed to save your default player.")
                            .await;
                        return;
                    };
                    match self
                        .api
                        .save_discord_player(
                            &extract_user_id(interaction).unwrap_or_default(),
                            &player_id,
                            &slot,
                        )
                        .await
                    {
                        Ok(saved) => {
                            let name = saved.get("name").and_then(|v| v.as_str()).unwrap_or(&n);
                            let safe_name: String =
                                embeds::clean_discord_text(&Value::String(name.to_string()), &n)
                                    .chars()
                                    .take(100)
                                    .collect();
                            let id = value_id(saved.get("id")).unwrap_or(player_id);
                            self.reply_text(interaction, format!(
                        "Saved **{}** (ID: `{}`) in `{}`. Player commands will use that slot whenever you omit the player option.", safe_name, id, slot
                    )).await
                        }
                        Err(error) => {
                            self.reply_text(
                                interaction,
                                api_error_message(&error, "Failed to save your default player"),
                            )
                            .await
                        }
                    }
                }
                Err(error) => {
                    self.reply_text(
                        interaction,
                        api_error_message(&error, "Failed to save your default player"),
                    )
                    .await
                }
            }
        }
    }

    async fn forget(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let owner = extract_user_id(interaction).unwrap_or_default();
        let slot = opt_string(opts, "slot").unwrap_or_else(|| "primary".into());
        let embed = embeds::simple_embed(
            "Confirm saved-player deletion",
            &format!("Delete the `{slot}` saved-player link{}? This does not delete public Paladins match data.", if slot == "all" { "s" } else { "" }),
            None,
        );
        let components = vec![Component::ActionRow(ActionRow {
            components: vec![Component::Button(Button {
                custom_id: Some(format!("forget:{owner}:{slot}")),
                disabled: false,
                emoji: None,
                label: Some("Delete saved link".into()),
                style: ButtonStyle::Danger,
                url: None,
                sku_id: None,
            })],
        })];
        self.send_webhook(&embed, &components, &interaction.token)
            .await;
    }

    async fn player_input(
        &self,
        interaction: &Interaction,
        opts: &[CommandDataOption],
    ) -> Result<PlayerInput, String> {
        if let Some(name) = opt_string(opts, "player") {
            return Ok(PlayerInput {
                query: name.trim().to_string(),
                resolved: None,
            });
        }
        let slot = opt_string(opts, "slot").unwrap_or_else(|| "primary".into());
        match self
            .api
            .saved_discord_player(&extract_user_id(interaction).unwrap_or_default(), &slot)
            .await
        {
            Ok(player) => value_id(player.get("id"))
                .map(|query| PlayerInput {
                    query,
                    resolved: Some(player),
                })
                .ok_or_else(missing_saved_player_message),
            Err(error) if error.status == Some(404) => Err(missing_saved_player_message()),
            Err(error) => Err(api_error_message(
                &error,
                "The saved player could not be loaded.",
            )),
        }
    }

    async fn player(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let input = match self.player_input(interaction, opts).await {
            Ok(input) => input,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        match self.api.discord_player(&input.query).await {
            Ok(val) => {
                let embed = embeds::build_player_profile(&val, &self.web_url);
                self.send_embed(interaction, embed).await;
            }
            Err(e) => {
                tracing::error!(player = %input.query, err = %e, "discord_player request failed");
                self.reply_text(
                    interaction,
                    api_error_message(&e, &format!("Failed to look up player '{}'", input.query)),
                )
                .await;
            }
        }
    }

    async fn match_cmd(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let requested_id = opt_string(opts, "id");
        let (id, cooldown_claimed) = match requested_id {
            Some(id) => {
                if !valid_match_id(&id) {
                    return self
                        .reply_text(interaction, "Enter a valid numeric match ID.")
                        .await;
                }
                (id, false)
            }
            None => {
                let Some(user_id) = extract_user_id(interaction) else {
                    return self
                        .reply_text(interaction, missing_saved_match_player_message())
                        .await;
                };
                let player = match self.api.saved_discord_player(&user_id, "primary").await {
                    Ok(player) => player,
                    Err(error) if error.status == Some(404) => {
                        return self
                            .reply_text(interaction, missing_saved_match_player_message())
                            .await;
                    }
                    Err(error) => {
                        return self
                            .reply_text(
                                interaction,
                                api_error_message(&error, "The saved player could not be loaded."),
                            )
                            .await;
                    }
                };
                let Some(player_id) = value_id(player.get("id")) else {
                    return self
                        .reply_text(interaction, missing_saved_match_player_message())
                        .await;
                };
                if let Err(message) = claim_image_cooldown(&user_id) {
                    return self.reply_text(interaction, message).await;
                }
                let latest = match self.api.latest_player_match(&player_id).await {
                    Ok(Some(row)) => row,
                    Ok(None) => {
                        return self
                            .reply_text(
                                interaction,
                                "No recent matches were found for your saved player.",
                            )
                            .await;
                    }
                    Err(error) => {
                        return self
                            .reply_text(
                                interaction,
                                api_error_message(
                                    &error,
                                    "Failed to load your saved player's latest match.",
                                ),
                            )
                            .await;
                    }
                };
                let Some(id) = match_id_from_history_row(&latest) else {
                    return self
                        .reply_text(
                            interaction,
                            "No recent matches were found for your saved player.",
                        )
                        .await;
                };
                (id, true)
            }
        };
        if !cooldown_claimed {
            let Some(user_id) = extract_user_id(interaction) else {
                return self
                    .reply_text(interaction, "A Discord user is required to render images.")
                    .await;
            };
            if let Err(message) = claim_image_cooldown(&user_id) {
                return self.reply_text(interaction, message).await;
            }
        }
        let url = format!("{}/matches/{}", self.web_url, id);
        if let Some(images) = &self.image_service {
            if let Some(png) = images.cached_match(&id).await {
                let filename = format!("paladinscat-match-{}.png", id);
                let description = format!("Paladins match {}", id);
                self.send_webhook_image(
                    &url,
                    png,
                    &filename,
                    Some(&description),
                    &interaction.token,
                )
                .await;
                return;
            }
        }
        match self.api.match_info(&id).await {
            Ok(val) => {
                let match_data = val.get("match").unwrap_or(&val);
                let mode = match_data
                    .get("queue_id")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "Unknown".into());
                let map = match_data
                    .get("map")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "Unknown".into());
                let duration = match_data
                    .get("duration_seconds")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".into());
                let description = format!(
                    "**{}** · {}\nDuration: {}\n[View match]({})",
                    mode, map, duration, url
                );
                let embed =
                    embeds::simple_embed(&format!("Match {}", id), &description, Some(&url));

                if let Some(png) = self.render_match_scoreboard(&val).await {
                    let filename = format!("paladinscat-match-{}.png", id);
                    let description = format!("Paladins match {}", id);
                    self.send_webhook_image(
                        &url,
                        png,
                        &filename,
                        Some(&description),
                        &interaction.token,
                    )
                    .await;
                    return;
                }

                // Fallback: edit the deferred response with the embed via webhook.
                self.send_webhook(&embed, &[], &interaction.token).await;
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, &format!("Match '{}' not found", id)),
                )
                .await;
            }
        }
    }

    async fn history(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let input = match self.player_input(interaction, opts).await {
            Ok(input) => input,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        let resolved = match input.resolved {
            Some(player) => Ok(player),
            None => self.api.resolve_player(&input.query).await,
        };
        match resolved {
            Ok(player) => {
                let id = value_id(player.get("id")).unwrap_or_default();
                let canonical_name = player
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&input.query)
                    .to_string();
                let page = opt_integer(opts, "page").unwrap_or(1).clamp(1, 100) as usize - 1;
                let champion_id = match opt_string(opts, "champion") {
                    Some(name) => self.api.champion_id(&name).await.ok().flatten(),
                    None => None,
                };
                let filters = HistoryFilters {
                    queue_id: opt_string(opts, "queue"),
                    champion_id,
                    win_status: opt_string(opts, "result"),
                    offset: page * 10,
                };
                match self.api.player_history(&id, 11, &filters).await {
                    Ok(mut rows) => {
                        let has_next = rows.len() > 10;
                        rows.truncate(10);
                        let embed =
                            embeds::build_history_payload(&canonical_name, &rows, &self.web_url);
                        prune_sessions();
                        let token = uuid::Uuid::new_v4().simple().to_string();
                        let token = &token[..12];
                        HISTORY_SESSIONS.write().unwrap().insert(
                            token.into(),
                            HistorySession {
                                user_id: extract_user_id(interaction).unwrap_or_default(),
                                player_id: id,
                                player_name: canonical_name,
                                filters,
                                page,
                                expires_at: chrono::Utc::now().timestamp() as u64
                                    + LOADOUT_SESSION_TTL_SECS,
                            },
                        );
                        self.send_webhook(
                            &embed,
                            &history_components(token, page, has_next, &rows),
                            &interaction.token,
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::error!(player_id = id, err = %e, "player_history request failed");
                        self.reply_text(
                            interaction,
                            api_error_message(&e, "Failed to fetch match history"),
                        )
                        .await;
                    }
                }
            }
            Err(e) => {
                tracing::error!(player = input.query.as_str(), err = %e, "player lookup failed");
                self.reply_text(
                    interaction,
                    api_error_message(&e, &format!("Player '{}' not found", input.query)),
                )
                .await;
            }
        }
    }

    async fn current(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let input = match self.player_input(interaction, opts).await {
            Ok(input) => input,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        match self.api.live_match(&input.query).await {
            Ok(val) => {
                let embed = if opt_boolean(opts, "details").unwrap_or(false) {
                    embeds::build_current_payload_detailed(&val, &self.web_url)
                } else {
                    embeds::build_current_payload(&val, &self.web_url)
                };
                self.send_embed(interaction, embed).await;
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, &format!("Player '{}' not found", input.query)),
                )
                .await;
            }
        }
    }

    /// Handle `/loadout` — session-based select menu matching the TS bot 1:1.
/// refs: none
    async fn loadout(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let input = match self.player_input(interaction, opts).await {
            Ok(input) => input,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        let Some(champion) = opt_string(opts, "champion") else {
            return self
                .reply_text(interaction, "Provide a champion name")
                .await;
        };

        let resolved = match input.resolved {
            Some(player) => Ok(player),
            None => self.api.resolve_player(&input.query).await,
        };
        match resolved {
            Ok(val) => {
                let Some(player_id) = val.get("id") else {
                    let embed = embeds::simple_embed(
                        &format!("{} · {}", input.query, champion),
                        &format!("Player '{}' not found", input.query),
                        None,
                    );
                    self.send_webhook(&embed, &[], &interaction.token).await;
                    return;
                };
                let id = value_id(Some(player_id)).unwrap_or_default();
                let player_name = val
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&input.query);

                match self.api.loadouts_response(&id).await {
                    Ok(cached) => {
                        // The first read is DB-only. Match the TS flow: refresh only
                        // when that cache has no deck for the requested champion.
                        let matches_champion = |rows: &[Value]| {
                            rows.iter()
                                .filter(|lo| {
                                    lo.get("champion_name")
                                        .and_then(|v| v.as_str())
                                        .map(|value| {
                                            normalize_champion(value)
                                                == normalize_champion(&champion)
                                        })
                                        .unwrap_or(false)
                                })
                                .cloned()
                                .collect()
                        };
                        let mut champ_loadouts: Vec<Value> = matches_champion(&cached.loadouts);
                        let mut refreshed = cached.refreshed;
                        let mut refresh_error = cached.refresh_error;
                        if champ_loadouts.is_empty() {
                            match self.api.refresh_loadouts(&id).await {
                                Ok(result) => {
                                    champ_loadouts = matches_champion(&result.loadouts);
                                    refreshed = result.refreshed;
                                    refresh_error = result.refresh_error;
                                }
                                // Preserve the cached result on the backend's refresh guard.
                                Err(error) if error.status == Some(429) => {
                                    refresh_error = Some(error.message)
                                }
                                Err(error) => {
                                    return self
                                        .reply_text(
                                            interaction,
                                            api_error_message(&error, "Failed to refresh loadouts"),
                                        )
                                        .await;
                                }
                            }
                        }

                        let champion_name = champ_loadouts
                            .first()
                            .and_then(|loadout| loadout.get("champion_name"))
                            .and_then(Value::as_str)
                            .unwrap_or(&champion);
                        if champ_loadouts.is_empty() {
                            let embed = embeds::build_no_loadouts_payload(
                                player_name,
                                champion_name,
                                refresh_error.as_deref(),
                            );
                            self.send_webhook(&embed, &[], &interaction.token).await;
                            return;
                        }

                        // Prune expired sessions.
                        prune_sessions();

                        // Create a session token.
                        let token = uuid::Uuid::new_v4().to_string();
                        let now = chrono::Utc::now().timestamp() as u64;

                        // Get the user ID for session binding.
                        let user_id = extract_user_id(interaction).unwrap_or_default();

                        let session = LoadoutSession {
                            user_id,
                            player: val.clone(),
                            loadouts: champ_loadouts.iter().take(25).cloned().collect(),
                            expires_at: now + LOADOUT_SESSION_TTL_SECS,
                        };
                        insert_session(&token, session);

                        // Build select menu options (max 25).
                        let options: Vec<SelectMenuOption> = champ_loadouts
                            .iter()
                            .take(25)
                            .map(loadout_select_option)
                            .collect();

                        let select_menu = SelectMenu {
                            custom_id: format!("loadout:{}", token),
                            disabled: false,
                            kind: SelectMenuType::Text,
                            options: Some(options),
                            placeholder: Some(format!("Choose a {} loadout", champion_name)),
                            max_values: Some(1),
                            min_values: Some(1),
                            channel_types: None,
                            default_values: None,
                        };

                        let components = vec![Component::ActionRow(ActionRow {
                            components: vec![Component::SelectMenu(select_menu)],
                        })];

                        let embed = embeds::build_loadout_selection_payload(
                            player_name,
                            champion_name,
                            champ_loadouts.len(),
                            &self.web_url,
                            &id,
                            refreshed,
                        );

                        self.send_webhook(&embed, &components, &interaction.token)
                            .await;
                    }
                    Err(error) => {
                        self.reply_text(
                            interaction,
                            api_error_message(&error, "Failed to fetch loadouts"),
                        )
                        .await;
                    }
                }
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, &format!("Player '{}' not found", input.query)),
                )
                .await;
            }
        }
    }

    async fn champion(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let c: String = opt_string(opts, "champion").unwrap_or_else(|| "any".to_string());
        let scope: String = opt_string(opts, "lobby").unwrap_or_else(|| "global".to_string());
        let lobby_label = match scope.as_str() {
            "bronze-gold" => "Bronze–Gold lobbies",
            "platinum" => "Platinum+ lobbies",
            "diamond" => "Diamond+ lobbies",
            _ => "Global ranked lobbies",
        };
        match self.api.champion_page_data(&c.to_lowercase(), &scope).await {
            Ok(val) => {
                let embed = embeds::build_champion_payload(&val, &self.web_url, lobby_label);
                self.send_embed(interaction, embed).await;
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, &format!("No data for champion '{}'", c)),
                )
                .await;
            }
        }
    }

    async fn stats(&self, interaction: &Interaction, command: &str, opts: &[CommandDataOption]) {
        match command {
            "maps" => match self.api.ranked_maps(100).await {
                Ok(rows) => {
                    let embed = embeds::build_maps_payload(&rows, &self.web_url);
                    self.send_embed(interaction, embed).await;
                }
                Err(error) => {
                    self.reply_text(
                        interaction,
                        api_error_message(&error, "Failed to fetch map stats"),
                    )
                    .await;
                }
            },
            "composition" => match self.api.ranked_compositions(5).await {
                Ok(rows) => {
                    let embed = embeds::build_composition_payload(&rows, &self.web_url);
                    self.send_embed(interaction, embed).await;
                }
                Err(error) => {
                    self.reply_text(
                        interaction,
                        api_error_message(&error, "Failed to fetch composition stats"),
                    )
                    .await;
                }
            },
            "items" => {
                let scope = opt_string(opts, "lobby").unwrap_or_else(|| "global".to_string());
                let lobby_label = match scope.as_str() {
                    "bronze-gold" => "Bronze–Gold lobbies",
                    "platinum" => "Platinum+ lobbies",
                    "diamond" => "Diamond+ lobbies",
                    _ => "Global ranked lobbies",
                };
                match self.api.ranked_items(&scope, 20).await {
                    Ok(rows) => {
                        let embed = embeds::build_items_payload(&rows, &self.web_url, lobby_label);
                        self.send_embed(interaction, embed).await;
                    }
                    Err(error) => {
                        self.reply_text(
                            interaction,
                            api_error_message(&error, "Failed to fetch item stats"),
                        )
                        .await;
                    }
                }
            }
            _ => {
                self.reply_text(interaction, format!("{} stats coming soon", command))
                    .await;
            }
        }
    }

    async fn player_champions(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let input = match self.player_input(interaction, opts).await {
            Ok(input) => input,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        let player = match input.resolved {
            Some(player) => Ok(player),
            None => self.api.resolve_player(&input.query).await,
        };
        let player = match player {
            Ok(player) => player,
            Err(error) => {
                return self
                    .reply_text(interaction, api_error_message(&error, "Player not found"))
                    .await
            }
        };
        let id = value_id(player.get("id")).unwrap_or_default();
        let name = player
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&input.query);
        match self.api.player_champions(&id).await {
            Ok(mut rows) => {
                if let Some(role) = opt_string(opts, "role") {
                    rows.retain(|row| {
                        row.get("role").and_then(Value::as_str) == Some(role.as_str())
                    });
                }
                match opt_string(opts, "sort").as_deref().unwrap_or("matches") {
                    "winrate" => rows.sort_by(|a, b| {
                        numeric_value(b.get("win_rate"))
                            .total_cmp(&numeric_value(a.get("win_rate")))
                    }),
                    "kda" => rows.sort_by(|a, b| champion_kda(b).total_cmp(&champion_kda(a))),
                    _ => rows.sort_by(|a, b| {
                        numeric_value(b.get("matches_played"))
                            .total_cmp(&numeric_value(a.get("matches_played")))
                    }),
                }
                let lines = rows
                    .iter()
                    .filter(|row| numeric_value(row.get("matches_played")) > 0.0)
                    .take(10)
                    .map(|row| {
                        format!(
                            "**{}** · {} matches · {:.1}% WR · {:.2} KDA",
                            clean_value(row.get("champion_name"), "Champion"),
                            numeric_value(row.get("matches_played")) as i64,
                            numeric_value(row.get("win_rate")),
                            champion_kda(row)
                        )
                    })
                    .collect::<Vec<_>>();
                let description = if lines.is_empty() {
                    "No champion matches found.".into()
                } else {
                    lines.join("\n")
                };
                self.send_embed(
                    interaction,
                    embeds::simple_embed(&format!("{name} · Champions"), &description, None),
                )
                .await;
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, "Failed to fetch champion statistics"),
                )
                .await
            }
        }
    }

    async fn leaderboard(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let category = opt_string(opts, "category").unwrap_or_else(|| "performance".into());
        let metric = opt_string(opts, "metric")
            .or_else(|| (category == "performance").then(|| "dpm".into()));
        let role = opt_string(opts, "role")
            .or_else(|| (category != "performance").then(|| "Damage".into()));
        let champion_id = match opt_string(opts, "champion") {
            Some(name) => self.api.champion_id(&name).await.ok().flatten(),
            None => None,
        };
        match self
            .api
            .leaderboard(
                &category,
                metric.as_deref(),
                role.as_deref(),
                champion_id.as_deref(),
            )
            .await
        {
            Ok(value) => {
                let rows = value
                    .get("data")
                    .and_then(Value::as_array)
                    .or_else(|| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                let description = leaderboard_description(&rows, metric.as_deref());
                self.send_embed(
                    interaction,
                    embeds::simple_embed("PaladinsCat leaderboard", &description, None),
                )
                .await;
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, "Failed to fetch leaderboard"),
                )
                .await
            }
        }
    }

    async fn activity(&self, interaction: &Interaction) {
        match self.api.activity().await {
            Ok(value) => {
                let presence = value.get("presence").unwrap_or(&value);
                let lines = [
                    ("Public players", "public_players"),
                    ("Private players", "private_players"),
                    ("Public lower bound", "public_players_lower_bound"),
                    ("Public upper bound", "public_players_upper_bound"),
                    ("Unresolved matches", "unresolved_matches"),
                ]
                .iter()
                .filter_map(|(label, key)| {
                    presence
                        .get(*key)
                        .map(|v| format!("**{label}:** {}", clean_value(Some(v), "0")))
                })
                .chain(
                    value
                        .get("overview")
                        .and_then(|v| v.get("hourly"))
                        .and_then(|v| v.get("allQueuesTotal24h"))
                        .map(|v| format!("**Observed matches:** {}", clean_value(Some(v), "0"))),
                )
                .collect::<Vec<_>>();
                self.send_embed(
                    interaction,
                    embeds::embed_with_footer(
                        "Player activity · last 24 hours",
                        &lines.join("\n"),
                        "Observed match evidence; counts are bounded where identity is unresolved.",
                        0x4fd1c5,
                    ),
                )
                .await;
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, "Player activity is temporarily unavailable"),
                )
                .await
            }
        }
    }

    async fn status(&self, interaction: &Interaction) {
        match self.api.status().await {
            Ok(value) => {
                self.send_embed(
                    interaction,
                    embeds::simple_embed(
                        &format!(
                            "Paladins service status · {}",
                            clean_value(value.get("status"), "unknown")
                        ),
                        &clean_value(value.get("message"), "No service message available."),
                        None,
                    ),
                )
                .await
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, "Service status is temporarily unavailable"),
                )
                .await
            }
        }
    }

    async fn random(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let kind = opt_string(opts, "kind").unwrap_or_else(|| "champion".into());
        let role = opt_string(opts, "role");
        let result = if kind == "map" {
            self.api.ranked_maps(50).await.map(|rows| {
                rows.get(pseudo_index(rows.len()))
                    .map(|row| {
                        clean_value(row.get("map").or_else(|| row.get("name")), "Unknown map")
                    })
                    .unwrap_or_else(|| "No maps available".into())
            })
        } else {
            self.api.champions().await.map(|value| {
                let mut rows = value.as_array().cloned().unwrap_or_default();
                if let Some(role) = role.as_deref() {
                    rows.retain(|row| {
                        champion_role(row).is_some_and(|value| value.eq_ignore_ascii_case(role))
                    });
                }
                if kind == "team" {
                    ["Frontline", "Damage", "Flank", "Support", "Any"]
                        .iter()
                        .enumerate()
                        .filter_map(|(i, wanted)| {
                            let choices = rows
                                .iter()
                                .filter(|row| {
                                    *wanted == "Any"
                                        || champion_role(row)
                                            .is_some_and(|value| value.eq_ignore_ascii_case(wanted))
                                })
                                .collect::<Vec<_>>();
                            (!choices.is_empty()).then(|| {
                                clean_value(
                                    choices[(pseudo_index(choices.len()) + i) % choices.len()]
                                        .get("name"),
                                    "Champion",
                                )
                            })
                        })
                        .collect::<Vec<_>>()
                        .join(" · ")
                } else {
                    rows.get(pseudo_index(rows.len()))
                        .map(|row| clean_value(row.get("name"), "Champion"))
                        .unwrap_or_else(|| "No champions available".into())
                }
            })
        };
        match result {
            Ok(value) => {
                self.reply_text(interaction, format!("🎲 **{value}**"))
                    .await
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, "Random selection is temporarily unavailable"),
                )
                .await
            }
        }
    }

    async fn teams(&self, interaction: &Interaction) {
        let (Some(guild_id), Some(user_id)) = (
            interaction.guild_id,
            extract_user_id(interaction).and_then(|id| id.parse::<u64>().ok()),
        ) else {
            return self
                .reply_text(interaction, "Use `/teams` from a server voice channel.")
                .await;
        };
        let Some(state) = self
            .discord_cache
            .voice_state(twilight_model::id::Id::new(user_id), guild_id)
        else {
            return self
                .reply_text(interaction, "Join a voice channel before using `/teams`.")
                .await;
        };
        let channel_id = state.channel_id();
        drop(state);
        let mut users = self
            .discord_cache
            .voice_channel_states(channel_id)
            .map(|states| {
                states
                    .map(|state| state.user_id().get())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if users.len() < 2 {
            return self
                .reply_text(
                    interaction,
                    "At least two people must be in your voice channel.",
                )
                .await;
        }
        users.sort_unstable_by_key(|id| id.rotate_left((pseudo_index(63) + 1) as u32));
        let midpoint = users.len().div_ceil(2);
        let format_team = |team: &[u64]| {
            team.iter()
                .map(|id| format!("<@{id}>"))
                .collect::<Vec<_>>()
                .join(" · ")
        };
        self.reply_text(
            interaction,
            format!(
                "**Team 1**\n{}\n\n**Team 2**\n{}",
                format_team(&users[..midpoint]),
                format_team(&users[midpoint..])
            ),
        )
        .await;
    }

    async fn user_context(&self, interaction: &Interaction, command: &CommandData) {
        let Some(target) = command.target_id.map(|id| id.get().to_string()) else {
            return self
                .reply_text(interaction, "No Discord user was selected.")
                .await;
        };
        let player = match self.api.saved_discord_player(&target, "primary").await {
            Ok(player) => player,
            Err(error) if error.status == Some(404) => {
                return self
                    .reply_text(
                        interaction,
                        "That Discord user has not saved a primary Paladins player.",
                    )
                    .await
            }
            Err(error) => {
                return self
                    .reply_text(
                        interaction,
                        api_error_message(&error, "The saved player could not be loaded"),
                    )
                    .await
            }
        };
        let id = value_id(player.get("id")).unwrap_or_default();
        let name = player
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Player");
        match command.name.as_str() {
            "Paladins Profile" => match self.api.discord_player(&id).await {
                Ok(value) => {
                    self.send_embed(
                        interaction,
                        embeds::build_player_profile(&value, &self.web_url),
                    )
                    .await
                }
                Err(error) => {
                    self.reply_text(
                        interaction,
                        api_error_message(&error, "Failed to fetch profile"),
                    )
                    .await
                }
            },
            "Paladins History" => match self
                .api
                .player_history(&id, 10, &HistoryFilters::default())
                .await
            {
                Ok(rows) => {
                    self.send_embed(
                        interaction,
                        embeds::build_history_payload(name, &rows, &self.web_url),
                    )
                    .await
                }
                Err(error) => {
                    self.reply_text(
                        interaction,
                        api_error_message(&error, "Failed to fetch history"),
                    )
                    .await
                }
            },
            _ => match self.api.live_match(&id).await {
                Ok(value) => {
                    self.send_embed(
                        interaction,
                        embeds::build_current_payload(&value, &self.web_url),
                    )
                    .await
                }
                Err(error) => {
                    self.reply_text(
                        interaction,
                        api_error_message(&error, "Failed to fetch live match"),
                    )
                    .await
                }
            },
        }
    }

    // ——— Helpers ———

    async fn send_embed(&self, interaction: &Interaction, embed: Embed) {
        self.send_webhook(&embed, &[], &interaction.token).await;
    }

    async fn send_response(
        &self,
        interaction_id: twilight_model::id::Id<twilight_model::id::marker::InteractionMarker>,
        token: &str,
        resp: InteractionResponse,
    ) {
        if let Err(error) = self
            .http
            .interaction(self.app_id)
            .create_response(interaction_id, token, &resp)
            .await
        {
            tracing::error!(%error, interaction_id = %interaction_id, "interaction response failed");
        }
    }

    async fn reply_text(&self, interaction: &Interaction, msg: impl Into<String>) {
        self.send_webhook_text(&interaction.token, msg.into()).await;
    }

    async fn reply_ephemeral_text(&self, interaction: &Interaction, msg: impl Into<String>) {
        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::ChannelMessageWithSource,
                data: Some(InteractionResponseData {
                    content: Some(msg.into()),
                    flags: Some(MessageFlags::EPHEMERAL),
                    ..Default::default()
                }),
            },
        )
        .await;
    }

    /// Defer the initial interaction response inside Discord's 3-second ACK window.
/// refs: none
    async fn defer_response(&self, interaction: &Interaction, ephemeral: bool) {
        let data = ephemeral.then(|| InteractionResponseData {
            flags: Some(MessageFlags::EPHEMERAL),
            ..Default::default()
        });
        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::DeferredChannelMessageWithSource,
                data,
            },
        )
        .await;
    }

    async fn defer_update(&self, interaction: &Interaction) {
        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::DeferredUpdateMessage,
                data: None,
            },
        )
        .await;
    }

    fn original_response_url(&self, token: &str) -> String {
        format!(
            "https://discord.com/api/v10/webhooks/{}/{}/messages/@original",
            self.app_id.get(),
            token
        )
    }

    /// Edit the original deferred interaction response.
/// refs: none
    async fn send_webhook(&self, embed: &Embed, components: &[Component], token: &str) {
        let url = self.original_response_url(token);
        let payload = serde_json::json!({
            "content": "",
            "embeds": [embed],
            "components": components,
            "attachments": [],
        });
        match WEBHOOK_HTTP.patch(&url).json(&payload).send().await {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                tracing::error!(url = %url, status = %response.status(), "webhook PATCH rejected")
            }
            Err(e) => {
                tracing::error!(url = %url, err = %e, "webhook PATCH failed");
            }
        }
    }

    async fn send_webhook_text(&self, token: &str, content: String) {
        let url = self.original_response_url(token);
        let payload = serde_json::json!({
            "content": content,
            "embeds": [],
            "components": [],
            "attachments": [],
            "allowed_mentions": { "parse": [] },
        });
        match WEBHOOK_HTTP.patch(&url).json(&payload).send().await {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                tracing::error!(url = %url, status = %response.status(), "webhook text PATCH rejected")
            }
            Err(error) => tracing::error!(url = %url, %error, "webhook text PATCH failed"),
        }
    }

    async fn send_webhook_image(
        &self,
        content: &str,
        png: Vec<u8>,
        filename: &str,
        description: Option<&str>,
        token: &str,
    ) {
        self.send_webhook_image_with_components(content, png, filename, description, &[], token)
            .await;
    }

    async fn send_webhook_image_with_components(
        &self,
        content: &str,
        png: Vec<u8>,
        filename: &str,
        description: Option<&str>,
        components: &[Component],
        token: &str,
    ) {
        let url = self.original_response_url(token);
        let payload = webhook_image_payload(content, filename, description, components);
        let body = reqwest::multipart::Form::new()
            .text(
                "payload_json",
                serde_json::to_string(&payload).unwrap_or_default(),
            )
            .part(
                "files[0]",
                reqwest::multipart::Part::bytes(png).file_name(filename.to_string()),
            );
        match WEBHOOK_HTTP.patch(&url).multipart(body).send().await {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                tracing::error!(url = %url, status = %response.status(), "webhook image PATCH rejected")
            }
            Err(error) => tracing::error!(url = %url, %error, "webhook image PATCH failed"),
        }
    }

    async fn render_match_scoreboard(&self, value: &Value) -> Option<Vec<u8>> {
        let images = Arc::clone(self.image_service.as_ref()?);
        let renderer = Arc::clone(&images);
        let value = value.clone();
        let render = async move { renderer.render_match(&value).await };
        match tokio::time::timeout(RENDER_TIMEOUT, render).await {
            Ok(Ok(png)) => Some(png),
            Ok(Err(error)) => {
                tracing::warn!(%error, "match image render failed — falling back to embed");
                None
            }
            Err(_) => {
                tracing::warn!("match image render timed out — falling back to embed");
                // `timeout` drops the render future before its recovery wrapper
                // sees an error. Reset the shared browser so the next render is
                // never queued behind a poisoned CDP page.
                images.recycle().await;
                None
            }
        }
    }
}

fn webhook_image_payload(
    content: &str,
    filename: &str,
    description: Option<&str>,
    components: &[Component],
) -> Value {
    serde_json::json!({
        "content": content,
        "embeds": [],
        "components": components,
        "attachments": [{ "id": 0, "filename": filename, "description": description }],
        "allowed_mentions": { "parse": [] },
    })
}

fn extract_command_data(data: &Option<InteractionData>) -> Option<&CommandData> {
    let data = data.as_ref()?;
    match data {
        InteractionData::ApplicationCommand(cmd) => Some(cmd),
        InteractionData::MessageComponent(_) => None,
        InteractionData::ModalSubmit(_) => None,
        _ => None,
    }
}

fn opt_string(opts: &[CommandDataOption], name: &str) -> Option<String> {
    opts.iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            CommandOptionValue::String(s) => Some(s.clone()),
            _ => None,
        })
}

fn opt_integer(opts: &[CommandDataOption], name: &str) -> Option<i64> {
    opts.iter()
        .find(|option| option.name == name)
        .and_then(|option| match option.value {
            CommandOptionValue::Integer(value) => Some(value),
            _ => None,
        })
}

fn opt_boolean(opts: &[CommandDataOption], name: &str) -> Option<bool> {
    opts.iter()
        .find(|option| option.name == name)
        .and_then(|option| match option.value {
            CommandOptionValue::Boolean(value) => Some(value),
            _ => None,
        })
}

fn numeric_value(value: Option<&Value>) -> f64 {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
        })
        .unwrap_or_default()
}

fn clean_value(value: Option<&Value>, fallback: &str) -> String {
    embeds::clean_discord_text(value.unwrap_or(&Value::Null), fallback)
}

fn champion_kda(row: &Value) -> f64 {
    let deaths = numeric_value(row.get("deaths"));
    if deaths > 0.0 {
        (numeric_value(row.get("kills")) + numeric_value(row.get("assists")) * 0.5) / deaths
    } else {
        0.0
    }
}

fn champion_role(row: &Value) -> Option<&str> {
    ["role", "roles", "class_name", "class"]
        .iter()
        .find_map(|key| row.get(*key).and_then(Value::as_str))
}

fn leaderboard_description(rows: &[Value], metric: Option<&str>) -> String {
    let lines = rows
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, row)| {
            let rank = numeric_value(row.get("rank")) as usize;
            let rank = if rank > 0 { rank } else { index + 1 };
            let player = clean_value(row.get("player_name"), "Player");
            let detail = row
                .get("champion_name")
                .and_then(Value::as_str)
                .or_else(|| row.get("class_name").and_then(Value::as_str))
                .unwrap_or("");
            let score = ["value", "elo", "mu", metric.unwrap_or("dpm")]
                .iter()
                .find_map(|key| row.get(*key))
                .map(|value| numeric_value(Some(value)))
                .unwrap_or_default();
            format!(
                "`#{rank}` **{player}**{} · {:.0}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(" · {detail}")
                },
                score
            )
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "No leaderboard rows found.".into()
    } else {
        lines.join("\n")
    }
}

fn pseudo_index(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default()
        .unsigned_abs() as usize
        % len
}

fn button(custom_id: String, label: &str, style: ButtonStyle, disabled: bool) -> Component {
    Component::Button(Button {
        custom_id: Some(custom_id),
        disabled,
        emoji: None,
        label: Some(label.into()),
        style,
        url: None,
        sku_id: None,
    })
}

fn history_components(token: &str, page: usize, has_next: bool, rows: &[Value]) -> Vec<Component> {
    let mut components = vec![Component::ActionRow(ActionRow {
        components: vec![
            button(
                format!("history:{token}:prev"),
                "Previous",
                ButtonStyle::Secondary,
                page == 0,
            ),
            button(
                format!("history:{token}:next"),
                "Next",
                ButtonStyle::Primary,
                !has_next,
            ),
        ],
    })];
    let options = rows
        .iter()
        .filter_map(|row| {
            let match_id = value_id(row.get("match_id"))?;
            Some(SelectMenuOption {
                default: false,
                description: Some(
                    format!(
                        "{} · {}/{}/{}",
                        clean_value(row.get("map"), "Unknown map"),
                        numeric_value(row.get("kills")) as i64,
                        numeric_value(row.get("deaths")) as i64,
                        numeric_value(row.get("assists")) as i64
                    )
                    .chars()
                    .take(100)
                    .collect(),
                ),
                emoji: None,
                label: clean_value(row.get("champion_name"), "Match")
                    .chars()
                    .take(100)
                    .collect(),
                value: match_id,
            })
        })
        .collect::<Vec<_>>();
    if !options.is_empty() {
        components.push(Component::ActionRow(ActionRow {
            components: vec![Component::SelectMenu(SelectMenu {
                custom_id: format!("history-match:{token}"),
                disabled: false,
                kind: SelectMenuType::Text,
                options: Some(options),
                placeholder: Some("Select a match for details".into()),
                max_values: Some(1),
                min_values: Some(1),
                channel_types: None,
                default_values: None,
            })],
        }));
    }
    components
}

fn history_back_component(token: &str) -> Vec<Component> {
    vec![Component::ActionRow(ActionRow {
        components: vec![button(
            format!("history:{token}:stay"),
            "Back to history",
            ButtonStyle::Secondary,
            false,
        )],
    })]
}

fn history_match_loading_response(match_id: &str) -> InteractionResponse {
    let embed = embeds::simple_embed(
        &format!("Loading match {match_id}…"),
        "Fetching the complete match details.",
        None,
    );
    InteractionResponse {
        kind: InteractionResponseType::UpdateMessage,
        data: Some(InteractionResponseData {
            content: Some(String::new()),
            embeds: Some(vec![embed]),
            components: Some(Vec::new()),
            ..Default::default()
        }),
    }
}

fn value_id(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(id) => Some(id.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn normalize_champion(value: &str) -> String {
    value
        .nfkd()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

fn loadout_attachment_metadata(player: &Value, loadout: &Value) -> (String, String) {
    let champion = loadout
        .get("champion_name")
        .and_then(|v| v.as_str())
        .unwrap_or("champion");
    let loadout_id = value_id(loadout.get("id")).unwrap_or_else(|| "unknown".to_string());
    let filename = format!(
        "paladinscat-loadout-{}-{}.png",
        normalize_champion(champion),
        loadout_id
    );
    let player_name = player
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Player");
    let loadout_name = loadout
        .get("loadout_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Loadout");
    (
        filename,
        format!("{}'s {} loadout {}", player_name, champion, loadout_name),
    )
}

fn loadout_select_option(loadout: &Value) -> SelectMenuOption {
    let label = loadout
        .get("loadout_name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("Unnamed Loadout")
        .chars()
        .take(100)
        .collect::<String>();
    let card_points: i64 = loadout
        .get("card_levels")
        .and_then(Value::as_array)
        .map(|levels| {
            levels
                .iter()
                .filter_map(|value| {
                    value
                        .as_i64()
                        .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
                })
                .sum()
        })
        .unwrap_or(0);
    SelectMenuOption {
        default: false,
        description: Some(format!("{} card points", card_points)),
        emoji: None,
        label,
        value: value_id(loadout.get("id")).unwrap_or_default(),
    }
}

fn api_error_message(error: &ApiError, fallback: &str) -> String {
    match error.status {
        Some(429) => "PaladinsCat is busy. Try again in a few seconds.".into(),
        Some(502..=504) => "Paladins data is temporarily unavailable. Try again shortly.".into(),
        _ if error.message == "The PaladinsCat service request failed." => {
            error.code.as_deref().map_or_else(
                || fallback.to_owned(),
                |code| format!("{fallback} (`{code}`)"),
            )
        }
        _ => error.message.clone(),
    }
}

fn missing_saved_player_message() -> String {
    "No player name was entered and you do not have a saved player. Enter a player or use `/save player:<name or ID>` first.".to_string()
}

fn missing_saved_match_player_message() -> String {
    "No match ID was entered and you do not have a saved primary player. Use `/save player:<name or ID>` first.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn match_id_validation_matches_legacy_command() {
        assert!(valid_match_id("1281335238"));
        assert!(!valid_match_id("12345"));
        assert!(!valid_match_id("12813x5238"));
    }

    #[test]
    fn latest_history_match_id_accepts_backend_strings_and_numbers() {
        assert_eq!(
            match_id_from_history_row(&serde_json::json!({"match_id":"1281335238"})),
            Some("1281335238".into())
        );
        assert_eq!(
            match_id_from_history_row(&serde_json::json!({"match_id":1281335238_u64})),
            Some("1281335238".into())
        );
        assert_eq!(
            match_id_from_history_row(&serde_json::json!({"match_id":"invalid"})),
            None
        );
    }

    #[test]
    fn image_cooldown_rejects_an_immediate_duplicate() {
        let user = format!("cooldown-test-{}", uuid::Uuid::new_v4());
        assert!(claim_image_cooldown(&user).is_ok());
        assert!(claim_image_cooldown(&user)
            .unwrap_err()
            .starts_with("Image cooldown: try again in "));
    }

    #[test]
    fn command_rate_window_enforces_capacity() {
        let store = RwLock::new(HashMap::new());
        assert!(claim_window(
            &store,
            "user".into(),
            2,
            Duration::from_secs(10)
        ));
        assert!(claim_window(
            &store,
            "user".into(),
            2,
            Duration::from_secs(10)
        ));
        assert!(!claim_window(
            &store,
            "user".into(),
            2,
            Duration::from_secs(10)
        ));
    }

    #[test]
    fn history_controls_include_paging_and_match_selection() {
        let components = history_components(
            "token",
            0,
            true,
            &[serde_json::json!({
                "match_id":"1281335238","champion_name":"Ying","map":"Bazaar",
                "kills":1,"deaths":2,"assists":3
            })],
        );
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn history_match_selection_immediately_updates_the_message() {
        let payload = serde_json::to_value(history_match_loading_response("1281335238")).unwrap();
        assert_eq!(payload["type"], 7);
        assert_eq!(
            payload["data"]["embeds"][0]["title"],
            "Loading match 1281335238…"
        );
        assert_eq!(payload["data"]["components"], serde_json::json!([]));
    }

    #[test]
    fn history_match_scoreboard_keeps_back_control() {
        let components = history_back_component("session-token");
        let payload = webhook_image_payload(
            "https://paladinscat.com/matches/1281335238",
            "paladinscat-match-1281335238.png",
            Some("Paladins match 1281335238"),
            &components,
        );

        assert_eq!(
            payload["attachments"][0]["filename"],
            "paladinscat-match-1281335238.png"
        );
        assert_eq!(
            payload["components"][0]["components"][0]["custom_id"],
            "history:session-token:stay"
        );
        assert_eq!(
            payload["components"][0]["components"][0]["label"],
            "Back to history"
        );
    }

    #[test]
    fn leaderboard_description_shows_at_most_ten_rows() {
        let rows = (1..=12)
            .map(|rank| {
                serde_json::json!({
                    "rank": rank,
                    "player_name": format!("Player {rank}"),
                    "champion_name": "Ying",
                    "dpm": rank * 100
                })
            })
            .collect::<Vec<_>>();
        let description = leaderboard_description(&rows, Some("dpm"));
        assert_eq!(description.lines().count(), 10);
        assert!(description.contains("Player 10"));
        assert!(!description.contains("Player 11"));
    }

    #[test]
    fn live_champion_catalog_roles_drive_social_filters() {
        assert_eq!(
            champion_role(&serde_json::json!({"roles":"Flank"})),
            Some("Flank")
        );
    }

    #[test]
    fn loadout_attachment_metadata_matches_legacy_format() {
        let (filename, description) = loadout_attachment_metadata(
            &serde_json::json!({ "name": "Nabi" }),
            &serde_json::json!({ "id": 42, "champion_name": "Mal'Damba", "loadout_name": "Snake Pit" }),
        );
        assert_eq!(filename, "paladinscat-loadout-maldamba-42.png");
        assert_eq!(description, "Nabi's Mal'Damba loadout Snake Pit");
    }

    #[test]
    fn loadout_menu_keeps_plain_labels_ids_and_card_totals() {
        let option = loadout_select_option(&serde_json::json!({
            "id": "deck-1",
            "loadout_name": "Speed Build",
            "card_levels": [5, "4", 3, 2, 1]
        }));
        assert_eq!(option.label, "Speed Build");
        assert_eq!(option.value, "deck-1");
        assert_eq!(option.description.as_deref(), Some("15 card points"));
    }

    #[test]
    fn registration_is_claimed_once_and_skips_dummy_mode() {
        let started = AtomicBool::new(false);
        let app_id = twilight_model::id::Id::new(42);
        assert_eq!(claim_registration(&started, Some(app_id)), Some(app_id));
        assert_eq!(claim_registration(&started, Some(app_id)), None);
        assert_eq!(claim_registration(&AtomicBool::new(false), None), None);
    }
}
