//! Slash command handlers — dispatches InteractionCreate events.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

use serde_json::Value;
use twilight_gateway::Event;
use twilight_http::Client as HttpClient;
use twilight_model::application::interaction::{
    application_command::{CommandData, CommandDataOption, CommandOptionValue},
    Interaction, InteractionData, InteractionType,
};
use twilight_model::channel::message::component::{
    ActionRow, Component, SelectMenu, SelectMenuOption, SelectMenuType,
};
use twilight_model::channel::message::embed::Embed;
use twilight_model::channel::message::MessageFlags;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use unicode_normalization::UnicodeNormalization;

use crate::api::{ApiClient, ApiError};
use crate::cache::RenderCache;
use crate::embeds;
use crate::image::ImageService;

/// Loadout session — maps a token to player/loadout data with an expiration.
#[derive(Clone)]
struct LoadoutSession {
    user_id: String,
    player: Value,
    loadouts: Vec<Value>,
    expires_at: u64,
}

/// 5-minute TTL for loadout sessions.
const LOADOUT_SESSION_TTL_SECS: u64 = 5 * 60;
const IMAGE_COOLDOWN_MS: i64 = 10 * 1000;

/// Maximum time to wait for an image render before falling back to an embed.
/// The interaction is deferred first, so this bounds only how long the user
/// waits for the image (or the embed fallback), not Discord's 3s ACK window.
const RENDER_TIMEOUT: Duration = Duration::from_secs(12);

/// Module-level session store.  Shared between command and component handlers.
static LOADOUT_SESSIONS: LazyLock<RwLock<HashMap<String, LoadoutSession>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));
static IMAGE_COOLDOWNS: LazyLock<RwLock<HashMap<String, i64>>> =
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
}

/// Main event dispatcher — routes gateway events to command handlers.
pub async fn handle_event(
    event: Event,
    api: Arc<ApiClient>,
    render_cache: Arc<RenderCache>,
    http: Arc<HttpClient>,
    web_url: String,
    image_service: Option<Arc<ImageService>>,
) {
    match event {
        Event::Ready(_) => {
            tracing::info!("Gateway connected");
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
                    });
                    tokio::spawn(async move { h.handle_component(interaction).await });
                }
                _ => {}
            }
        }
        _ => {}
    }
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
fn prune_sessions() {
    let now = chrono::Utc::now().timestamp() as u64;
    let mut sessions = LOADOUT_SESSIONS.write().unwrap();
    sessions.retain(|_, s| s.expires_at > now);
}

/// Clone a session out of the store, dropping the lock immediately.
fn get_session(token: &str) -> Option<LoadoutSession> {
    let sessions = LOADOUT_SESSIONS.read().unwrap();
    sessions.get(token).cloned()
}

/// Remove a session by token.
fn remove_session(token: &str) -> bool {
    let mut sessions = LOADOUT_SESSIONS.write().unwrap();
    sessions.remove(token).is_some()
}

/// Insert a session into the store.
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

