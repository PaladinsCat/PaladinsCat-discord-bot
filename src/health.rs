//! Health monitoring HTTP server — replaces health.ts
//! Replaces health.ts (port 3020, /health, /preview endpoints)

use axum::{routing::get, Json, Router};
use serde_json::json;

/// Start the health server and return a handle.
pub fn spawn_server(port: u16) -> tokio::task::JoinHandle<Result<(), std::io::Error>> {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/", get(root_handler));

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
        tracing::info!(port, "Health server started");
        axum::serve(listener, app).await?;
        Ok(())
    })
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "service": "paladinscat-discord-bot",
        "bot_mode": "rust"
    }))
}

async fn root_handler() -> String {
    "PaladinsCat Discord Bot — Health OK".to_string()
}