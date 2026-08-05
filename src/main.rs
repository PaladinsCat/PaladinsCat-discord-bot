//! PaladinsCat Discord Bot — Rust rewrite with Twilight 0.16
//!
//! Replaces Node.js + Puppeteer pipeline with native Twilight embed builders.
//! Target: 50-200ms latency, 10-50MB memory, zero GC pressure.

mod api;
mod autocomplete;
mod cache;
mod commands;
mod config;
mod embeds;
mod health;
mod register;

use std::sync::Arc;
use twilight_gateway::{EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client as HttpClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter("paladinscat_discord_bot=info,twilight=warn,reqwest=warn")
        .init();

    let cfg = config::Config::load()?;
    tracing::info!(bot_mode = %cfg.bot_mode, "Starting PaladinsCat Discord Bot");

    let api = Arc::new(api::ApiClient::new(&cfg.api_base_url));
    let render_cache = Arc::new(cache::RenderCache::new(cfg.cache_bytes, cfg.cache_ttl_secs));

    // Spawn health server
    let _handle = health::spawn_server(cfg.health_port);

    // Initialize Discord gateway
    let intents = Intents::GUILDS | Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT;
    let mut shard = Shard::new(ShardId::ONE, cfg.discord_token.clone(), intents);

    let http = Arc::new(HttpClient::new(cfg.discord_token.clone()));

    // Register slash commands on startup
    let app_id = match std::env::var("APPLICATION_ID") {
        Ok(id) => twilight_model::id::Id::new(id.parse::<u64>().unwrap()),
        Err(_) => {
            tracing::warn!("APPLICATION_ID not set; skipping command registration");
            twilight_model::id::Id::new(0)
        }
    };

    if app_id.get() > 0 {
        let dev_guild = cfg.development_guild_id
            .and_then(|s| s.parse::<u64>().ok().map(|n| twilight_model::id::Id::new(n)));
        match register::register_commands(&http, app_id, dev_guild, &[]).await {
            Ok(result) => {
                tracing::info!(
                    scope = %result.scope,
                    registered = result.registered,
                    cleared = result.cleared_guild_scopes,
                    failed = result.failed_guild_scopes,
                    "Commands registered"
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "Command registration failed");
            }
        }
    }

    tracing::info!("Connecting to Discord gateway...");

    while let Some(item) = shard.next_event(EventTypeFlags::all()).await {
        let Ok(event) = item else {
            tracing::warn!(error = ?item.unwrap_err(), "gateway event error");
            continue;
        };

        tokio::spawn(commands::handle_event(
            event,
            Arc::clone(&api),
            Arc::clone(&render_cache),
            Arc::clone(&http),
        ));
    }

    Ok(())
}