fn valid_match_id(id: &str) -> bool {
    (6..=20).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit())
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

        // Match the TS lifecycle: acknowledge every potentially slow command
        // before doing any backend lookup. Otherwise Discord invalidates the
        // interaction token while the Rust handler is still waiting on I/O.
        self.defer_response(&interaction, cmd_data.name == "save")
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

        // Only handle loadout:* custom IDs.
        let custom_id = &data.custom_id;
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
            "`/profile` profile, rank, record and performance",
            "`/match` optimized match-result image",
            "`/history` recent matches",
            "`/current` current live match",
            "`/loadout` choose and render a saved champion deck",
            "`/champion` database-backed ranked statistics by lobby tier",
            "`/maps` statistics for every ranked map",
            "`/composition` five most-played ranked team compositions",
            "`/items` ranked item usage and win rate by lobby tier",
            "",
            "Player options are optional after you use `/save`.",
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

    async fn save(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        if let Some(n) = opt_string(opts, "player") {
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
                        )
                        .await
                    {
                        Ok(saved) => {
                            let name = saved.get("name").and_then(|v| v.as_str()).unwrap_or(&n);
                            let id = value_id(saved.get("id")).unwrap_or(player_id);
                            self.reply_text(interaction, format!(
                        "Saved **{}** (ID: `{}`) as your default player. Player commands will use it whenever you omit the player option.", name, id
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

    async fn player_input(
        &self,
        interaction: &Interaction,
        opts: &[CommandDataOption],
    ) -> Result<String, String> {
        if let Some(name) = opt_string(opts, "player") {
            return Ok(name);
        }
        match self
            .api
            .saved_discord_player(&extract_user_id(interaction).unwrap_or_default())
            .await
        {
            Ok(player) => value_id(player.get("id")).ok_or_else(missing_saved_player_message),
            Err(error) if error.status == Some(404) => Err(missing_saved_player_message()),
            Err(error) => Err(api_error_message(
                &error,
                "The saved player could not be loaded.",
            )),
        }
    }

    async fn player(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let name = match self.player_input(interaction, opts).await {
            Ok(name) => name,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        match self.api.discord_player(&name).await {
            Ok(val) => {
                let embed = embeds::build_player_profile(&val, &self.web_url);
                self.send_embed(interaction, embed).await;
            }
            Err(e) => {
                tracing::error!(player = %name, err = %e, "discord_player request failed");
                self.reply_text(
                    interaction,
                    api_error_message(&e, &format!("Failed to look up player '{}'", name)),
                )
                .await;
            }
        }
    }

    async fn match_cmd(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let Some(id) = opt_string(opts, "id") else {
            return self.reply_text(interaction, "Provide a match ID").await;
        };
        if !valid_match_id(&id) {
            return self
                .reply_text(interaction, "Enter a valid numeric match ID.")
                .await;
        }
        if let Some(user_id) = extract_user_id(interaction) {
            if let Err(message) = claim_image_cooldown(&user_id) {
                return self.reply_text(interaction, message).await;
            }
        }
        match self.api.match_info(&id).await {
            Ok(val) => {
                let mode = val
                    .get("mode")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "Unknown".into());
                let map = val
                    .get("map")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "Unknown".into());
                let duration = val
                    .get("duration")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".into());
                let url = format!("{}/matches/{}", self.web_url, id);
                let description = format!(
                    "**{}** · {}\nDuration: {}\n[View match]({})",
                    mode, map, duration, url
                );
                let embed =
                    embeds::simple_embed(&format!("Match {}", id), &description, Some(&url));

                if let Some(img) = &self.image_service {
                    let img = Arc::clone(img);
                    let match_id = id.clone();
                    let match_url = url.clone();
                    let render_url = match_url.clone();
                    let token = interaction.token.clone();
                    let renderer = Arc::clone(&img);
                    let render =
                        async move { renderer.render_web_match(&match_id, &render_url).await };
                    match tokio::time::timeout(RENDER_TIMEOUT, render).await {
                        Ok(Ok(png)) => {
                            self.send_webhook_image(&match_url, png, "match.png", None, &token)
                                .await;
                            return;
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(err = %e, "match image render failed — falling back to embed");
                        }
                        Err(_) => {
                            tracing::warn!("match image render timed out — falling back to embed");
                            // `timeout` drops the render future before its recovery wrapper
                            // sees an error. Reset the shared browser so the next /match is
                            // never queued behind a poisoned CDP page.
                            img.recycle().await;
                        }
                    }
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
        let name = match self.player_input(interaction, opts).await {
            Ok(name) => name,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        match self.api.player(&name).await {
            Ok(val) => {
                let Some(player_id) = val.get("id") else {
                    return self
                        .reply_text(interaction, format!("Player '{}' not found", name))
                        .await;
                };
                let id = value_id(Some(player_id)).unwrap_or_default();
                let canonical_name = val.get("name").and_then(Value::as_str).unwrap_or(&name);
                match self.api.player_history(&id, 10).await {
                    Ok(rows) => {
                        let embed =
                            embeds::build_history_payload(canonical_name, &rows, &self.web_url);
                        self.send_embed(interaction, embed).await;
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
                tracing::error!(player = name.as_str(), err = %e, "player lookup failed");
                self.reply_text(
                    interaction,
                    api_error_message(&e, &format!("Player '{}' not found", name)),
                )
                .await;
            }
        }
    }

    async fn current(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let name = match self.player_input(interaction, opts).await {
            Ok(name) => name,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        match self.api.live_match(&name).await {
            Ok(val) => {
                let embed = embeds::build_current_payload(&val, &self.web_url);
                self.send_embed(interaction, embed).await;
            }
            Err(error) => {
                self.reply_text(
                    interaction,
                    api_error_message(&error, &format!("Player '{}' not found", name)),
                )
                .await;
            }
        }
    }

    /// Handle `/loadout` — session-based select menu matching the TS bot 1:1.
    async fn loadout(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let name = match self.player_input(interaction, opts).await {
            Ok(name) => name,
            Err(message) => return self.reply_text(interaction, message).await,
        };
        let Some(champion) = opt_string(opts, "champion") else {
            return self
                .reply_text(interaction, "Provide a champion name")
                .await;
        };

        match self.api.player(&name).await {
            Ok(val) => {
                let Some(player_id) = val.get("id") else {
                    let embed = embeds::simple_embed(
                        &format!("{} · {}", name, champion),
                        &format!("Player '{}' not found", name),
                        None,
                    );
                    self.send_webhook(&embed, &[], &interaction.token).await;
                    return;
                };
                let id = value_id(Some(player_id)).unwrap_or_default();

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

                        if champ_loadouts.is_empty() {
                            let embed = embeds::build_no_loadouts_payload(
                                &name,
                                &champion,
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
                        let user_id = extract_user_id(&interaction).unwrap_or_default();

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
                            .map(|lo| {
                                let label = lo
                                    .get("loadout_name")
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "Unnamed Loadout".into())
                                    .chars()
                                    .take(100)
                                    .collect::<String>();
                                let card_points: i64 = lo
                                    .get("card_levels")
                                    .and_then(|v| v.as_array())
                                    .map(|levels| {
                                        levels.iter().filter_map(|value| value.as_i64()).sum()
                                    })
                                    .unwrap_or(0);
                                let description = format!("{} card points", card_points)
                                    .chars()
                                    .take(100)
                                    .collect::<String>();
                                let value = lo.get("id").map(|v| v.to_string()).unwrap_or_default();
                                SelectMenuOption {
                                    default: false,
                                    description: Some(description),
                                    emoji: None,
                                    label,
                                    value,
                                }
                            })
                            .collect();

                        let select_menu = SelectMenu {
                            custom_id: format!("loadout:{}", token),
                            disabled: false,
                            kind: SelectMenuType::Text,
                            options: Some(options),
                            placeholder: Some(format!("Choose a {} loadout", champion)),
                            max_values: Some(1),
                            min_values: Some(1),
                            channel_types: None,
                            default_values: None,
                        };

                        let components = vec![Component::ActionRow(ActionRow {
                            components: vec![Component::SelectMenu(select_menu)],
                        })];

                        let embed = embeds::build_loadout_selection_payload(
                            &name,
                            &champion,
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
                    api_error_message(&error, &format!("Player '{}' not found", name)),
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
        let _ = self
            .http
            .interaction(self.app_id)
            .create_response(interaction_id, token, &resp)
            .await;
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
        let url = self.original_response_url(token);
        let payload = serde_json::json!({
            "content": content,
            "embeds": [],
            "components": [],
            "attachments": [{ "id": 0, "filename": filename, "description": description }],
            "allowed_mentions": { "parse": [] },
        });
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

fn api_error_message(error: &ApiError, fallback: &str) -> String {
    if error.message == "The PaladinsCat service request failed." {
        fallback.to_owned()
    } else {
        error.message.clone()
    }
}

fn missing_saved_player_message() -> String {
    "No player name was entered and you do not have a saved player. Enter a player or use `/save player:<name or ID>` first.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_id_validation_matches_legacy_command() {
        assert!(valid_match_id("1281335238"));
        assert!(!valid_match_id("12345"));
        assert!(!valid_match_id("12813x5238"));
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
    fn loadout_attachment_metadata_matches_legacy_format() {
        let (filename, description) = loadout_attachment_metadata(
            &serde_json::json!({ "name": "Nabi" }),
            &serde_json::json!({ "id": 42, "champion_name": "Mal'Damba", "loadout_name": "Snake Pit" }),
        );
        assert_eq!(filename, "paladinscat-loadout-maldamba-42.png");
        assert_eq!(description, "Nabi's Mal'Damba loadout Snake Pit");
    }
}
