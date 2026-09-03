//! Browser renderer — spawns headless Chromium, loads templates, screenshots to PNG.
//!
//! Mirrors TS `match-renderer.ts` Puppeteer pipeline using direct CDP communication.
//!
//! # Architecture
//! 1. Spawn headless Chromium with `--remote-debugging-port`
//! 2. Connect CdpClient to the debug WebSocket
//! 3. Create a new page, set viewport, inject HTML template
//! 4. Wait for fonts/images, then screenshot the target element
//! 5. Return PNG bytes
//! refs: none

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::image::cdp_client::CdpClient;
use crate::image::template::TemplateEngine;

/// Page dimensions for rendering.
/// refs: none
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

/// Device scale factor for match scoreboards (1280 × 1.6 = 2048px).
/// refs: none
const MATCH_SCALE: f64 = 1.6;

/// Device scale factor for loadout cards (1280 × 1.0 = 1280px).
/// refs: none
const LOADOUT_SCALE: f64 = 1.0;
static RENDER_DOCUMENT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Template version for cache invalidation keys.
/// refs: none
const TEMPLATE_VERSION: u32 = 18;

/// Loadout template version for cache invalidation keys.
/// refs: none
const LOADOUT_TEMPLATE_VERSION: u32 = 11;

/// Maximum time to wait for the browser debug port to appear.
/// refs: none
const BROWSER_START_TIMEOUT: Duration = Duration::from_secs(8);

const WEB_SCOREBOARD_EXPORT_TIMEOUT: Duration = Duration::from_secs(18);
const WEB_SCOREBOARD_READY_TITLE: &str = "PALADINSCAT_SCOREBOARD_EXPORT_READY";
const WEB_SCOREBOARD_ERROR_TITLE: &str = "PALADINSCAT_SCOREBOARD_EXPORT_ERROR:";
const WEB_SCOREBOARD_BOOTSTRAP: &str = r#"(() => {
  const timer = setInterval(async () => {
    if (window.__paladinscatExportStarted) return;
    const scoreboard = document.querySelector('#browser-scoreboard .scoreboard');
    if (!scoreboard || scoreboard.querySelectorAll('.player-row').length < 10) {
      document.title = 'PALADINSCAT_SCOREBOARD_EXPORT_WAITING';
      return;
    }
    window.__paladinscatExportStarted = true;
    clearInterval(timer);
    document.title = 'PALADINSCAT_SCOREBOARD_EXPORTING';
    try {
      const assetDeadline = performance.now() + 2000;
      while (scoreboard.querySelector('span.talent-icon[role="img"]') && performance.now() < assetDeadline) {
        await new Promise(resolve => requestAnimationFrame(resolve));
      }
      await Promise.race([document.fonts.ready, new Promise(resolve => setTimeout(resolve, 2000))]);
      await Promise.all(Array.from(scoreboard.querySelectorAll('img')).map(image => {
        if (image.complete) return image.decode?.().catch(() => undefined) ?? Promise.resolve();
        return Promise.race([
          new Promise(resolve => {
            image.addEventListener('load', resolve, { once: true });
            image.addEventListener('error', resolve, { once: true });
          }),
          new Promise(resolve => setTimeout(resolve, 2000)),
        ]);
      }));
      scoreboard.setAttribute('data-image-export', 'true');
      const browser = document.querySelector('#browser-scoreboard');
      const viewport = browser?.querySelector('.viewport');
      const controls = browser?.firstElementChild;
      document.documentElement.style.cssText = 'margin:0;width:2048px;height:1152px;overflow:hidden;background:#161618';
      document.body.style.cssText = 'margin:0;width:2048px;height:1152px;overflow:hidden';
      if (browser) browser.style.cssText = 'position:fixed;inset:0;width:2048px;height:1152px;margin:0;padding:0;overflow:hidden';
      if (controls) controls.style.display = 'none';
      if (viewport) viewport.style.cssText = 'width:2048px;max-width:none;transform:none;transform-origin:top left';
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      document.title = 'PALADINSCAT_SCOREBOARD_EXPORT_READY';
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      document.title = 'PALADINSCAT_SCOREBOARD_EXPORT_ERROR:' + message.slice(0, 120);
    }
  }, 100);
  setTimeout(() => clearInterval(timer), 18000);
})()"#;

/// Configuration for the match renderer.
/// refs: none
#[derive(Debug, Clone)]
/// Define MatchRendererConfig.
///
/// Contract: accepts the arguments shown in the signature and returns the documented result; side effects follow the implementation.
///
/// refs: none
pub struct MatchRendererConfig {
    /// Path to the Chromium/chrome executable.
/// refs: none
    pub chromium_path: String,
    /// Remote debugging port (0 = auto-select).
/// refs: none
    pub debug_port: u16,
}

impl Default for MatchRendererConfig {
    fn default() -> Self {
        Self {
            chromium_path: default_chromium_path(),
            debug_port: 0,
        }
    }
}

