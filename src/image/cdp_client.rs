//! Chrome DevTools Protocol WebSocket client — communicates with headless Chromium.

use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone)]
pub struct CdpResponse {
    pub method: String,
    pub params: Value,
    pub error: Option<Value>,
    pub id: Option<u64>,
}

struct PendingRequest {
    tx: oneshot::Sender<CdpResponse>,
}

pub struct CdpClient {
    next_id: Mutex<u64>,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    sender: mpsc::Sender<String>,
}

impl CdpClient {
    pub async fn connect(ws_url: String) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await
            .map_err(|e| format!("CDP WebSocket connection failed: {}", e))?;
        Self::from_stream(ws_stream)
    }

    fn from_stream(stream: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>)
        -> Result<Self, Box<dyn std::error::Error + Send + Sync>>
    {
        let (msg_tx, msg_rx) = mpsc::channel::<String>(64);
        let pending: Arc<Mutex<HashMap<u64, PendingRequest>>> = Arc::new(Mutex::new(HashMap::new()));

        // tokio_tungstenite's split() returns (write, read), NOT (read, write)
        // See tokio_tungstenite docs: split() → (SplitSink, SplitStream)
        let (write_half, read_half) = stream.split();

        // Reader task: consume incoming messages from the stream
        let pending_clone = Arc::clone(&pending);
        tokio::spawn(async move {
            let mut stream = read_half;
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(resp) = parse_cdp_message(&text) {
                            if let Some(id) = resp.id {
                                let mut guard = pending_clone.lock().await;
                                if let Some(pending_req) = guard.remove(&id) {
                                    let response = CdpResponse {
                                        method: resp.method,
                                        params: resp.params,
                                        error: resp.error,
                                        id: Some(id),
                                    };
                                    let _ = pending_req.tx.send(response);
                                }
                            }
                        }
                    }
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(_)) | Ok(Message::Binary(_)) | Ok(Message::Frame(_)) => break,
                    Err(_) => break,
                }
            }
        });

        // Writer task: send outgoing messages to the sink
        tokio::spawn(async move {
            let mut sink = write_half;
            let mut input = msg_rx;
            while let Some(text) = input.recv().await {
                use futures_util::SinkExt;
                use tokio_tungstenite::tungstenite::protocol::Message;
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            next_id: Mutex::new(0),
            pending: Mutex::new(HashMap::new()),
            sender: msg_tx,
        })
    }

    pub async fn send(
        &self,
        method: &str,
        params: Value,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        let id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            *next = id.wrapping_add(1);
            id
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, PendingRequest { tx });
        let cmd = json!({ "id": id, "method": method, "params": params });
        if self.sender.send(cmd.to_string()).await.is_err() {
            self.pending.lock().await.remove(&id);
            return Err("Failed to send CDP command".into());
        }
        rx.await
            .map_err(|e| format!("Request channel closed: {}", e))
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })
    }

    pub async fn send_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        tokio::time::timeout(timeout, self.send(method, params))
            .await
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> { "CDP command timed out".into() })?
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })
    }

    pub async fn navigate(&self, url: &str)
        -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send("Page.navigate", json!({ "url": url })).await
    }

    pub async fn evaluate(&self, expression: &str)
        -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self.send("Runtime.evaluate", json!({
            "expression": expression, "returnByValue": true, "awaitPromise": true,
        })).await?;
        match &resp.error {
            Some(err) => Err(format!("Runtime.evaluate error: {}", err).into()),
            None => {
                let result = resp.params.get("result").and_then(|r| r.get("value")).cloned();
                result.ok_or_else(|| "No result value in evaluation response".into())
            }
        }
    }

    pub async fn execute(&self, expression: &str)
        -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send("Runtime.evaluate", json!({
            "expression": expression, "returnByValue": true,
        })).await
    }

    pub async fn set_device_scale_factor(&self, factor: f64, width: u32, height: u32)
        -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send("Emulation.setDeviceMetricsOverride", json!({
            "width": width, "height": height, "deviceScaleFactor": factor, "mobile": false,
        })).await
    }

    pub async fn screenshot(&self)
        -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self.send("Page.captureScreenshot", json!({ "format": "png" })).await?;
        match &resp.error {
            Some(err) => Err(format!("Screenshot error: {}", err).into()),
            None => {
                let data = resp.params["data"].as_str().ok_or("No data in screenshot")?;
                Ok(base64_decode(data).map_err(|e| format!("Base64 error: {}", e))?)
            }
        }
    }

    pub async fn screenshot_element(&self, selector: &str)
        -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let escaped = selector.replace('\\', "\\\\").replace('"', "\\\"");
        let expr = format!(
            r#"(async () => {{ const el = document.querySelector("{}"); if (!el) return null; const r = el.getBoundingClientRect(); return {{ x: Math.floor(r.x), y: Math.floor(r.y), width: Math.ceil(r.width), height: Math.ceil(r.height) }} }})()"#,
            escaped
        );
        let rect = self.evaluate(&expr).await?;
        if rect.is_null() {
            return Err(format!("Element not found: {}", selector).into());
        }
        let x = rect["x"].as_f64().unwrap_or(0.0);
        let y = rect["y"].as_f64().unwrap_or(0.0);
        let w = rect["width"].as_f64().unwrap_or(1280.0);
        let h = rect["height"].as_f64().unwrap_or(720.0);
        let resp = self.send("Page.captureScreenshot", json!({
            "format": "png",
            "clip": { "x": x, "y": y, "width": w, "height": h, "scale": 1.0 },
        })).await?;
        match &resp.error {
            Some(err) => Err(format!("Screenshot error: {}", err).into()),
            None => {
                let data = resp.params["data"].as_str().ok_or("No data in screenshot")?;
                Ok(base64_decode(data).map_err(|e| format!("Base64 error: {}", e))?)
            }
        }
    }

    pub async fn create_isolated_world(&self, world_name: &str)
        -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send("Page.createIsolatedWorld", json!({
            "frameId": "", "worldName": world_name,
        })).await
    }

    pub async fn close(&self) {
        let _ = self.sender.send(String::new()).await;
    }
}

struct ParsedCdp {
    id: Option<u64>,
    method: String,
    params: Value,
    error: Option<Value>,
}

fn parse_cdp_message(text: &str) -> Result<ParsedCdp, String> {
    let msg: Value = serde_json::from_str(text).map_err(|e| format!("JSON parse error: {}", e))?;
    Ok(ParsedCdp {
        id: msg.get("id").and_then(|v| v.as_u64()),
        method: msg.get("method").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        params: msg.get("params").cloned().unwrap_or(Value::Null),
        error: msg.get("error").cloned(),
    })
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let lookup: [u8; 256] = {
        let mut t = [255u8; 256];
        for i in b'A'..=b'Z' { t[i as usize] = (i - b'A') as u8; }
        for i in b'a'..=b'z' { t[i as usize] = (i - b'a' + 26) as u8; }
        for i in b'0'..=b'9' { t[i as usize] = (i - b'0' + 52) as u8; }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut i = 0;
    while i + 3 < bytes.len() {
        let b0 = lookup[bytes[i] as usize];
        let b1 = lookup[bytes[i + 1] as usize];
        let b2 = lookup[bytes[i + 2] as usize];
        let b3 = lookup[bytes[i + 3] as usize];
        out.push((b0 << 2) | (b1 >> 4));
        out.push((b1 << 4) | (b2 >> 2));
        out.push((b2 << 6) | b3);
        i += 4;
    }
    Ok(out)
}
