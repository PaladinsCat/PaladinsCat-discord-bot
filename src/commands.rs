//! Slash command handlers — dispatches InteractionCreate events.

use std::sync::Arc;
use twilight_gateway::Event;
use twilight_http::Client as HttpClient;
use twilight_model::application::interaction::{
    application_command::{CommandData, CommandDataOption, CommandOptionValue},
    Interaction, InteractionData, InteractionType,
};
use twilight_model::channel::message::embed::{Embed, EmbedField};
use twilight_model::http::interaction::{InteractionResponse, InteractionResponseData, InteractionResponseType};
use twilight_util::builder::embed::{EmbedBuilder, EmbedFooterBuilder};

use crate::api::ApiClient;
use crate::cache::RenderCache;
use crate::embeds;

struct Handler {
    api: Arc<ApiClient>,
    _cache: Arc<RenderCache>,
    http: Arc<HttpClient>,
    app_id: twilight_model::id::Id<twilight_model::id::marker::ApplicationMarker>,
}

fn make_field(name: String, value: String) -> EmbedField {
    EmbedField { inline: false, name, value }
}

/// Main event dispatcher — routes gateway events to command handlers.
pub async fn handle_event(
    event: Event,
    api: Arc<ApiClient>,
    render_cache: Arc<RenderCache>,
    http: Arc<HttpClient>,
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
                    });
                    tokio::spawn(async move { h.handle_command(interaction).await });
                }
                InteractionType::ApplicationCommandAutocomplete => {
                    let h = Arc::new(Handler {
                        api: api.clone(),
                        _cache: render_cache.clone(),
                        http: http.clone(),
                        app_id: interaction.application_id,
                    });
                    tokio::spawn(async move { h.handle_autocomplete(interaction).await });
                }
                _ => {}
            }
        }
        _ => {}
    }
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

    // ——— Command implementations ———

    async fn help(&self, interaction: &Interaction) {
        let embed = EmbedBuilder::new()
            .title("PaladinsCat Bot Commands")
            .description(
                "```/player <name>    — Player profile\n\
                 /match <id>         — Match result\n\
                 /history <name>     — Recent matches\n\
                 /current <name>     — Live match\n\
                 /champion <name>    — Champion stats\n\
                 /maps               — Ranked map stats\n\
                 /composition        — Top team comps\n\
                 /items              — Item stats\n\
                 /loadout <champ>    — Loadout image```",
            )
            .color(embeds::color::PRIMARY)
            .build();
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
                // /players/discord returns {"player": {...}} — extract inner profile
                let profile = val.get("player").cloned().unwrap_or(val);
                let mut builder = EmbedBuilder::new().color(embeds::color::PRIMARY);
                if let Some(gamertag) = profile.get("gamertag") {
                    let gt = gamertag.as_str().unwrap_or("N/A");
                    builder = builder.title(gt).footer(EmbedFooterBuilder::new(format!("Gamertag: {}", gt)));
                } else {
                    builder = builder.title(&name);
                }
                if let Some(hr) = profile.get("headroom") {
                    builder = builder.field(make_field("Headroom".to_string(), hr.to_string()));
                }
                if let Some(peak) = profile.get("peak_rank") {
                    builder = builder.field(make_field("Peak Rank".to_string(), peak.to_string()));
                }
                self.send_embed(interaction, builder.build()).await;
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
                let mut builder = EmbedBuilder::new().title("Match Result").color(embeds::color::VICTORY);
                if let Some(mode) = val.get("mode") {
                    builder = builder.description(mode.to_string());
                }
                if let Some(dur) = val.get("duration") {
                    builder = builder.field(make_field("Duration".to_string(), dur.to_string()));
                }
                if let Some(map) = val.get("map") {
                    builder = builder.field(make_field("Map".to_string(), map.to_string()));
                }
                self.send_embed(interaction, builder.build()).await;
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
                        let mut builder = EmbedBuilder::new()
                            .title(format!("Match History — {}", name))
                            .color(embeds::color::PRIMARY);
                        for (i, row) in rows.iter().take(10).enumerate() {
                            let match_id = row.get("match_id").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            let mode = row.get("mode").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            builder = builder.field(make_field(format!("{}. {}", i + 1, mode), match_id));
                        }
                        self.send_embed(interaction, builder.build()).await;
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
                if val.get("in_game").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let map = val.get("map").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                    let mode = val.get("mode").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                    let duration = val.get("duration").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                    let mut builder = EmbedBuilder::new()
                        .title(format!("Currently in Game — {}", name))
                        .description(format!("Map: {} | Mode: {}", map, mode))
                        .color(embeds::color::IN_GAME);
                    builder = builder.field(make_field("Duration".to_string(), duration));
                    self.send_embed(interaction, builder.build()).await;
                } else {
                    self.reply_text(interaction, format!("{} is not currently in a match", name)).await;
                }
            }
            Err(_) => {
                self.reply_text(interaction, format!("Player '{}' not found", name)).await;
            }
        }
    }

    async fn loadout(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
        let Some(name) = opt_string(opts, "player") else {
            return self.reply_text(interaction, "Provide a player name").await;
        };
        let Some(champion) = opt_string(opts, "champion") else {
            return self.reply_text(interaction, "Provide a champion name").await;
        };
        match self.api.player(&name).await {
            Ok(val) => {
                let Some(player_id) = val.get("id") else {
                    return self.reply_text(interaction, format!("Player '{}' not found", name)).await;
                };
                let id = player_id.as_str().unwrap_or("");
                match self.api.loadouts(&id).await {
                    Ok(loadouts) => {
                        let champ_loadouts: Vec<_> = loadouts
                            .iter()
                            .filter(|lo| {
                                lo.get("champion").map(|v| v.to_string().eq(&champion)).unwrap_or(false)
                            })
                            .collect();
                        if champ_loadouts.is_empty() {
                            self.reply_text(interaction, format!("No {} loadouts found for {}", champion, name)).await;
                        } else {
                            let mut builder = EmbedBuilder::new()
                                .title(format!("{} Loadouts — {}", champion, name))
                                .color(embeds::color::PRIMARY);
                            for lo in champ_loadouts.iter().take(5) {
                                let l_name = lo.get("loadout_name").map(|v| v.to_string()).unwrap_or_else(|| "Unnamed".into());
                                let cards: usize = lo.get("card_levels")
                                    .and_then(|arr| arr.as_array())
                                    .map(|arr| arr.iter().filter_map(|v| v.as_i64()).sum())
                                    .unwrap_or(0) as usize;
                                builder = builder.field(make_field(l_name, format!("{} cards", cards)));
                            }
                            self.send_embed(interaction, builder.build()).await;
                        }
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
        match self.api.champion_page_data(&c.to_lowercase(), &scope).await {
            Ok(val) => {
                let mut builder = EmbedBuilder::new()
                    .title(&c)
                    .color(embeds::color::PRIMARY);
                if let Some(wp) = val.get("win_rate") {
                    builder = builder.field(make_field("Win Rate".to_string(), wp.to_string()));
                }
                if let Some(pick) = val.get("pick_rate") {
                    builder = builder.field(make_field("Pick Rate".to_string(), pick.to_string()));
                }
                if let Some(games) = val.get("games") {
                    builder = builder.field(make_field("Games".to_string(), games.to_string()));
                }
                self.send_embed(interaction, builder.build()).await;
            }
            Err(_) => {
                self.reply_text(interaction, format!("No data for champion '{}'", c)).await;
            }
        }
    }

    async fn stats(&self, interaction: &Interaction, command: &str) {
        match command {
            "maps" => {
                match self.api.ranked_maps(10).await {
                    Ok(rows) => {
                        let mut builder = EmbedBuilder::new()
                            .title("Ranked Map Stats")
                            .color(embeds::color::PRIMARY);
                        for row in rows.iter().take(10) {
                            let map = row.get("map").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            let games = row.get("games").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            builder = builder.field(make_field(map, games));
                        }
                        self.send_embed(interaction, builder.build()).await;
                    }
                    Err(_) => {
                        self.reply_text(interaction, "Failed to fetch map stats").await;
                    }
                }
            }
            "composition" => {
                match self.api.ranked_compositions(10).await {
                    Ok(rows) => {
                        let mut builder = EmbedBuilder::new()
                            .title("Top Compositions")
                            .color(embeds::color::PRIMARY);
                        for row in rows.iter().take(10) {
                            let champs = row.get("champions").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            let games = row.get("games").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            builder = builder.field(make_field(champs, games));
                        }
                        self.send_embed(interaction, builder.build()).await;
                    }
                    Err(_) => {
                        self.reply_text(interaction, "Failed to fetch composition stats").await;
                    }
                }
            }
            "items" => {
                match self.api.ranked_items(10).await {
                    Ok(rows) => {
                        let mut builder = EmbedBuilder::new()
                            .title("Item Stats")
                            .color(embeds::color::PRIMARY);
                        for row in rows.iter().take(10) {
                            let item = row.get("item").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            let pick = row.get("pick_rate").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                            builder = builder.field(make_field(item, pick));
                        }
                        self.send_embed(interaction, builder.build()).await;
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
