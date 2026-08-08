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

use std::os::windows::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex as StdMutex;
use std::sync::Arc;
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
const BROWSER_START_TIMEOUT: Duration = Duration::from_secs(15);

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
        Self {
            template_engine,
            config,
            browser_process: StdMutex::new(None),
            ws_url: StdMutex::new(None),
            cdp_client: StdMutex::new(None),
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
        self.render_element(&document, "#scoreboard", MATCH_SCALE).await
    }

    /// Render a loadout card JSON record to PNG bytes.
    pub async fn render_loadout(
        &self,
        record: &Value,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let document = self.template_engine.loadout_document(record);
        self.render_element(&document, "#loadout", LOADOUT_SCALE).await
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

        // Check if we already have a child process (might just need reconnect)
        // Use ref pattern to get mutable access for try_wait
        let needs_spawn = {
            let proc = self.browser_process.lock().unwrap();
            match proc.as_ref() {
                Some(child) => {
                    // try_wait requires &mut Child, but we have &Child behind a RefGuard.
                    // Work around by checking pid existence instead.
                    child.id() != 0
                }
                None => true,
            }
        };

        if !needs_spawn {
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
            let ws_url = format!("http://127.0.0.1:{}/json/version", port);
            let json_str = reqwest::get(&ws_url)
                .await
                .map_err(|e| format!("Failed to fetch browser JSON: {}", e))?
                .text()
                .await
                .map_err(|e| format!("Failed to parse browser JSON: {}", e))?;

            let json: Value = serde_json::from_str(&json_str)
                .map_err(|e| format!("Browser JSON parse error: {}", e))?;

            let debug_url = json["webSocketDebuggerUrl"]
                .as_str()
                .ok_or("No webSocketDebuggerUrl in browser JSON")?;

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
            .stderr(Stdio::null())
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
        self.config.debug_port
    }

    /// Wait for the browser debug port to accept connections.
    async fn wait_for_browser_ready(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let port = self.discover_debug_port();
        let deadline = Instant::now() + BROWSER_START_TIMEOUT;

        loop {
            if Instant::now() > deadline {
                return Err(format!(
                    "Browser debug port {} did not become ready in {:?}",
                    port, BROWSER_START_TIMEOUT
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
        let client = self.ensure_browser().await?;

        // Navigate to the document
        let html_url = format!("data:text/html,{}", url_escape(document_html));
        client.send("Page.navigate", json!({
            "url": &html_url,
        })).await?;

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
// Helper: find default Chromium/chrome executable
// ---------------------------------------------------------------------------

fn default_chromium_path() -> String {
    // Check Playwright-installed Chromium
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        let root = format!("{}/ms-playwright", localappdata);
        if let Ok(entries) = std::fs::read_dir(&root) {
            let mut shells: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("chromium_headless_shell-"))
                .collect();
            shells.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
            for entry in shells {
                let exe = entry.path().join("chrome-headless-shell-win64/chrome-headless-shell.exe");
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

/// URL-escape a string for data URIs.
fn url_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' | '@'
            | '?' | '#' | '=' | '&' | '$' | '!' | '+' => {
                result.push(c);
            }
            ' ' => result.push_str("%20"),
            '\n' => result.push_str("%0A"),
            '\r' => result.push_str("%0D"),
            _ => {
                for byte in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", *byte));
                }
            }
        }
    }
    result
}
