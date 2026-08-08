# Image Generation Architecture — Rust Discord Bot

**Status**: Draft · Owner: Rust Bot Team · Date: 2026-08-07

---

## 1. Options Analysis

### 1A — HTTP Client to Existing Render Service

Call the TS bot's render endpoint (or backend API) via `reqwest`, receive PNG bytes back, attach to Discord response.

| Criterion | Assessment |
|---|---|
| **Visual parity** | **Perfect** — reuses the same Puppeteer pipeline, HTML template, and fonts. Pixel-identical output guaranteed. |
| **Performance** | **Mixed** — adds one network hop (~5–50ms inter-container). The TS bot's render queue (concurrency=2, 8s timeout) still gates throughput. A dedicated render-only HTTP endpoint avoids running two copies of Chromium. |
| **Complexity** | **Low** — ~200 lines: an `ImageClient` struct, `render_match()` / `render_loadout()` methods, and a render queue wrapper. No new dependencies. |
| **Runtime** | **None** — no Chrome, no fonts needed in the Rust container. TS bot container already ships Chromium + fontconfig + fonts. |
| **Risk** | If the TS bot is decommissioned, rendering stops. Coupling between two services. Requires a dedicated `/api/render` endpoint on the TS bot or a shared render microservice. |

**Verdict**: Lowest effort, highest parity. Viable only if TS bot stays alive alongside Rust bot during transition.

### 1B — Canvas-based Rendering with `image` Crate

Generate PNGs pixel-by-pixel using `image` 0.24. Draw rectangles, text (via `rusttype` or `ab_glyph`), and composite layers to replicate the TS template.

| Criterion | Assessment |
|---|---|
| **Visual parity** | **Hard** — the HTML template (`match-result-scoreboard.html`, v14) uses CSS gradients, border-radius, box-shadow, `clip-path`, and complex flex layouts. Reproducing exact CSS rendering in Rust is a multi-month effort. Even 95% parity is risky. |
| **Performance** | **Excellent** — pure in-process rendering, no network hop, no browser overhead. Sub-100ms per image. |
| **Complexity** | **Very High** — need to port the entire rendering pipeline: HTML/CSS → canvas drawing. Must handle champion icons, tier badges, team backgrounds, gradient overlays, card frame rendering, and dynamic-width text layouts. |
| **Runtime** | **Minimal** — `image` crate + optional `rusttype`/`ab_glyph` for fonts. No Chrome dependency. |
| **Risk** | High effort, low ROI. Even with `usvg`/`resvg` for SVG support, CSS layout reproduction is non-trivial. Fonts need embedding. |

**Verdict**: Reject. The gap between "looks similar" and "pixel-identical" is enormous for the HTML/CSS templates the TS bot already maintains.

### 1C — Headless Browser via CDP (tokio-tungstenite + Chrome DevTools Protocol)

Spawn a headless Chromium instance inside the Rust container, communicate via WebSocket using `tokio-tungstenite`, load the HTML template, fill data, screenshot, return bytes.

| Criterion | Assessment |
|---|---|
| **Visual parity** | **Perfect** — same Chromium engine, same HTML template, same fonts. 1:1 with TS bot output. |
| **Performance** | **Good** — in-process Chromium avoids inter-service latency. Queue limits (concurrency=2) bound memory. Render takes 2–4s per image. |
| **Complexity** | **Medium** — must install Chromium + fonts in Docker, implement CDP session lifecycle, page navigation, `Page.captureScreenshot`, and queue management. ~1500 lines. |
| **Runtime** | **Heavy** — Chromium (~300MB) + fonts in the Docker image. Container size grows from ~25MB to ~350MB. |
| **Risk** | Chromium in a container requires `--no-sandbox` flag, which is a known security consideration. Page crashes, OOM on burst traffic. |

**Verdict**: Best long-term path. Full parity, no service coupling, single-container deployment.

---

## 2. Recommendation

