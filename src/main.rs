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
mod image;
mod register;

use std::sync::Arc;
use twilight_gateway::{EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client as HttpClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Honor RUST_LOG when set (e.g. DEBUG/TRACE for gateway close diagnostics).
    // Falls back to a sane default that keeps Discord/HTTP noise quiet.
    let default_filter =
        "paladinscat_discord_bot=info,twilight=warn,reqwest=warn,tokio_tungstenite=off";
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .init();

    let cfg = config::Config::load()?;
    tracing::info!(bot_mode = %cfg.bot_mode, "Starting PaladinsCat Discord Bot");

    let service_token = std::env::var("PALADINSCAT_SERVICE_TOKEN").ok();
    let api = Arc::new(api::ApiClient::new(
        &cfg.api_base_url,
        service_token.as_deref(),
    ));
    let render_cache = Arc::new(cache::RenderCache::new(cfg.cache_bytes, cfg.cache_ttl_secs));

    // Spawn health + preview server (shares ApiClient + RenderCache)
    let _handle = health::spawn_server(cfg.health_port, api.clone(), render_cache.clone());

    // Initialize image service (optional — requires Chrome/Chromium on the host).
    // If CHROME_PATH is empty or browser fails to start, image rendering will be
    // unavailable but commands will continue to work with embed-only responses.
    let template_engine = image::TemplateEngine::load(&image::TemplateConfig::dev_defaults());
    let image_service: Option<Arc<image::ImageService>> = match template_engine {
        Ok(te) if !cfg.chrome_path.is_empty() => {
            let renderer = image::MatchRenderer::new(
                te,
                image::MatchRendererConfig {
                    chromium_path: cfg.chrome_path.clone(),
                    debug_port: 0,
                },
            );
            let service =
                image::ImageService::new(Arc::new(renderer), image::ImageServiceConfig::default());
            tracing::info!("Image service initialized");
            let service = Arc::new(service);
            // Warm the browser in the background so the first render is fast and
            // startup failures surface in the logs instead of on the first /match.
            {
                let svc = Arc::clone(&service);
                tokio::spawn(async move {
                    match svc.warm().await {
                        Ok(()) => tracing::info!("Browser warmed up"),
                        Err(e) => tracing::warn!(
                            err = %e,
                            "Browser warm-up failed — first render will cold-start"
                        ),
                    }
                });
            }
            Some(service)
        }
        Ok(te) => {
            tracing::warn!(
                "CHROME_PATH not set — image rendering disabled (commands fall back to embeds)"
            );
            drop(te);
            None
        }
        Err(e) => {
            tracing::warn!(err = %e, "Template loading failed — image rendering disabled");
            None
        }
    };

    // Initialize Discord gateway. Slash commands arrive via InteractionCreate,
    // which requires only the GUILDS intent. MESSAGE_CONTENT is a privileged
    // intent that must be enabled in the developer portal; we don't read raw
    // message content, so requesting it caused close code 4014 (Disallowed
    // intent(s)) and was removed.
    let intents = Intents::GUILDS;

    let http = Arc::new(HttpClient::new(cfg.discord_token.clone()));

    // Register slash commands on startup (skip for dummy/test mode)
    let app_id_raw: u64 = std::env::var("APPLICATION_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let should_register = app_id_raw > 0;
    let app_id: twilight_model::id::Id<twilight_model::id::marker::ApplicationMarker> =
        if should_register {
            twilight_model::id::Id::new(app_id_raw)
        } else {
            tracing::warn!("APPLICATION_ID not set or zero; skipping command registration");
            twilight_model::id::Id::new(1) // dummy; never used
        };

    // TS registers after ClientReady so its guild cache can clear stale
    // development-scope commands. The Ready handler owns registration here too.
    let registration_started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dev_guild = cfg
        .development_guild_id
        .and_then(|s| s.parse::<u64>().ok().map(twilight_model::id::Id::new));

    // Check if we should skip gateway (dummy token or missing)
    let is_dummy = cfg.discord_token.starts_with("dummy") || cfg.discord_token.is_empty();
    if is_dummy {
        tracing::warn!("Discord token is dummy — skipping gateway loop. Health server only.");
        // Keep the health server alive indefinitely
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutting down...");
        return Ok(());
    }

    tracing::info!("Connecting to Discord gateway...");

    // Resilient gateway loop: Twilight auto-reconnects internally on recoverable
    // close codes, but surfaces `None` (FatalClose) for 4004/4010/4013. Rather
    // than letting main() exit 0 (which the container interprets as a normal
    // shutdown and force-restarts), recreate the shard with backoff and log the
    // reason so fatal conditions are visible and recoverable hiccups don't kill
    // the process.
    let mut attempts: u32 = 0;
    loop {
        let mut shard = Shard::new(ShardId::ONE, cfg.discord_token.clone(), intents);
        let stream_ended = loop {
            match shard.next_event(EventTypeFlags::all()).await {
                Some(Ok(event)) => {
                    tokio::spawn(commands::handle_event(
                        event,
                        Arc::clone(&api),
                        Arc::clone(&render_cache),
                        Arc::clone(&http),
                        cfg.web_url.clone(),
                        image_service.clone(),
                        should_register.then_some(app_id),
                        dev_guild,
                        Arc::clone(&registration_started),
                    ));
                }
                Some(Err(err)) => {
                    tracing::warn!(error = %err, "gateway event error");
                }
                None => break true,
            }
        };
        if stream_ended {
            attempts += 1;
            let backoff = (attempts.min(8) as u64) * 2; // 2s, 4s, ... capped at 16s
            tracing::warn!(
                gateway_attempts = attempts,
                backoff_secs = backoff,
                "Gateway stream ended (fatal close or session loss); reconnecting"
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
        }
    }
}
