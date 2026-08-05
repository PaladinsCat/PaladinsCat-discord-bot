//! Health & preview HTTP server — replaces health.ts
//!
//! Exposes:
//!   GET /health                — status + performance metrics
//!   GET /                        — root text ping
//!   GET /preview/cmd/{command}  — HTTP dispatch of bot commands (mirrors TS bot test surfaces)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

use crate::api::ApiClient;
use crate::cache::RenderCache;

// ── Shared state ──────────────────────────────────────────────────────────────

/// Shared state injected via Router::with_state so all routes see the same
/// ApiClient + RenderCache + metrics.  Atomic counters are wrapped in Arc so
/// AppState can implement Clone (required by axum's Router state machinery).
#[derive(Clone)]
pub struct AppState {
    pub api: Arc<ApiClient>,
    pub render_cache: Arc<RenderCache>,

    // Metrics (wrapped in Arc so AppState: Clone)
    commands_processed: Arc<AtomicU64>,
    last_latency_ms: Arc<AtomicU64>,
    total_latency_ms: Arc<AtomicU64>,
}

impl AppState {
    pub fn new(api: Arc<ApiClient>, render_cache: Arc<RenderCache>) -> Self {
        Self {
            api,
            render_cache,
            commands_processed: Arc::new(AtomicU64::new(0)),
            last_latency_ms: Arc::new(AtomicU64::new(0)),
            total_latency_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Record a completed command dispatch (ms).
    fn record(&self, latency_ms: u64) {
        self.commands_processed.fetch_add(1, Ordering::Relaxed);
        self.last_latency_ms.store(latency_ms, Ordering::Relaxed);
        self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
    }

    /// Build the /health metrics sub-object.
    fn health_metrics(&self) -> serde_json::Value {
        let cmds = self.commands_processed.load(Ordering::Relaxed);
        let total = self.total_latency_ms.load(Ordering::Relaxed);
        let last = self.last_latency_ms.load(Ordering::Relaxed);
        let avg = if cmds == 0 { 0 } else { total / cmds };
        let entries = self.render_cache.entry_count();
        let bytes = self.render_cache.approximate_bytes();

        serde_json::json!({
            "commands_processed": cmds,
            "last_latency_ms": last,
            "avg_latency_ms": avg,
            "cache_entries": entries,
            "cache_bytes": bytes
        })
    }
}

// ── Spawn helper ─────────────────────────────────────────────────────────────

/// Start the health + preview server and return a JoinHandle.
pub fn spawn_server(
    port: u16,
    api: Arc<ApiClient>,
    render_cache: Arc<RenderCache>,
) -> tokio::task::JoinHandle<Result<(), std::io::Error>> {
    let state = AppState::new(api, render_cache);
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/", get(root_handler))
        .route("/preview/cmd/:cmd", get(preview_cmd_handler))
        .with_state(state);

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
        tracing::info!(port, "Health server started");
        axum::serve(listener, app).await?;
        Ok(())
    })
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn health_handler(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "paladinscat-discord-bot",
        "bot_mode": "rust",
        "metrics": state.health_metrics()
    }))
}

async fn root_handler() -> String {
    "PaladinsCat Discord Bot — Health OK".to_string()
}

/// GET /preview/cmd/{command}?params…
///
/// Dispatches the same API pipeline as the Discord slash-command handlers, but
/// returns JSON instead of Discord embeds.
async fn preview_cmd_handler(
    State(state): State<AppState>,
    Path(cmd): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let start = Instant::now();

    let result = match cmd.as_str() {
        "player" => preview_player(&state, &params).await,
        "match" => preview_match(&state, &params).await,
        "history" => preview_history(&state, &params).await,
        "current" => preview_current(&state, &params).await,
        "loadout" => preview_loadout(&state, &params).await,
        "champion" => preview_champion(&state, &params).await,
        "maps" => preview_maps(&state, &params).await,
        "composition" => preview_composition(&state, &params).await,
        "items" => preview_items(&state, &params).await,
        unknown => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            state.record(elapsed_ms);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Unknown preview command: {}", unknown),
                    "supported": ["player", "match", "history", "current", "loadout", "champion", "maps", "composition", "items"],
                    "latency_ms": elapsed_ms
                })),
            );
        }
    };

    let elapsed_ms = start.elapsed().as_millis() as u64;
    state.record(elapsed_ms);

    // Attach latency metadata
    let mut response = result;
    response.as_object_mut().map(|obj| {
        obj.insert("latency_ms".into(), serde_json::json!(elapsed_ms));
    });

    (StatusCode::OK, Json(response))
}

// ── Helper: param extraction ────────────────────────────────────────────────

fn param<'a>(params: &'a HashMap<String, String>, key: &str) -> Option<&'a String> {
    params.get(key)
}