/// Browser renderer for rendering HTML templates to PNG images via CDP.
///
/// Mirrors TS `MatchRenderer` from `match-renderer.ts`.
/// refs: none
pub struct MatchRenderer {
    /// Template engine for data binding.
/// refs: none
    template_engine: TemplateEngine,
    /// Renderer configuration.
/// refs: none
    config: MatchRendererConfig,
    /// Browser child process (Some while Chromium is running).
/// refs: none
    browser_process: StdMutex<Option<Child>>,
    /// Browser WebSocket URL for CDP (set after spawn + discovery).
/// refs: none
    ws_url: StdMutex<Option<String>>,
    /// Current CDP client (wrapped in Arc for cloning across tasks).
/// refs: none
    cdp_client: StdMutex<Option<Arc<CdpClient>>>,
    /// Actual debug port in use (set after spawn; 0 until spawned).
/// refs: none
    active_port: StdMutex<u16>,
    /// Serializes access to the single shared Chromium page so concurrent
    /// renders can't corrupt each other's DOM/viewport state.
/// refs: none
    render_lock: tokio::sync::Mutex<()>,
}

impl MatchRenderer {
    /// Template version for cache key generation.
    ///
    /// I/O: () -> `u32`
/// refs: none
    pub fn template_version(&self) -> u32 {
        TEMPLATE_VERSION
    }

    /// Loadout template version for cache key generation.
    ///
    /// I/O: () -> `u32`
/// refs: none
    pub fn loadout_template_version(&self) -> u32 {
        LOADOUT_TEMPLATE_VERSION
    }

    /// Create a new renderer with the given template engine and config.
    ///
    /// I/O: `TemplateEngine`, `MatchRendererConfig` -> `MatchRenderer`
/// refs: none
    pub fn new(template_engine: TemplateEngine, config: MatchRendererConfig) -> Self {
        let initial_port = config.debug_port;
        Self {
            template_engine,
            config,
            browser_process: StdMutex::new(None),
            ws_url: StdMutex::new(None),
            cdp_client: StdMutex::new(None),
            active_port: StdMutex::new(initial_port),
            render_lock: tokio::sync::Mutex::new(()),
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Warm up the browser by spawning it and performing a dummy navigation.
    ///
    /// I/O: () -> `Result<(), Box<dyn Error + Send + Sync>>`
/// refs: none
    pub async fn warm(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_browser().await?;
        Ok(())
    }

    /// Render a match scoreboard JSON record to PNG bytes.
    ///
    /// I/O: `&Value` (record) -> `Result<Vec<u8>, Box<dyn Error + Send + Sync>>`
/// refs: none
    pub async fn render(
        &self,
        record: &Value,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let document = self.template_engine.match_document(record);
        self.render_element(&document, "#scoreboard", MATCH_SCALE)
            .await
    }

    /// Render the canonical web scoreboard itself. This is the `/match`
    /// command path, so the Discord PNG shares the web component's data
    /// fallbacks, team markers, markup, and CSS instead of duplicating them.
    ///
    /// I/O: `&str` (url) -> `Result<Vec<u8>, Box<dyn Error + Send + Sync>>`
/// refs: none
    pub async fn render_web_match(
        &self,
        url: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let _render_guard = self.render_lock.lock().await;
        let client = self.ensure_browser().await?;
        client.set_device_scale_factor(1.0, WIDTH, HEIGHT).await?;
        let bootstrap = client
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": WEB_SCOREBOARD_BOOTSTRAP }),
            )
            .await?;
        if let Some(error) = bootstrap.error {
            return Err(format!("Could not install scoreboard exporter: {error}").into());
        }
        let script_id = bootstrap.result["identifier"].as_str().map(str::to_owned);
        // Discord links remain public. Compose config may supply an internal
        // frontend origin for Chromium when the public edge rejects automation.
        // The path/query remain exact and no rewrite occurs without that origin.
        let internal_origin = std::env::var("PALADINSCAT_RENDER_WEB_URL").ok();
        let render_url = internal_render_url(url, internal_origin.as_deref());
        client
            .send("Page.navigate", json!({ "url": render_url }))
            .await?;

        let deadline = Instant::now() + WEB_SCOREBOARD_EXPORT_TIMEOUT;
        let mut last_title = String::new();
        let result = loop {
            if let Some(title) = page_target_title(self.discover_debug_port()).await {
                last_title = title;
            }
            if last_title == WEB_SCOREBOARD_READY_TITLE {
                client.set_device_scale_factor(1.0, 2048, 1152).await?;
                break client.screenshot().await;
            } else if let Some(message) = last_title.strip_prefix(WEB_SCOREBOARD_ERROR_TITLE) {
                break Err(format!("Web scoreboard export failed: {message}").into());
            } else if Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(100)).await
            } else {
                break Err(format!("Web scoreboard did not export ({last_title}): {url}").into());
            }
        };
        if let Some(identifier) = script_id {
            let _ = client
                .send(
                    "Page.removeScriptToEvaluateOnNewDocument",
                    json!({ "identifier": identifier }),
                )
                .await;
        }
        result
    }

    /// Render a loadout card JSON record to PNG bytes.
    ///
    /// I/O: `&Value` (record) -> `Result<Vec<u8>, Box<dyn Error + Send + Sync>>`
/// refs: none
    pub async fn render_loadout(
        &self,
        record: &Value,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let document = self.template_engine.loadout_document(record);
        self.render_element(&document, "#loadout", LOADOUT_SCALE)
            .await
    }

    /// Close the browser and release all resources.
    ///
    /// I/O: `MatchRenderer` (self) -> `()`