### Winner: Approach C (CDP + Headless Chromium)

**Rationale**:

1. **Pixel-perfect parity** — identical rendering engine and template. The `/match` scoreboard (2048×1152) and `/loadout` card (1280×720) use the same HTML (`match-result-scoreboard.html` v14 / loadout template v9) and same fonts (Inter, Dejavu).

2. **Independent deployment** — Rust bot is a self-contained unit. No dependency on the TS bot surviving. Critical for the migration timeline.

3. **Performance envelope** — 2 concurrent Chromium pages (~150MB each) within the OVH container's memory budget. Queue concurrency of 2 matches the TS bot.

4. **Future-proof** — Adding new render targets (highlight cards, stat images) only requires new HTML templates, not new rendering code.

### Why not Approach A as Phase 1?

HTTP delegation works as a bootstrap if the TS bot is still running, but it creates a migration bottleneck: the Rust bot cannot ship independently, and the TS bot must expose a render API surface it was never designed for. The added HTTP hop also introduces latency and failure modes (network partition, TS bot OOM) that complicate debugging.

### Why not Approach B?

The `image` crate approach cannot produce pixel-identical output for the CSS-heavy templates. The scoreboard uses:
- CSS `linear-gradient()` backgrounds with semi-transparent stops
- `box-shadow` with multiple layered shadows for depth
- `border-radius` on card corners with gradient strokes
- Flexbox layout for player rows that wrap responsively
- `clip-path` for the center dividing line
- Dynamic text sizing with `clamp()` and variable-width fonts

Reproducing all of this in a 2D canvas API would be a 2–3 month effort for two commands and still risk subtle differences. The `image` crate dependency already in `Cargo.toml` is retained as a **fallback for thumbnails and post-processing** (resize, watermark).

---

## 3. Implementation Plan

### 3.1 — File Structure

```
src/discord-bot-rust/src/
├── image/
│   ├── mod.rs          # Module entry, public trait + factory
│   ├── cdp_client.rs   # Chrome DevTools Protocol WebSocket client
│   ├── render_queue.rs # BoundedWorkQueue (mirrors TS BoundedWorkQueue)
│   ├── template.rs     # HTML template string injection + data binding
│   └── cache.rs        # Render cache integration (bridges to crate::cache)
├── commands.rs         # Updated: match_cmd() / loadout() call image::render()
├── main.rs             # Updated: spawn Chromium, init ImageService
├── config.rs           # Updated: IMAGE_* config fields
└── ...existing modules
```

### 3.2 — New Files

#### `image/mod.rs`

```rust
//! Image rendering subsystem — headless Chromium via CDP.
//!
//! Mirrors TS: render-service.ts + render-queue.ts + match-renderer.ts

pub mod cdp_client;
pub mod render_queue;
pub mod template;

use std::sync::Arc;
use tokio::sync::mpsc;

/// Public entry point for image rendering.
pub struct ImageService {
    queue: Arc<render_queue::RenderQueue>,
    chrome: Arc<cdp_client::CdpSession>,
    cache: Arc<crate::cache::RenderCache>,
    cooldowns: moka::future::Cache<String, std::time::Instant>,
}

impl ImageService {
    /// Render a match scoreboard image (2048x1152).
    pub async fn render_match(&self, record: MatchRecord) -> Result<Vec<u8>, ImageError>;

    /// Render a loadout card image (1280x720).
    pub async fn render_loadout(&self, record: LoadoutRecord) -> Result<Vec<u8>, ImageError>;
}
```

#### `image/cdp_client.rs`