fn param_int(params: &HashMap<String, String>, key: &str, default: usize) -> usize {
    params
        .get(key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

// ── Preview implementations ──────────────────────────────────────────────────

/// /preview/cmd/player?name=xxx
async fn preview_player(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let name = param(params, "name");
    match name {
        Some(n) => match state.api.player(n).await {
            Ok(val) => serde_json::json!({ "type": "player", "data": val }),
            Err(_) => serde_json::json!({ "type": "player", "error": format!("Player '{}' not found", n) }),
        },
        None => serde_json::json!({ "type": "player", "error": "Missing required parameter: name" }),
    }
}

/// /preview/cmd/match?id=xxx
async fn preview_match(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let id = param(params, "id");
    match id {
        Some(i) => match state.api.match_info(i).await {
            Ok(val) => serde_json::json!({ "type": "match", "data": val }),
            Err(_) => serde_json::json!({ "type": "match", "error": format!("Match '{}' not found", i) }),
        },
        None => serde_json::json!({ "type": "match", "error": "Missing required parameter: id" }),
    }
}

/// /preview/cmd/history?name=xxx
async fn preview_history(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let name = param(params, "name");
    match name {
        Some(n) => {
            match state.api.player(n).await {
                Ok(val) => {
                    let Some(player_id) = val.get("id") else {
                        return serde_json::json!({ "type": "history", "error": format!("Player '{}' not found", n) });
                    };
                    let id = player_id.to_string();
                    match state.api.player_history(&id, 10).await {
                        Ok(rows) => serde_json::json!({
                            "type": "history",
                            "player": n,
                            "data": val,
                            "history": rows
                        }),
                        Err(_) => serde_json::json!({ "type": "history", "error": "Failed to fetch match history" }),
                    }
                }
                Err(_) => serde_json::json!({ "type": "history", "error": format!("Player '{}' not found", n) }),
            }
        }
        None => serde_json::json!({ "type": "history", "error": "Missing required parameter: name" }),
    }
}

/// /preview/cmd/current?name=xxx
async fn preview_current(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let name = param(params, "name");
    match name {
        Some(n) => match state.api.live_match(n).await {
            Ok(val) => serde_json::json!({ "type": "current", "data": val }),
            Err(_) => serde_json::json!({ "type": "current", "error": format!("Player '{}' not found", n) }),
        },
        None => serde_json::json!({ "type": "current", "error": "Missing required parameter: name" }),
    }
}

/// /preview/cmd/champion?name=xxx   (or no name → list all champions)
async fn preview_champion(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let name = param(params, "name");
    match name {
        Some(n) => {
            let scope = param(params, "lobby").map(|s| s.as_str()).unwrap_or("global");
            match state.api.champion_page_data(&n.to_lowercase(), scope).await {
                Ok(val) => serde_json::json!({ "type": "champion", "data": val }),
                Err(_) => serde_json::json!({ "type": "champion", "error": format!("Champion '{}' not found", n) }),
            }
        }
        None => {
            match state.api.champion_names().await {
                Ok(names) => serde_json::json!({ "type": "champion", "data": names }),
                Err(_) => serde_json::json!({ "type": "champion", "error": "Failed to fetch champions" }),
            }
        }
    }
}

/// /preview/cmd/loadout?name=xxx&champion=xxx
async fn preview_loadout(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let name = param(params, "name");
    let champion = param(params, "champion");
    match (name, champion) {
        (Some(n), Some(c)) => {
            match state.api.player(n).await {
                Ok(val) => {
                    let Some(player_id) = val.get("id") else {
                        return serde_json::json!({ "type": "loadout", "error": format!("Player '{}' not found", n) });
                    };
                    let id = player_id.to_string();
                    match state.api.loadouts(&id).await {
                        Ok(loadouts) => {
                            let champ_loadouts: Vec<_> = loadouts
                                .iter()
                                .filter(|lo| {
                                    lo.get("champion")
                                        .map(|v| v.to_string() == *c)
                                        .unwrap_or(false)
                                })
                                .collect();
                            serde_json::json!({
                                "type": "loadout",
                                "player": n,
                                "champion": c,
                                "data": val,
                                "loadouts": champ_loadouts
                            })
                        }
                        Err(_) => serde_json::json!({ "type": "loadout", "error": "Failed to fetch loadouts" }),
                    }
                }
                Err(_) => serde_json::json!({ "type": "loadout", "error": format!("Player '{}' not found", n) }),
            }
        }
        _ => serde_json::json!({ "type": "loadout", "error": "Missing required parameters: name, champion" }),
    }
}

/// /preview/cmd/maps?limit=N
async fn preview_maps(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let limit = param_int(params, "limit", 10);
    match state.api.ranked_maps(limit).await {
        Ok(rows) => serde_json::json!({ "type": "maps", "data": rows }),
        Err(_) => serde_json::json!({ "type": "maps", "error": "Failed to fetch map stats" }),
    }
}

/// /preview/cmd/composition?limit=N
async fn preview_composition(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let limit = param_int(params, "limit", 10);
    match state.api.ranked_compositions(limit).await {
        Ok(rows) => serde_json::json!({ "type": "composition", "data": rows }),
        Err(_) => serde_json::json!({ "type": "composition", "error": "Failed to fetch composition stats" }),
    }
}

/// /preview/cmd/items?limit=N
async fn preview_items(
    state: &AppState,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    let limit = param_int(params, "limit", 10);
    match state.api.ranked_items(limit).await {
        Ok(rows) => serde_json::json!({ "type": "items", "data": rows }),
        Err(_) => serde_json::json!({ "type": "items", "error": "Failed to fetch item stats" }),
    }
}