/// refs: none
    pub async fn close(&self) {
        {
            let mut proc = self.browser_process.lock().unwrap();
            if let Some(mut child) = proc.take() {
                let _ = child.kill();
            }
        }
        {
            let mut ws = self.ws_url.lock().unwrap();
            *ws = None;
        }
        {
            let mut client = self.cdp_client.lock().unwrap();
            if let Some(c) = client.take() {
                c.close().await;
            }
        }
    }

    /// Recycle the browser after a failure — kill and reset for next render.
    ///
    /// Browser.close() can wait on the same poisoned renderer that caused
    /// the timeout, so recovery deliberately kills the child process and
    /// drops the client. The next render lazily starts a clean browser.
    ///
    /// I/O: `MatchRenderer` (self) -> `()`
/// refs: none
    pub async fn recycle(&self) {
        tracing::info!("Recycling browser…");
        {
            let mut proc = self.browser_process.lock().unwrap();
            if let Some(mut child) = proc.take() {
                let _ = child.kill();
            }
        }
        {
            // Extract the client, drop the guard, then await — std::sync::MutexGuard is !Send
            let maybe_client = self.cdp_client.lock().unwrap().take();
            if let Some(c) = maybe_client {
                c.close().await;
            }
        }
        let mut ws = self.ws_url.lock().unwrap();
        *ws = None;
    }

    // -----------------------------------------------------------------------
    // Browser lifecycle
    // -----------------------------------------------------------------------

    /// Ensure a browser process is running and connected, returning the CDP client as Arc.
/// refs: none
    async fn ensure_browser(
        &self,
    ) -> Result<Arc<CdpClient>, Box<dyn std::error::Error + Send + Sync>> {
        // Fast path: client already exists
        {
            let client = self.cdp_client.lock().unwrap();
            if let Some(c) = client.as_ref() {
                return Ok(Arc::clone(c));
            }
        }

        // Spawn a fresh browser unless we already have a live child process.
        // try_wait requires &mut Child, so check pid existence instead.
        let has_live_child = {
            let proc = self.browser_process.lock().unwrap();
            match proc.as_ref() {
                Some(child) => child.id() != 0,
                None => false,
            }
        };

        if !has_live_child {
            self.spawn_browser()?;
        }

        // Wait for the debug port to accept connections
        self.wait_for_browser_ready().await?;

        // Resolve the ws_url if we haven't already
        let maybe_url = self.ws_url.lock().unwrap().clone();
        if let Some(url) = maybe_url {
            self.connect_client(url).await?;
        } else {
            // Discover the debug port
            let port = self.discover_debug_port();
            let debug_url = resolve_page_ws_url(port).await?;

            // Set the ws_url, then drop the guard before awaiting
            {
                let mut ws = self.ws_url.lock().unwrap();
                *ws = Some(debug_url.to_string());
            }
            self.connect_client(debug_url.to_string()).await?;
        }

        let client = self.cdp_client.lock().unwrap();
        Ok(Arc::clone(client.as_ref().unwrap()))
    }

    /// Spawn headless Chromium with remote debugging enabled.
