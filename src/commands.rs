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
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};

use crate::api::ApiClient;
use crate::cache::RenderCache;
use crate::embeds;
use crate::image::ImageService;

/// Loadout session — maps a token to player/loadout data with an expiration.
#[derive(Clone)]
struct LoadoutSession {
    user_id: String,
    player: Value,
    loadouts: Vec<Value>,
    champion_name: String,
    player_name: String,
    expires_at: u64,
}

/// 5-minute TTL for loadout sessions.
const LOADOUT_SESSION_TTL_SECS: u64 = 5 * 60;

/// Maximum time to wait for an image render before falling back to an embed.
/// The interaction is deferred first, so this bounds only how long the user
/// waits for the image (or the embed fallback), not Discord's 3s ACK window.
const RENDER_TIMEOUT: Duration = Duration::from_secs(12);

/// Module-level session store.  Shared between command and component handlers.
static LOADOUT_SESSIONS: LazyLock<RwLock<HashMap<String, LoadoutSession>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

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

impl Handler {
    async fn handle_command(&self, interaction: Interaction) {
        let Some(cmd_data) = extract_command_data(&interaction.data) else {
            return self.send_response(
                interaction.id,
                &interaction.token,
                InteractionResponse {
                    kind: InteractionResponseType::Pong,
                    data: None,
                },
            )
            .await;
        };

        match cmd_data.name.as_str() {
            "help" => self.help(&interaction).await,
            "player" | "profile" => self.player(&interaction, &cmd_data.options).await,
            "match" => self.match_cmd(&interaction, &cmd_data.options).await,
            "history" => self.history(&interaction, &cmd_data.options).await,
            "current" => self.current(&interaction, &cmd_data.options).await,
            "loadout" => self.loadout(&interaction, &cmd_data.options).await,
            "champion" => self.champion(&interaction, &cmd_data.options).await,
            "maps" | "composition" | "items" => self.stats(&interaction, &cmd_data.name).await,
            "save" => self.save(&interaction, &cmd_data.options).await,
            other => {
                tracing::debug!(command = other, "unknown command");
                self.send_response(
                    interaction.id,
                    &interaction.token,
                    InteractionResponse {
                        kind: InteractionResponseType::Pong,
                        data: None,
                    },
                )
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

        // Acknowledge the interaction immediately.
        self.defer_response(&interaction).await;

        // Clone the session out, dropping the lock immediately.
        let session = match get_session(token) {
            Some(s) => s,
            None => {
                tracing::debug!(token, "loadout session not found");
                return;
            }
        };

        // Verify the user matches.
        let interaction_user_id = extract_user_id(&interaction);
        if let Some(uid) = &interaction_user_id {
            if uid != &session.user_id {
                tracing::warn!(uid, session_user = session.user_id, "user mismatch on loadout selection");
                return;
            }
        }

        // Check expiry.
        let now = chrono::Utc::now().timestamp() as u64;
        if now >= session.expires_at {
            remove_session(token);
            let embed = embeds::build_no_loadouts_payload(
                &session.player_name,
                &session.champion_name,
                Some("session expired"),
            );
            self.send_webhook(&embed, &[], &interaction.token).await;
            return;
        }

        // Extract the selected loadout ID from the value.
        let Some(selected_value) = data.values.first() else {
            return;
        };
        let loadout_id = selected_value.as_str();

        // Find the selected loadout.
        let selected = session.loadouts.iter().find(|lo| {
            lo.get("id").map(|v| v.to_string() == loadout_id).unwrap_or(false)
        });

        // Delete session after use (single-use token).
        remove_session(token);

        let Some(selected) = selected else {
            return;
        };

        // Build the loadout detail embed and send it.
        let loadouts_vec = vec![selected.clone()];
        let embed = embeds::build_loadouts_payload(
            &session.player_name,
            &loadouts_vec,
            &self.web_url,
            session.player.get("id").and_then(|v| v.as_str()),
        );

        self.send_webhook(&embed, &[], &interaction.token).await;
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
        self.send_embed(interaction, embed).await;
    }

    async fn save(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        if let Some(n) = opt_string(opts, "player") {
            self.reply_text(interaction, format!("Saved player: {}", n)).await;
        }
    }

    async fn player(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let Some(name) = opt_string(opts, "player") else {
            return self.reply_text(interaction, "Provide a player name or ID").await;
        };
        match self.api.discord_player(&name).await {
            Ok(val) => {
                let embed = embeds::build_player_profile(&val, &self.web_url);
                self.send_embed(interaction, embed).await;
            }
            Err(e) => {
                tracing::error!(player = %name, err = %e, "discord_player request failed");
                self.reply_text(interaction, format!("Failed to look up player '{}'", name)).await;
            }
        }
    }

    async fn match_cmd(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let Some(id) = opt_string(opts, "id") else {
            return self.reply_text(interaction, "Provide a match ID").await;
        };
        match self.api.match_info(&id).await {
            Ok(val) => {
                let mode = val.get("mode").map(|v| v.to_string()).unwrap_or_else(|| "Unknown".into());
                let map = val.get("map").map(|v| v.to_string()).unwrap_or_else(|| "Unknown".into());
                let duration = val.get("duration").map(|v| v.to_string()).unwrap_or_else(|| "—".into());
                let url = format!("{}/matches/{}", self.web_url, id);
                let description = format!(
                    "**{}** · {}\nDuration: {}\n[View match]({})",
                    mode, map, duration, url
                );
                let embed = embeds::simple_embed(&format!("Match {}", id), &description, Some(&url));

                // Acknowledge immediately so Discord's 3s window is never exceeded,
                // then render in the background with a bounded timeout.
                self.defer_response(interaction).await;

                if let Some(img) = &self.image_service {
                    let img = Arc::clone(img);
                    let match_id = id.clone();
                    let match_url = url.clone();
                    let token = interaction.token.clone();
                    let embed_for_img = embed.clone();
                    let render = async move { img.render_web_match(&match_id, &match_url).await };
                    match tokio::time::timeout(RENDER_TIMEOUT, render).await {
                        Ok(Ok(png)) => {
                            self.send_followup_image(embed_for_img, png, "match.png", &token)
                                .await;
                            return;
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(err = %e, "match image render failed — falling back to embed");
                        }
                        Err(_) => {
                            tracing::warn!("match image render timed out — falling back to embed");
                        }
                    }
                }

                // Fallback: edit the deferred response with the embed via webhook.
                self.send_webhook(&embed, &[], &interaction.token).await;
            }
            Err(_) => {
                self.reply_text(interaction, format!("Match '{}' not found", id)).await;
            }
        }
    }

    async fn history(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let Some(name) = opt_string(opts, "player") else {
            return self.reply_text(interaction, "Provide a player name").await;
        };
        match self.api.player(&name).await {
            Ok(val) => {
                let Some(player_id) = val.get("id") else {
                    return self.reply_text(interaction, format!("Player '{}' not found", name)).await;
                };
                let id = player_id.as_str().unwrap_or("");
                match self.api.player_history(&id, 10).await {
                    Ok(rows) => {
                        let embed = embeds::build_history_payload(&name, &rows, &self.web_url);
                        self.send_embed(interaction, embed).await;
                    }
                    Err(e) => {
                        tracing::error!(player_id = id, err = %e, "player_history request failed");
                        self.reply_text(interaction, "Failed to fetch match history").await;
                    }
                }
            }
            Err(e) => {
                tracing::error!(player = name.as_str(), err = %e, "player lookup failed");
                self.reply_text(interaction, format!("Player '{}' not found", name)).await;
            }
        }
    }

    async fn current(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let Some(name) = opt_string(opts, "player") else {
            return self.reply_text(interaction, "Provide a player name").await;
        };
        match self.api.live_match(&name).await {
            Ok(val) => {
                let embed = embeds::build_current_payload(&val, &self.web_url);
                self.send_embed(interaction, embed).await;
            }
            Err(_) => {
                self.reply_text(interaction, format!("Player '{}' not found", name)).await;
            }
        }
    }

    /// Handle `/loadout` — session-based select menu matching the TS bot 1:1.
    async fn loadout(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let Some(name) = opt_string(opts, "player") else {
            return self.reply_text(interaction, "Provide a player name").await;
        };
        let Some(champion) = opt_string(opts, "champion") else {
            return self.reply_text(interaction, "Provide a champion name").await;
        };

        // Defer first so we have time for the API call.
        self.defer_response(interaction).await;

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
                let id = player_id.as_str().unwrap_or("");

                match self.api.loadouts(&id).await {
                    Ok(loadouts) => {
                        // Filter to the requested champion (case-insensitive).
                        let champ_loadouts: Vec<Value> = loadouts
                            .iter()
                            .filter(|lo| {
                                lo.get("champion_name")
                                    .map(|v| {
                                        v.to_string().to_lowercase()
                                            == champion.to_lowercase()
                                    })
                                    .unwrap_or(false)
                            })
                            .cloned()
                            .collect();

                        if champ_loadouts.is_empty() {
                            let embed = embeds::build_no_loadouts_payload(
                                &name,
                                &champion,
                                None,
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
                            loadouts: champ_loadouts.clone(),
                            champion_name: champion.clone(),
                            player_name: name.clone(),
                            expires_at: now + LOADOUT_SESSION_TTL_SECS,
                        };
                        insert_session(&token, session);

                        // Build select menu options (max 25).
                        let options: Vec<SelectMenuOption> = champ_loadouts
                            .iter()
                            .take(25)
                            .map(|lo| {
                                let label = lo.get("loadout_name")
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "Unnamed Loadout".into())
                                    .chars().take(100).collect::<String>();
                                let card_points = lo.get("card_points")
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "0".into());
                                let description = format!("{} card points", card_points)
                                    .chars().take(100).collect::<String>();
                                let value = lo.get("id")
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
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
                            id,
                            false,
                        );

                        self.send_webhook(&embed, &components, &interaction.token).await;
                    }
                    Err(_) => {
                        self.reply_text(interaction, "Failed to fetch loadouts").await;
                    }
                }
            }
            Err(_) => {
                self.reply_text(interaction, format!("Player '{}' not found", name)).await;
            }
        }
    }