```rust
//! CDP session manager — spawns headless Chromium, manages WebSocket sessions.
//!
//! Protocol: tokio-tungstenite for WebSocket transport.
//! CDP commands: Page.navigate, Page.screenshot, Runtime.evaluate.

use tokio_tungstenite::WebSocketStream;
use tungstenite::protocol::WebSocket;

pub struct CdpSession {
    ws: mpsc::UnboundedSender<cdp::Command>,
    ws_url: String,
}

impl CdpSession {
    /// Spawn headless Chromium and connect via CDP WebSocket.
    pub async fn launch() -> Result<Self, CdpError>;

    /// Navigate to data URI and capture screenshot.
    pub async fn screenshot(&self, html: String, width: u32, height: u32) -> Result<Vec<u8>, CdpError>;

    /// Recycle: close browser, restart. Called on page crashes.
    pub async fn recycle(&mut self);
}
```

#### `image/render_queue.rs`

```rust
//! Bounded work queue — mirrors TS BoundedWorkQueue.
//!
//! Concurrency: 2 concurrent renders (matches TS default).
//! Queue limit: 20 pending (rejects with QueueFullError above this).
//! Timeout: 8000ms per render attempt.
//! Deduplication: same cache key within the window returns the existing promise.

use tokio::sync::{mpsc, watch};

pub struct RenderQueue {
    sender: mpsc::Sender<RenderJob>,
    in_flight: tokio::sync::RwLock<FxHashStringMap<tokio::sync::oneshot::Sender<Vec<u8>>>>,
}

#[derive(Debug)]
pub struct RenderJob {
    key: String,
    work: Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<Vec<u8>, RenderError>>>> + Send>,
    response: oneshot::Sender<Result<Vec<u8>, RenderError>>,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("The image queue is busy. Try again shortly.")]
    QueueFull,
    #[error("Render timed out")]
    Timeout,
    #[error("Browser crash")]
    BrowserCrashed,
}
```

#### `image/template.rs`

```rust
//! HTML template injection — fills the scoreboard/loadout templates with data.
//!
//! Mirrors TS: match-renderer.ts xml() escaping + template string building.

pub fn build_match_html(record: &MatchRecord) -> String;
pub fn build_loadout_html(record: &LoadoutRecord) -> String;
```

### 3.3 — Existing File Changes

#### `commands.rs`

```diff
  struct Handler {
      api: Arc<ApiClient>,
+     image_service: Arc<image::ImageService>,
      _cache: Arc<RenderCache>,
      http: Arc<HttpClient>,
      ...
  }

  async fn match_cmd(&self, interaction: &Interaction, opts: &[CommandDataOption]) {
      let Some(id) = opt_string(opts, "id") else {
          return self.reply_text(interaction, "Provide a match ID").await;
      };
+     // Check cooldown (10s per user)
+     if !self.image_service.check_cooldown(interaction.user_id).await {
+         return self.reply_text(interaction, "Image cooldown active. Try again in 10 seconds.").await;
+     }
      match self.api.match_info(&id).await {
          Ok(val) => {
+             let record = MatchRecord::from_api_response(&val)?;
+             match self.image_service.render_match(record).await {
+                 Ok(bytes) => {
+                     // Attach image to Discord embed
+                     let attach = self.upload_and_attach(&bytes, interaction).await?;
+                     let embed = embeds::match_image_embed(&val, &attach.url, &self.web_url);
+                     self.send_embed(interaction, embed).await;
+                 }
+                 Err(ImageError::QueueFull) => {
+                     self.reply_text(interaction, "The image queue is busy. Try again shortly.").await;
+                 }
+                 Err(_) => {
+                     // Fall back to embed
+                     let embed = embeds::simple_embed(...);
+                     self.send_embed(interaction, embed).await;
+                 }
+             }
          }
      }
  }
```

Same pattern for `loadout()`.

#### `main.rs`

```diff
 mod api;
 mod autocomplete;
 mod cache;
 mod commands;
 mod config;
 mod embeds;
+mod image;
 mod health;
 mod register;

 async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
     let cfg = config::Config::load()?;
+
+    // Launch headless Chromium for image rendering
+    let cdp = image::cdp_client::CdpSession::launch().await?;
+    let render_cache = Arc::new(cache::RenderCache::new(cfg.cache_bytes, cfg.cache_ttl_secs));
+    let image_service = Arc::new(image::ImageService::new(
+        cdp,
+        render_cache.clone(),
+        image::render_queue::RenderQueue::new(cfg.image_concurrency, cfg.image_queue_limit, cfg.image_timeout_ms),
+    ));
+
+    // Pass image_service to command handler
     ...
 }
```