/// refs: none
    fn spawn_browser(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let debug_port = if self.config.debug_port == 0 {
            let mut port = 9222;
            loop {
                let listener = std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).ok();
                if listener.is_some() {
                    break;
                }
                port += 1;
                if port > 9299 {
                    return Err("No available debug port".into());
                }
            }
            port
        } else {
            self.config.debug_port
        };

        // Record the actual port so wait_for_browser_ready can probe it.
        {
            let mut ap = self.active_port.lock().unwrap();
            *ap = debug_port;
        }

        // Capture Chromium stderr to a temp file so startup failures are diagnosable.
        let stderr_path = std::env::temp_dir().join("paladinscat-chromium-stderr.log");
        let stderr_file = std::fs::File::create(&stderr_path)
            .map_err(|e| format!("Failed to create Chromium stderr log: {}", e))?;

        let child = Command::new(&self.config.chromium_path)
            .args([
                "--headless",
                "--no-sandbox",
                "--disable-setuid-sandbox",
                "--disable-dev-shm-usage",
                "--disable-gpu",
                "--font-render-hinting=medium",
                "--allow-file-access-from-files",
                &format!("--remote-debugging-port={}", debug_port),
                "--remote-debugging-address=127.0.0.1",
                "about:blank",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .map_err(|e| format!("Failed to spawn Chromium: {}", e))?;

        let mut proc = self.browser_process.lock().unwrap();
        *proc = Some(child);
        tracing::info!(
            chromium_path = %self.config.chromium_path,
            debug_port,
            "Spawned headless Chromium"
        );
        Ok(())
    }

    /// Discover the debug port from the config or child process.
/// refs: none
    fn discover_debug_port(&self) -> u16 {
        *self.active_port.lock().unwrap()
    }

    /// Wait for the browser debug port to accept connections.
/// refs: none
    async fn wait_for_browser_ready(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let port = self.discover_debug_port();
        let deadline = Instant::now() + BROWSER_START_TIMEOUT;

        loop {
            if Instant::now() > deadline {
                let stderr_path = std::env::temp_dir().join("paladinscat-chromium-stderr.log");
                let stderr_tail = std::fs::read_to_string(&stderr_path)
                    .map(|s| {
                        let bytes = s.as_bytes();
                        let start = bytes.len().saturating_sub(2000);
                        String::from_utf8_lossy(&bytes[start..]).into_owned()
                    })
                    .unwrap_or_default();
                return Err(format!(
                    "Browser debug port {} did not become ready in {:?}. Chromium stderr: {}",
                    port, BROWSER_START_TIMEOUT, stderr_tail
                )
                .into());
            }

            match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
                Ok(_) => return Ok(()),
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    /// Connect to the browser's CDP WebSocket.
/// refs: none
    async fn connect_client(
        &self,
        ws_url: String,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = CdpClient::connect(ws_url).await?;
        for domain in ["Page.enable", "Runtime.enable"] {
            let response = client.send(domain, json!({})).await?;
            if let Some(error) = response.error {
                return Err(format!("Could not initialize CDP domain {domain}: {error}").into());
            }
        }
        let mut guard = self.cdp_client.lock().unwrap();
        *guard = Some(Arc::new(client));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Render pipeline
    // -----------------------------------------------------------------------

    /// Full render pipeline: inject HTML, wait for assets, screenshot element.
/// refs: none
    async fn render_element(
        &self,
        document_html: &str,
        selector: &str,
        scale: f64,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Serialize all CDP page work onto the single shared page.
        let _render_guard = self.render_lock.lock().await;

        let client = self.ensure_browser().await?;

        // Inject the generated document directly into the existing page frame.
        // This avoids base64-encoding an HTML document whose AVIF assets are
        // already base64 data URLs (a large, redundant cold-path copy).
        let render_id = RENDER_DOCUMENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let tagged_html = tagged_render_document(document_html, render_id);
        let frame_tree = client.send("Page.getFrameTree", json!({})).await?;
        if let Some(error) = frame_tree.error {
            return Err(format!("get render frame failed: {error}").into());
        }
        let frame_id = frame_tree.result["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or("render page frame is unavailable")?;
        let injected = client
            .send(
                "Page.setDocumentContent",
                json!({
                    "frameId": frame_id,
                    "html": tagged_html,
                }),
            )
            .await?;
        if let Some(error) = injected.error {
            return Err(format!("inject render document failed: {error}").into());
        }

        // Page.navigate acknowledges the request before the new data document's
        // JavaScript context is guaranteed to be active. Confirm this exact
        // document before evaluating readiness or capturing a previous match.
        let marker = format!(
            r#"document.querySelector('meta[name="paladinscat-render-id"]')?.content === "{render_id}""#
        );
        let navigation_deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if client.evaluate(&marker).await.unwrap_or(Value::Bool(false)) == Value::Bool(true) {
                break;
            }
            if Instant::now() >= navigation_deadline {
                return Err(format!("render document {render_id} did not become active").into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Set viewport and device scale factor
        client.set_device_scale_factor(scale, WIDTH, HEIGHT).await?;

        // Wait for fonts and images to be ready
        let wait_script = r#"(async () => {
            await document.fonts.ready;
            await Promise.all([...document.images].map(async (img) => {
                if (!img.complete) {
                    await new Promise((resolve) => {
                        img.addEventListener('load', resolve, { once: true });
                        img.addEventListener('error', resolve, { once: true });
                    });
                }
                // A completed image with intrinsic dimensions is already ready
                // for layout; calling decode() again is costly for AVIF on the
                // production CPU quota. Keep the fallback for incomplete or
                // failed images so capture never races their first decode.
                if (!img.complete || img.naturalWidth === 0) {
                    await img.decode().catch(() => {});
                }
            }));
            await new Promise((resolve) => {
                requestAnimationFrame(() => requestAnimationFrame(resolve));
            });
        })()"#;

        let wait = client.execute_await(wait_script).await?;
        if let Some(error) = wait.error {
            return Err(format!("Asset readiness evaluation failed: {error}").into());
        }

        // Screenshot the element
        client.screenshot_element(selector).await
    }
}

fn tagged_render_document(document_html: &str, render_id: u64) -> String {
    let marker = format!(r#"<meta name="paladinscat-render-id" content="{render_id}">"#);
    if let Some(insert_at) = document_html.rfind("</body>") {
        let mut tagged = String::with_capacity(document_html.len() + marker.len());
        tagged.push_str(&document_html[..insert_at]);
        tagged.push_str(&marker);
        tagged.push_str(&document_html[insert_at..]);
        tagged
    } else {
        format!("{marker}{document_html}")
    }
}

/// Substitute only paladinscat.com's origin when an internal origin is set.
/// Localhost and other URLs remain untouched for development and tests.
/// refs: none
fn internal_render_url(url: &str, internal_origin: Option<&str>) -> String {
    const PUBLIC_HTTP: &str = "http://paladinscat.com";
    const PUBLIC_HTTPS: &str = "https://paladinscat.com";
    let suffix = url
        .strip_prefix(PUBLIC_HTTPS)
        .or_else(|| url.strip_prefix(PUBLIC_HTTP));
    match (suffix, internal_origin) {
        (Some(path), Some(base))
            if path.is_empty() || path.starts_with('/') || path.starts_with('?') =>
        {
            format!("{}{}", base.trim_end_matches('/'), path)
        }
        _ => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Helpers: browser page-target discovery & URL escaping
// ---------------------------------------------------------------------------

/// Resolve the CDP WebSocket URL of a *page* target (not the browser-level
/// /json/version target). Page.*, Runtime.* and Emulation.* commands only work
/// on a page target, so we reuse an existing tab or create a fresh about:blank
/// tab via `PUT /json/new`, then connect to that tab's webSocketDebuggerUrl.
/// refs: PUT /json/new`,
async fn resolve_page_ws_url(
    port: u16,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let base = format!("http://127.0.0.1:{}", port);

    // 1) Look for an existing page target.
    let list_url = format!("{}/json/list", base);
    let list_str = reqwest::get(&list_url)
        .await
        .map_err(|e| format!("Failed to fetch /json/list: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read /json/list: {}", e))?;

    if let Ok(list) = serde_json::from_str::<Value>(&list_str) {
        if let Some(targets) = list.as_array() {
            for t in targets {
                let ty = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let url = t.get("url").and_then(|v| v.as_str()).unwrap_or("");
                // Debian Chromium opens a special chrome://newtab target by
                // default. It can navigate visually while Runtime.evaluate
                // remains stuck on its browser-UI execution context.
                if ty == "page" && !url.starts_with("chrome://") {
                    if let Some(ws) = t.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        return Ok(ws.to_string());
                    }
                }
            }
        }
    }

    // 2) No page target — create a fresh about:blank tab.
    let new_url = format!("{}/json/new?about:blank", base);
    let resp = reqwest::Client::new()
        .put(&new_url)
        .send()
        .await
        .map_err(|e| format!("Failed to create page via /json/new: {}", e))?;
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read /json/new response: {}", e))?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Failed to parse /json/new response: {}", e))?;
    let ws = json["webSocketDebuggerUrl"]
        .as_str()
        .ok_or("No webSocketDebuggerUrl in /json/new response")?;
    Ok(ws.to_string())
}

async fn page_target_title(port: u16) -> Option<String> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let Ok(response) = reqwest::get(url).await else {
        return None;
    };
    let Ok(targets) = response.json::<Value>().await else {
        return None;
    };
    targets.as_array()?.iter().find_map(|target| {
        (target["type"] == "page")
            .then(|| target["title"].as_str().map(str::to_owned))
            .flatten()
    })
}

/// Build a base64 `data:text/html;base64,...` URI from raw HTML. Base64-encoding
/// (vs percent-encoding) guarantees `#` from CSS hex colors is never parsed as a
/// URL fragment, which would truncate the document.
/// refs: none
fn html_data_uri(document_html: &str) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(document_html.as_bytes());
    format!("data:text/html;base64,{}", b64)
}

fn default_chromium_path() -> String {
    // Check Playwright-installed Chromium
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        let root = format!("{}/ms-playwright", localappdata);
        if let Ok(entries) = std::fs::read_dir(&root) {
            let mut shells: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("chromium_headless_shell-")
                })
                .collect();
            shells.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
            for entry in shells {
                let exe = entry
                    .path()
                    .join("chrome-headless-shell-win64/chrome-headless-shell.exe");
                if exe.exists() {
                    return exe.to_string_lossy().to_string();
                }
            }
        }
    }

    // Check system browsers
    let edge = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
    let chrome = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
    if std::path::Path::new(edge).exists() {
        return edge.to_string();
    }
    if std::path::Path::new(chrome).exists() {
        return chrome.to_string();
    }

    "/usr/bin/chromium-browser".into()
}