    async fn champion(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let c: String = opt_string(opts, "champion").unwrap_or_else(|| "any".to_string());
        let scope: String = opt_string(opts, "lobby").unwrap_or_else(|| "global".to_string());
        let lobby_label = match scope.as_str() {
            "bronze-gold" => "Bronze–Gold ranked lobbies",
            "platinum" => "Platinum ranked lobbies",
            "diamond" => "Diamond ranked lobbies",
            _ => "Global ranked lobbies",
        };
        match self.api.champion_page_data(&c.to_lowercase(), &scope).await {
            Ok(val) => {
                let embed = embeds::build_champion_payload(&val, &self.web_url, lobby_label);
                self.send_embed(interaction, embed).await;
            }
            Err(_) => {
                self.reply_text(interaction, format!("No data for champion '{}'", c)).await;
            }
        }
    }

    async fn stats(&self, interaction: &Interaction, command: &str) {
        match command {
            "maps" => {
                match self.api.ranked_maps(100).await {
                    Ok(rows) => {
                        let embed = embeds::build_maps_payload(&rows, &self.web_url);
                        self.send_embed(interaction, embed).await;
                    }
                    Err(_) => {
                        self.reply_text(interaction, "Failed to fetch map stats").await;
                    }
                }
            }
            "composition" => {
                match self.api.ranked_compositions(5).await {
                    Ok(rows) => {
                        let embed = embeds::build_composition_payload(&rows, &self.web_url);
                        self.send_embed(interaction, embed).await;
                    }
                    Err(_) => {
                        self.reply_text(interaction, "Failed to fetch composition stats").await;
                    }
                }
            }
            "items" => {
                match self.api.ranked_items(20).await {
                    Ok(rows) => {
                        let embed = embeds::build_items_payload(&rows, &self.web_url, "Global ranked lobbies");
                        self.send_embed(interaction, embed).await;
                    }
                    Err(_) => {
                        self.reply_text(interaction, "Failed to fetch item stats").await;
                    }
                }
            }
            _ => {
                self.reply_text(interaction, format!("{} stats coming soon", command)).await;
            }
        }
    }