#### `config.rs`

```diff
 pub struct Config {
     ...
+    /// Max concurrent image renders (mirrors TS: queue.concurrency = 2)
+    pub image_concurrency: usize,
+    /// Max queued image renders before rejection
+    pub image_queue_limit: usize,
+    /// Per-render timeout in ms (mirrors TS: 8000ms)
+    pub image_timeout_ms: u64,
+    /// Image cooldown in seconds per user (10s)
+    pub image_cooldown_secs: u64,
+    /// Path to Chromium binary
+    pub chromium_path: String,
 }

 impl Config {
     pub fn load() -> Result<Self, ...> {
         Ok(Config {
             ...
+            image_concurrency: parse_env("IMAGE_CONCURRENCY", 2),
+            image_queue_limit: parse_env("IMAGE_QUEUE_LIMIT", 20),
+            image_timeout_ms: parse_env("IMAGE_TIMEOUT_MS", 8000),
+            image_cooldown_secs: parse_env("IMAGE_COOLDOWN_SECS", 10),
+            chromium_path: std::env::var("CHROMIUM_PATH").unwrap_or_else(|_| "/usr/bin/chromium".into()),
         })
     }
 }
```

#### `Cargo.toml`

```diff
  # Image processing (replaces sharp)
  image = "0.24"
+
+# CDP / WebSocket
+tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
+tungstenite = "0.24"
+
+# For data URI encoding
+base64 = "0.22"
+
+# Error types
+thiserror = "2"
```

#### `cache.rs` — Updates to RenderCache

The existing `RenderCache` stores `String` values (base64-encoded images). Expand to support `Vec<u8>` directly:

```diff
 pub struct RenderCache {
-    inner: Cache<String, String>,
+    inner: Cache<String, Vec<u8>>,
 }

 impl RenderCache {
-    pub async fn get(&self, key: &str) -> Option<String> { ... }
+    pub async fn get(&self, key: &str) -> Option<Vec<u8>> { ... }
+    pub async fn set(&self, key: String, value: Vec<u8>) { ... }
 }
```

### 3.4 — Data Structures

#### `MatchRecord`

```rust
//! Mirrors TS: MatchRecord + MatchPlayer

#[derive(Debug, Clone, serde::Serialize)]
pub struct MatchRecord {
    pub match_id: String,
    pub entry_datetime: String,
    pub queue_id: i32,
    pub duration_seconds: i64,
    pub region: String,
    pub map: String,
    pub team1_score: Option<i32>,
    pub team2_score: Option<i32>,
    pub winning_task_force: i32,
    pub players: Vec<MatchPlayer>,
    pub bans: Vec<BanEntry>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MatchPlayer {
    pub player_id: String,
    pub player_name: String,
    pub champion_id: i32,
    pub champion_name: String,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub damage_done_physical: i64,
    pub damage_done_in_hand: Option<i64>,
    pub damage_taken: i64,
    pub damage_mitigated: i64,
    pub healing: i64,
    pub gold_earned: i64,
    pub objective_assists: Option<i64>,
    pub final_match_level: Option<i32>,
    pub account_level: Option<i32>,
    pub party: Option<i32>,
    pub party_id: Option<i32>,
    pub party_number: Option<i32>,
    pub tier: Option<i32>,
    /// Which team: 1 or 2
    pub team: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BanEntry {
    pub ban_slot: Option<i32>,
    pub champion_id: i32,
    pub champion_name: String,
}
```

#### `LoadoutRecord`