#[cfg(test)]
mod tests {
    use super::{html_data_uri, internal_render_url, tagged_render_document};
    use std::time::Instant;

    #[test]
    fn html_data_uri_is_base64_and_preserves_css_hex_colors() {
        let html = "<style>.x{color:#123456;background:#abcdef}</style><div>hi</div>";
        let uri = html_data_uri(html);
        assert!(uri.starts_with("data:text/html;base64,"), "got: {}", uri);

        use base64::Engine as _;
        let payload = &uri["data:text/html;base64,".len()..];
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        let decoded = String::from_utf8(decoded).unwrap();
        // The `#` hex colors must round-trip unchanged (no URL-fragment truncation).
        assert_eq!(decoded, html);
        assert!(decoded.contains("#123456"));
        assert!(decoded.contains("#abcdef"));
        // No raw '#' appears in the URI itself.
        assert!(!payload.contains('#'));
    }

    #[test]
    fn canonical_web_export_uses_internal_frontend_only_for_public_origin() {
        assert_eq!(
            internal_render_url(
                "https://paladinscat.com/matches/1281335238?x=1",
                Some("http://frontend:3000")
            ),
            "http://frontend:3000/matches/1281335238?x=1"
        );
        assert_eq!(
            internal_render_url(
                "http://localhost:3000/matches/1281335238",
                Some("http://frontend:3000")
            ),
            "http://localhost:3000/matches/1281335238"
        );
        assert_eq!(
            internal_render_url("https://paladinscat.com/matches/1281335238", None),
            "https://paladinscat.com/matches/1281335238"
        );
    }

    #[test]
    fn html_data_uri_encodes_arbitrary_bytes() {
        let uri = html_data_uri("héllo ← wörld");
        assert!(uri.starts_with("data:text/html;base64,"));
    }

