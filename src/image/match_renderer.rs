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

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::image::cdp_client::CdpClient;
use crate::image::template::TemplateEngine;

/// Page dimensions for rendering.
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

/// Device scale factor for match scoreboards (1280 × 1.6 = 2048px).
const MATCH_SCALE: f64 = 1.6;

/// Device scale factor for loadout cards (1280 × 1.0 = 1280px).
const LOADOUT_SCALE: f64 = 1.0;

/// Template version for cache invalidation keys.
const TEMPLATE_VERSION: u32 = 14;

/// Loadout template version for cache invalidation keys.
const LOADOUT_TEMPLATE_VERSION: u32 = 9;

/// Maximum time to wait for the browser debug port to appear.
const BROWSER_START_TIMEOUT: Duration = Duration::from_secs(8);

/// Configuration for the match renderer.
#[derive(Debug, Clone)]
pub struct MatchRendererConfig {
    /// Path to the Chromium/chrome executable.
    pub chromium_path: String,
    /// Remote debugging port (0 = auto-select).
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
pub struct MatchRenderer {
    /// Template engine for data binding.
    template_engine: TemplateEngine,
    /// Renderer configuration.
    config: MatchRendererConfig,
    /// Browser child process (Some while Chromium is running).
    browser_process: StdMutex<Option<Child>>,
    /// Browser WebSocket URL for CDP (set after spawn + discovery).
    ws_url: StdMutex<Option<String>>,
    /// Current CDP client (wrapped in Arc for cloning across tasks).
    cdp_client: StdMutex<Option<Arc<CdpClient>>>,
    /// Actual debug port in use (set after spawn; 0 until spawned).
    active_port: StdMutex<u16>,
    /// Serializes access to the single shared Chromium page so concurrent
    /// renders can't corrupt each other's DOM/viewport state.
    render_lock: tokio::sync::Mutex<()>,
}

impl MatchRenderer {
    /// Template version for cache key generation.
    pub fn template_version(&self) -> u32 {
        TEMPLATE_VERSION
    }

    /// Loadout template version for cache key generation.
    pub fn loadout_template_version(&self) -> u32 {
        LOADOUT_TEMPLATE_VERSION
    }

    /// Create a new renderer with the given template engine and config.
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
    pub async fn warm(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.ensure_browser().await?;
        Ok(())
    }

    /// Render a match scoreboard JSON record to PNG bytes.
    pub async fn render(
        &self,
        record: &Value,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let document = self.template_engine.match_document(record);
        self.render_element(&document, "#scoreboard", MATCH_SCALE)
            .await
    }

    /// Render a loadout card JSON record to PNG bytes.
    pub async fn render_loadout(
        &self,
        record: &Value,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let document = self.template_engine.loadout_document(record);
        self.render_element(&document, "#loadout", LOADOUT_SCALE)
            .await
    }

    /// Close the browser and release all resources.
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
    fn discover_debug_port(&self) -> u16 {
        *self.active_port.lock().unwrap()
    }

    /// Wait for the browser debug port to accept connections.
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
    async fn connect_client(
        &self,
        ws_url: String,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = CdpClient::connect(ws_url).await?;
        let mut guard = self.cdp_client.lock().unwrap();
        *guard = Some(Arc::new(client));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Render pipeline
    // -----------------------------------------------------------------------

    /// Full render pipeline: inject HTML, wait for assets, screenshot element.
    async fn render_element(
        &self,
        document_html: &str,
        selector: &str,
        scale: f64,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Serialize all CDP page work onto the single shared page.
        let _render_guard = self.render_lock.lock().await;

        let client = self.ensure_browser().await?;

        // Navigate to the document. Base64-encode the HTML into a data URI so
        // `#` (present in CSS hex colors) isn't interpreted as a URL fragment.
        let html_url = html_data_uri(document_html);
        client
            .send(
                "Page.navigate",
                json!({
                    "url": &html_url,
                }),
            )
            .await?;

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
                await img.decode().catch(() => {});
            }));
            await new Promise((resolve) => {
                requestAnimationFrame(() => requestAnimationFrame(resolve));
            });
        })()"#;

        let _ = client.execute(wait_script).await;

        // Screenshot the element
        client.screenshot_element(selector).await
    }
}

// ---------------------------------------------------------------------------
// Helpers: browser page-target discovery & URL escaping
// ---------------------------------------------------------------------------

/// Resolve the CDP WebSocket URL of a *page* target (not the browser-level
/// /json/version target). Page.*, Runtime.* and Emulation.* commands only work
/// on a page target, so we reuse an existing tab or create a fresh about:blank
/// tab via `PUT /json/new`, then connect to that tab's webSocketDebuggerUrl.
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
                if ty == "page" {
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

/// Build a base64 `data:text/html;base64,...` URI from raw HTML. Base64-encoding
/// (vs percent-encoding) guarantees `#` from CSS hex colors is never parsed as a
/// URL fragment, which would truncate the document.
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
    use super::html_data_uri;

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
    fn html_data_uri_encodes_arbitrary_bytes() {
        let uri = html_data_uri("héllo ← wörld");
        assert!(uri.starts_with("data:text/html;base64,"));
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
        // `dev_defaults()` uses repo-root-relative paths; the test harness runs
        // from the crate dir, so resolve them to the repo root explicitly.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root");
        let cfg = TemplateConfig {
            match_template_path: root
                .join("dev/prototypes/match-result-scoreboard.html")
                .to_string_lossy()
                .into_owned(),
            loadout_template_path: root
                .join("dev/prototypes/loadout-card-layout.html")
                .to_string_lossy()
                .into_owned(),
            cheater_pattern_path: root
                .join("dev/prototypes/cheater-police-line.svg")
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