```rust
//! Mirrors TS: LoadoutRenderRecord

#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadoutRecord {
    pub player: PlayerInfo,
    pub loadout: ChampionLoadout,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlayerInfo {
    pub id: String,
    pub name: String,
    pub headroom: i32,
    pub peak_rank: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChampionLoadout {
    pub id: String,
    pub champion_id: i32,
    pub champion_name: String,
    pub champion_icon: String,
    pub cards: Vec<Card>,
    pub talents: Vec<Talent>,
    pub updated_at: String,
}
```

#### `RenderQueue` State Machine

```
Idle → [user sends /match] → Cooldown Check → Cache Lookup → Queue Enqueue
    → Cooldown active: "Try again in 10s"
    → Cache hit: return cached image instantly
    → Queue full: "The image queue is busy. Try again shortly."
    → Enqueued → Rendering → Browser crash? → Recycle + Retry (max 2 attempts)
    → Success → Store in RenderCache → Return bytes to Discord
    → Timeout: "Image render timed out" → fall back to embed
```

---

## 4. API Integration

### 4.1 — Data Flow

```
User types /match id:xxx
    │
    ├─ commands.rs::match_cmd()
    │   ├─ Check cooldown (10s per user via moka Cache)
    │   ├─ cache::RenderCache::get("match:xxx:summary:v14")
    │   │   └─ Cache hit → return cached image
    │   │   └─ Cache miss → continue
    │   ├─ api::ApiClient::match_info("xxx")
    │   │   └─ GET {API_BASE_URL}/matches/xxx
    │   │   └─ Returns: { match_id, mode, map, duration, players: [...], ... }
    │   ├─ image::ImageService::render_match(MatchRecord)
    │   │   ├─ render_queue::add(key, work_fn)
    │   │   ├─ cdp_client::screenshot(html, 2048, 1152)
    │   │   │   └─ Navigate to data URI → capture PNG
    │   │   ├─ cache::RenderCache::set(key, png_bytes)
    │   │   └─ Return Vec<u8>
    │   └─ Discord API: create_message + attach file
    │
    └─ Fallback on any error: send text embed (current behavior)
```

### 4.2 — API Endpoints Used

| Endpoint | Method | Purpose | Used By |
|---|---|---|---|
| `{API_BASE_URL}/matches/{id}` | GET | Match detail data | `/match` |
| `{API_BASE_URL}/players/{id}?include=ratings` | GET | Player profile | `/match` (player names), `/loadout` |
| `{API_BASE_URL}/players/{id}/loadouts` | GET | Loadout list | `/loadout` |
| `{API_BASE_URL}/players/{name}` | GET | Player ID resolution | `/match`, `/loadout` |

These endpoints are already implemented in `api::ApiClient` (`match_info()`, `player()`, `loadouts()`).

### 4.3 — Cooldown Enforcement

```rust
// Per-user cooldown check
async fn check_cooldown(&self, user_id: &str) -> bool {
    if let Some(last_used) = self.cooldowns.get(user_id).await {
        if Instant::now() - last_used < Duration::from_secs(10) {
            return false;
        }
    }
    self.cooldowns.insert(user_id.to_string(), Instant::now()).await;
    true
}
```

### 4.4 — Queue Overflow Handling

Error messages mirror the TS bot exactly:

| Condition | Error Message |
|---|---|
| Queue full (>20 pending) | `"The image queue is busy. Try again shortly."` |
| Render timeout (>8s) | `"Image render timed out. Please try again."` |
| Browser crash (recovery exhausted) | `"Image rendering failed. Please try again."` |
| Cooldown active | `"Cooldown active. Try again in a few seconds."` |
| Cache miss + API failure | Fallback to embed (current behavior) |

---

## 5. Docker & Runtime

### 5.1 — Dockerfile Changes