    #[test]
    fn render_documents_receive_unique_navigation_markers() {
        let html = "<html><head></head><body>match</body></html>";
        let first = tagged_render_document(html, 41);
        let second = tagged_render_document(html, 42);
        assert!(first.contains(r#"name="paladinscat-render-id" content="41""#));
        assert!(second.contains(r#"name="paladinscat-render-id" content="42""#));
        assert_ne!(first, second);
    }

    /// Returns true when the caller opted into the real-browser integration test
    /// by setting PALADINSCAT_RENDER_IT=1. Enabled via an env var so the default
    /// suite stays hermetic; set it in the container render smoke test.
    fn integration_enabled() -> bool {
        std::env::var("PALADINSCAT_RENDER_IT")
            .map(|v| v == "1")
            .unwrap_or(false)
    }

    fn chromium_path() -> String {
        std::env::var("CHROME_PATH").unwrap_or_else(|_| super::default_chromium_path())
    }

    fn test_renderer() -> crate::image::match_renderer::MatchRenderer {
        use crate::image::match_renderer::MatchRendererConfig;
        use crate::image::template::{TemplateConfig, TemplateEngine};
        let bot_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let frontend_root = bot_root
            .parent()
            .expect("workspace root")
            .join("paladinscat-frontend");
        let cfg = TemplateConfig {
            match_template_path: bot_root
                .join("assets/templates/match-result-scoreboard.html")
                .to_string_lossy()
                .into_owned(),
            canonical_match_css_path: Some(
                frontend_root
                    .join("app/globals.css")
                    .to_string_lossy()
                    .into_owned(),
            ),
            loadout_template_path: bot_root
                .join("assets/templates/loadout-card-layout.html")
                .to_string_lossy()
                .into_owned(),
            cheater_pattern_path: bot_root
                .join("assets/templates/cheater-police-line.svg")
                .to_string_lossy()
                .into_owned(),
            asset_root_path: frontend_root
                .join("public/images")
                .to_string_lossy()
                .into_owned(),
        };
        let te = TemplateEngine::load(&cfg).unwrap();
        crate::image::match_renderer::MatchRenderer::new(
            te,
            MatchRendererConfig {
                chromium_path: chromium_path(),
                debug_port: 0,
            },
        )
    }

    #[tokio::test]
    async fn chromium_integration_render_produces_valid_png() {
        if !integration_enabled() {
            return;
        }
        let renderer = test_renderer();
        // Self-contained doc with a real id element — validates the full CDP
        // pipeline (page-target connect -> navigate -> evaluate -> screenshot ->
        // PNG decode) independent of the Ruby-idea template's own selector setup.
        let doc = r#"<!doctype html><html><head><meta charset="utf-8"/>
<style>#scoreboard{width:600px;height:200px;background:#10151c;color:#fff;font:600 24px sans-serif;display:flex;align-items:center;justify-content:center;border:2px solid #2a3340;color:#ff6b6b}</style>
</head><body><div id="scoreboard">MATCH 1281311346</div></body></html>"#;
        let png = renderer
            .render_element(doc, "#scoreboard", 1.0)
            .await
            .expect("render");
        const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
        assert!(png.starts_with(SIG), "output must start with PNG signature");
        // Round-trips through the crate decoder (valid PNG header + body).
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let decoded = crate::image::cdp_client::decode_base64_png(&b64).expect("decodable PNG");
        assert_eq!(decoded.len(), png.len());
    }

    #[tokio::test]
    async fn chromium_integration_awaits_asset_decode_before_capture() {
        if !integration_enabled() {
            return;
        }
        let renderer = test_renderer();
        let doc = r#"<!doctype html><html><head><style>
#scoreboard{width:200px;height:100px;background:#100000}.ready #scoreboard{background:#00d070}
</style></head><body><div id="scoreboard"></div><img src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'/%3E"><script>
document.querySelector('img').decode=()=>new Promise(resolve=>setTimeout(()=>{document.documentElement.classList.add('ready');resolve()},350));
</script></body></html>"#;
        let png = renderer
            .render_element(doc, "#scoreboard", 1.0)
            .await
            .expect("render after delayed decode");
        let image = image::load_from_memory(&png).expect("decode PNG").to_rgb8();
        let center = image.get_pixel(100, 50).0;
        assert!(
            center[1] > 150 && center[0] < 50,
            "capture occurred before image decode: {center:?}"
        );
        renderer.close().await;
    }

    #[tokio::test]
    async fn chromium_integration_scoreboard_is_styled() {
        if !integration_enabled() {
            return;
        }
        let renderer = test_renderer();
        let players: Vec<serde_json::Value> = (0..10)
            .map(|i| {
                serde_json::json!({
                    "player_id": format!("p{i}"),
                    "player_name": format!("Player{i}"),
                    "champion_name": if i % 2 == 0 { "Androxus" } else { "Fernando" },
                    "task_force": if i < 5 { 1 } else { 2 },
                    "final_match_level": 50 + i,
                    "queue_elo": 1500 + i * 10,
                    "kbm_tier": 15,
                    "kills": 8,
                    "deaths": 4,
                    "assists": 12,
                    "gold_earned": 10000,
                    "objective_assists": 120,
                    "damage_done_physical": 60000,
                    "damage_taken": 45000,
                    "damage_mitigated": 20000,
                    "healing": 15000
                })
            })
            .collect();
        let record = serde_json::json!({
            "match": {"match_id": 1281311346u64, "duration_seconds": 812, "region": "NA",
                "map": "Ranked Warder's Gate", "queue_id": 486, "winning_task_force": 1,
                "team1_score": 4, "team2_score": 2, "entry_datetime": "2026-08-08T12:00:00Z"},
            "players": players, "bans": [], "facts": []
        });
        let png = renderer
            .render(&record)
            .await
            .expect("render styled scoreboard");
        let image = image::load_from_memory(&png).expect("decode PNG").to_rgb8();
        let white = image
            .pixels()
            .filter(|p| p.0.iter().all(|channel| *channel > 245))
            .count();
        let white_ratio = white as f64 / (image.width() as f64 * image.height() as f64);
        assert!(
            white_ratio < 0.25,
            "scoreboard is unexpectedly white: {:.1}%",
            white_ratio * 100.0
        );
        renderer.close().await;
    }

    #[tokio::test]
    async fn chromium_integration_loadout_is_data_bound_and_styled() {
        if !integration_enabled() {
            return;
        }
        let renderer = test_renderer();
        let record = serde_json::json!({
            "player": {"id": "16706730", "name": "NabiCookTV"},
            "loadout": {
                "id": "grover-banner-regression",
                "champion_id": 2254,
                "champion_name": "Grover",
                "loadout_name": "Banner Regression",
                "card_ids": [25385, 13391, 13414, 15068, 13411],
                "card_levels": [5, 5, 1, 3, 1]
            }
        });
        let png = renderer
            .render_loadout(&record)
            .await
            .expect("render data-bound loadout");
        if let Ok(path) = std::env::var("LOADOUT_PNG_OUT") {
            std::fs::write(path, &png).expect("write LOADOUT_PNG_OUT");
        }
        let image = image::load_from_memory(&png).expect("decode PNG").to_rgb8();
        assert_eq!((image.width(), image.height()), (1280, 720));
        let near_white = image
            .pixels()
            .filter(|pixel| pixel.0.iter().all(|channel| *channel > 245))
            .count();
        let white_ratio = near_white as f64 / (image.width() as f64 * image.height() as f64);
        assert!(
            white_ratio < 0.25,
            "loadout is unexpectedly white: {white_ratio:.1}"
        );
        let client = renderer
            .cdp_client
            .lock()
            .unwrap()
            .clone()
            .expect("renderer CDP client");
        let longest_title = client
            .evaluate(
                r#"(() => {
                    const title = [...document.querySelectorAll('.loadout-card h2')]
                        .find(node => node.textContent === 'Unexpected Complications');
                    if (!title) return null;
                    const style = getComputedStyle(title);
                    return {
                        fits: title.scrollWidth <= title.clientWidth,
                        fontSize: style.fontSize,
                        fontWeight: style.fontWeight
                    };
                })()"#,
            )
            .await
            .expect("measure longest card title");
        assert_eq!(longest_title["fits"], true);
        assert_eq!(longest_title["fontSize"], "13px");
        assert_eq!(longest_title["fontWeight"], "800");
        for card_index in 0..5 {
            let left = 16 + card_index * 250;
            let mut colored = 0;
            for x in left + 25..left + 220 {
                for y in 290..410 {
                    if image.get_pixel(x, y).0.iter().any(|channel| *channel > 45) {
                        colored += 1;
                    }
                }
            }
            assert!(
                colored > 4_000,
                "loadout card {card_index} artwork is blank ({colored} colored pixels)"
            );
        }
        renderer.close().await;
    }

    #[tokio::test]
    async fn chromium_integration_real_match_from_env_is_styled() {
        if !integration_enabled() {
            return;
        }
        let Ok(path) = std::env::var("MATCH_JSON") else {
            return;
        };
        let payload: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).expect("read MATCH_JSON"))
                .expect("parse MATCH_JSON");
        let record = payload
            .get("matches")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .unwrap_or(&payload);
        let renderer = test_renderer();
        let document = renderer.template_engine.match_document(&record);
        let expected_player_assets = record
            .get("players")
            .and_then(serde_json::Value::as_array)
            .map_or(1, |players| players.len().max(1));
        assert!(
            document.matches("data:image/avif;base64,").count() >= expected_player_assets,
            "match document must embed player assets before PNG capture"
        );
        let cold_started = Instant::now();
        let png = renderer
            .render(record)
            .await
            .expect("render real scoreboard");
        let cold_elapsed = cold_started.elapsed();
        let warm_started = Instant::now();
        let warm_png = renderer
            .render(record)
            .await
            .expect("render warm real scoreboard");
        let warm_elapsed = warm_started.elapsed();
        println!(
            "local scoreboard cold={}ms warm={}ms",
            cold_elapsed.as_millis(),
            warm_elapsed.as_millis()
        );
        let warm_image = image::load_from_memory(&warm_png).expect("decode warm PNG");
        assert_eq!((warm_image.width(), warm_image.height()), (2048, 1152));
        if let Ok(path) = std::env::var("MATCH_PNG_OUT") {
            std::fs::write(path, &png).expect("write MATCH_PNG_OUT");
        }
        let image = image::load_from_memory(&png).expect("decode PNG").to_rgb8();
        let white = image
            .pixels()
            .filter(|p| p.0.iter().all(|channel| *channel > 245))
            .count();
        let white_ratio = white as f64 / (image.width() as f64 * image.height() as f64);
        println!("real scoreboard white ratio: {:.1}%", white_ratio * 100.0);
        assert!(
            white_ratio < 0.25,
            "real scoreboard is unexpectedly white: {:.1}%",
            white_ratio * 100.0
        );
        renderer.close().await;
    }