    // ——— Helpers ———

    async fn send_embed(&self, interaction: &Interaction, embed: Embed) {
        let data = InteractionResponseData {
            embeds: Some(vec![embed]),
            ..Default::default()
        };
        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::ChannelMessageWithSource,
                data: Some(data),
            },
        )
        .await;
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
        let data = InteractionResponseData {
            content: Some(msg.into()),
            ..Default::default()
        };
        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::ChannelMessageWithSource,
                data: Some(data),
            },
        )
        .await;
    }

    /// Defer the initial interaction response (15-second ACK).
    async fn defer_response(&self, interaction: &Interaction) {
        self.send_response(
            interaction.id,
            &interaction.token,
            InteractionResponse {
                kind: InteractionResponseType::DeferredChannelMessageWithSource,
                data: None,
            },
        )
        .await;
    }

    /// Send the deferred response / edit the original via the webhook endpoint.
    /// Uses `PATCH /webhooks/{app_id}/{token}` with the data payload.
    async fn send_webhook(&self, embed: &Embed, components: &[Component], token: &str) {
        let webhook_id = self.app_id.get();
        let url = format!("https://discord.com/api/v9/webhooks/{}/{}", webhook_id, token);
        let payload = serde_json::json!({
            "embeds": [embed],
            "components": if components.is_empty() { serde_json::Value::Null } else { serde_json::json!(components) },
        });
        let client = reqwest::Client::new();
        match client.patch(&url).json(&payload).send().await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!(url = %url, err = %e, "webhook PATCH failed");
            }
        }
    }

    /// Send an embed + PNG image attachment via the webhook follow-up endpoint.
    /// Uses multipart form data: embed in the `payload_json` field, image as `FILE`.
    async fn send_followup_image(
        &self,
        embed: Embed,
        png: Vec<u8>,
        filename: &str,
        token: &str,
    ) {
        let webhook_id = self.app_id.get();
        let url = format!(
            "https://discord.com/api/v9/webhooks/{}/{}",
            webhook_id, token
        );
        let payload = serde_json::json!({
            "embeds": [embed],
        });
        let client = reqwest::Client::new();
        let body = reqwest::multipart::Form::new()
            .text("payload_json", serde_json::to_string(&payload).unwrap_or_default())
            .part(
                "FILE",
                reqwest::multipart::Part::bytes(png.to_vec())
                    .file_name(filename.to_string()),
            );
        match client.post(&url).multipart(body).send().await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!(url = %url, err = %e, "follow-up image POST failed");
            }
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