```dockerfile
FROM rust:slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Install Chromium for headless rendering
RUN apt-get update && apt-get install -y --no-install-recommends \
    chromium fontconfig fonts-inter fonts-dejavu-core && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock .
COPY src/ src/
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl chromium fontconfig fonts-inter fonts-dejavu-core && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/paladinscat-discord-bot .
# Copy HTML templates from the TS bot's dev/prototypes directory
COPY src/discord-bot/dev/prototypes/match-result-scoreboard.html ./templates/
COPY src/discord-bot/dev/prototypes/cheater-police-line.svg ./templates/

# Champion assets (icons, banners) embedded from frontend
COPY src/frontend/public/images/champions/ ./assets/champions/
COPY src/frontend/public/images/cards/ ./assets/cards/

ENV CHROMIUM_PATH=/usr/bin/chromium \
    HOME=/tmp \
    XDG_CACHE_HOME=/tmp

EXPOSE 3020
ENTRYPOINT ["./paladinscat-discord-bot"]
```

### 5.2 — Container Size Estimate

| Component | Size |
|---|---|
| Rust binary | ~10MB |
| Chromium | ~150MB |
| Fonts | ~10MB |
| Templates + assets | ~50MB |
| Base image | ~100MB |
| **Total** | **~320MB** (vs. ~25MB without Chromium) |

### 5.3 — Chromium Launch Arguments

```bash
chromium --headless --no-sandbox --disable-gpu \
  --disable-dev-shm-usage \
  --disable-software-rasterizer \
  --screenshot \
  --window-size=2048,1152 \
  --remote-debugging-port=9222
```

- `--no-sandbox`: Required in container (no user namespace)
- `--disable-dev-shm-usage`: Prevents `/dev/shm` overflow in small containers
- Template copied to `./templates/` directory, served via `file://` or `data:` URI

---

## 6. Migration Path

### Phase 1: CDP Infrastructure (Week 1)
- [ ] Add `tokio-tungstenite`, `base64`, `thiserror` to `Cargo.toml`
- [ ] Implement `image/cdp_client.rs` — spawn Chromium, WebSocket connect, basic screenshot
- [ ] Dockerfile: add Chromium + fonts
- [ ] Test: `render("hello world", 800, 600)` → verify PNG output

### Phase 2: Render Pipeline (Week 2)
- [ ] Implement `image/render_queue.rs` — bounded queue with concurrency=2
- [ ] Implement `image/template.rs` — HTML data binding from `MatchRecord`/`LoadoutRecord`
- [ ] Wire up `commands.rs::match_cmd()` to call `ImageService::render_match()`
- [ ] Fallback to embed on any error

### Phase 3: Loadout + Polish (Week 3)
- [ ] Wire up `commands.rs::loadout()` to call `ImageService::render_loadout()`
- [ ] Implement cooldown system (10s per user)
- [ ] Hook into health server: expose render queue stats
- [ ] Error message parity with TS bot
- [ ] Performance testing: 2 concurrent renders, verify <8s timeout

### Phase 4: Production (Week 4)
- [ ] Update OVH Docker deployment config
- [ ] Monitor memory usage (2× Chromium instances)
- [ ] Set up render retry + browser recovery logic
- [ ] Validate pixel parity: diff Rust output against TS bot output for same match

---

## 7. Decision Log

| Decision | Rationale |
|---|---|
| **Approach C over A** | Independent deployment, no cross-service coupling. TS bot is being phased out. |
| **Approach C over B** | Pixel-identical output required. CSS-heavy templates impractical in 2D canvas. |
| **Concurrency = 2** | Matches TS bot, keeps memory ~300MB for Chromium. Higher concurrency risks OOM. |
| **Queue limit = 20** | Same as TS bot. 20 pending × 4s avg = ~80s backlog before rejection. |
| **Timeout = 8000ms** | Matches TS bot. Healthy renders complete in 2–4s; 8s allows for slow container. |
| **Cooldown = 10s per user** | Matches TS bot. Prevents queue spam from a single user. |
| **Fallback to embed on error** | Guarantees /match and /loadout always return *something*, even during render failures. |