    #[tokio::test]
    async fn chromium_integration_consecutive_api_matches_are_distinct() {
        if !integration_enabled() {
            return;
        }
        let Ok(base) = std::env::var("PALADINSCAT_MATCH_API_URL") else {
            return;
        };
        let Ok(ids) = std::env::var("PALADINSCAT_MATCH_IDS") else {
            return;
        };
        let mut ids = ids.split(',').map(str::trim).filter(|id| !id.is_empty());
        let first_id = ids.next().expect("first match id");
        let second_id = ids.next().expect("second match id");
        let api = crate::api::ApiClient::new(&base, None);
        let (first, second) = tokio::join!(api.match_info(first_id), api.match_info(second_id));
        let first = first.expect("load first match");
        let second = second.expect("load second match");
        let renderer = test_renderer();
        let started = Instant::now();
        let first_png = renderer.render(&first).await.expect("render first match");
        let first_elapsed = started.elapsed();
        let started = Instant::now();
        let second_png = renderer.render(&second).await.expect("render second match");
        println!(
            "consecutive match renders first={}ms second={}ms",
            first_elapsed.as_millis(),
            started.elapsed().as_millis()
        );
        assert_ne!(
            first_png, second_png,
            "consecutive matches reused one image"
        );
        renderer.close().await;
    }

    #[tokio::test]
    async fn chromium_integration_api_record_renders_isolated_scoreboard() {
        if !integration_enabled() {
            return;
        }
        let Ok(base) = std::env::var("PALADINSCAT_MATCH_API_URL") else {
            return;
        };
        let match_id =
            std::env::var("PALADINSCAT_MATCH_ID").unwrap_or_else(|_| "1281311346".to_string());
        let api = crate::api::ApiClient::new(&base, None);
        let record = api.match_info(&match_id).await.expect("load MatchRecord");
        assert_eq!(record["players"].as_array().map(Vec::len), Some(10));
        assert!(record["match"].is_object());
        assert!(record["bans"].is_array());
        let renderer = test_renderer();
        let document = renderer.template_engine.match_document(&record);
        assert!(
            document.matches("data:image/avif;base64,").count() >= 10,
            "match document must embed AVIF assets before PNG capture"
        );
        let png = renderer
            .render(&record)
            .await
            .expect("render isolated scoreboard");
        if let Ok(path) = std::env::var("MATCH_PNG_OUT") {
            std::fs::write(path, &png).expect("write MATCH_PNG_OUT");
        }
        let image = image::load_from_memory(&png).expect("decode PNG");
        assert_eq!((image.width(), image.height()), (2048, 1152));
        renderer.close().await;
    }

    #[tokio::test]
    async fn chromium_integration_web_scoreboard_is_canonical() {
        if !integration_enabled() {
            return;
        }
        let Ok(url) = std::env::var("PALADINSCAT_WEB_MATCH_URL") else {
            return;
        };
        let renderer = test_renderer();
        let cold_started = Instant::now();
        let png = renderer
            .render_web_match(&url)
            .await
            .expect("render web scoreboard");
        let cold_elapsed = cold_started.elapsed();
        let warm_started = Instant::now();
        let warm_png = renderer
            .render_web_match(&url)
            .await
            .expect("render warm web scoreboard");
        println!(
            "canonical scoreboard cold={}ms warm={}ms",
            cold_elapsed.as_millis(),
            warm_started.elapsed().as_millis()
        );
        if let Ok(path) = std::env::var("MATCH_PNG_OUT") {
            std::fs::write(path, &png).expect("write MATCH_PNG_OUT");
        }
        let image = image::load_from_memory(&png).expect("decode PNG");
        let dimensions = (image.width(), image.height());
        let warm_dimensions = image::load_from_memory(&warm_png)
            .map(|image| (image.width(), image.height()))
            .expect("decode warm PNG");
        renderer.close().await;
        assert_eq!(dimensions, (2048, 1152));
        assert_eq!(warm_dimensions, (2048, 1152));
    }

    #[tokio::test]
    async fn concurrent_renders_do_not_corrupt_page_state() {
        if !integration_enabled() {
            return;
        }
        let renderer = std::sync::Arc::new(test_renderer());
        let doc = |kills: u64| -> String {
            format!(
                r#"<!doctype html><html><head><meta charset="utf-8"/>
<style>#scoreboard{{width:400px;height:120px;background:#10151c;color:#fff;font:700 28px sans-serif}}</style>
</head><body><div id="scoreboard">KILLS {}</div></body></html>"#,
                kills
            )
        };
        let mut handles = Vec::new();
        for i in 0..4u64 {
            let r = std::sync::Arc::clone(&renderer);
            handles.push(tokio::spawn(async move {
                let html = doc(i);
                let png = r
                    .render_element(&html, "#scoreboard", 1.0)
                    .await
                    .expect("render");
                let sig: &[u8] = b"\x89PNG\r\n\x1a\n";
                assert!(png.starts_with(sig), "render {} produced non-PNG", i);
                png.len()
            }));
        }
        for (i, h) in handles.into_iter().enumerate() {
            let len = h.await.unwrap();
            assert!(len > 100, "render {} produced suspiciously small image", i);
        }
    }
}
