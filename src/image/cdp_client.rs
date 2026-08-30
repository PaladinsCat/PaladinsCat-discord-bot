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
    pub result: Value,
    pub error: Option<Value>,
    pub id: Option<u64>,
}

struct PendingRequest {
    tx: oneshot::Sender<CdpResponse>,
}

pub struct CdpClient {
    next_id: Mutex<u64>,
    // Shared with the reader task so `send()` inserts into the exact map
    // the reader removes responses from.
    pending: Arc<Mutex<HashMap<u64, PendingRequest>>>,
    sender: mpsc::Sender<String>,
}

impl CdpClient {
    pub async fn connect(ws_url: String) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| format!("CDP WebSocket connection failed: {}", e))?;
        Self::from_stream(ws_stream)
    }

    fn from_stream(
        stream: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (msg_tx, msg_rx) = mpsc::channel::<String>(64);
        let pending: Arc<Mutex<HashMap<u64, PendingRequest>>> =
            Arc::new(Mutex::new(HashMap::new()));

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
                                        result: resp.result,
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
            // Share the SAME Arc used by the reader task (P0 fix: the old code
            // stored a fresh empty map here, so responses were never matched).
            pending: pending,
            sender: msg_tx,
        })
    }

    pub async fn send(
        &self,
        method: &str,
        params: Value,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send_timeout(method, params, Duration::from_secs(10))
            .await
    }

    pub async fn send_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
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
            return Err(format!("Failed to send CDP command '{}'", method).into());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => {
                tracing::warn!(cdp_method = method, "CDP request channel closed: {}", e);
                Err(format!("CDP command '{}' channel closed: {}", method, e).into())
            }
            Err(_) => {
                // Drop the pending entry so the oneshot sender is freed and the
                // response can't pile up / leak for a request nobody awaits.
                self.pending.lock().await.remove(&id);
                tracing::warn!(
                    cdp_method = method,
                    timeout_ms = timeout.as_millis(),
                    "CDP command timed out"
                );
                Err(format!("CDP command '{}' timed out after {:?}", method, timeout).into())
            }
        }
    }

    pub async fn navigate(
        &self,
        url: &str,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send("Page.navigate", json!({ "url": url })).await
    }

    pub async fn evaluate(
        &self,
        expression: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": expression, "returnByValue": true, "awaitPromise": true,
                }),
            )
            .await?;
        match &resp.error {
            Some(err) => Err(format!("Runtime.evaluate error: {}", err).into()),
            None => {
                let result = resp
                    .result
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .cloned();
                result.ok_or_else(|| "No result value in evaluation response".into())
            }
        }
    }

    pub async fn execute(
        &self,
        expression: &str,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send(
            "Runtime.evaluate",
            json!({
                "expression": expression, "returnByValue": true,
            }),
        )
        .await
    }

    pub async fn execute_await(
        &self,
        expression: &str,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send(
            "Runtime.evaluate",
            json!({
                "expression": expression, "returnByValue": true, "awaitPromise": true,
            }),
        )
        .await
    }

    pub async fn set_device_scale_factor(
        &self,
        factor: f64,
        width: u32,
        height: u32,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width, "height": height, "deviceScaleFactor": factor, "mobile": false,
            }),
        )
        .await
    }

    pub async fn screenshot(&self) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self
            .send(
                "Page.captureScreenshot",
                json!({ "format": "png", "optimizeForSpeed": true }),
            )
            .await?;
        match &resp.error {
            Some(err) => Err(format!("Screenshot error: {}", err).into()),
            None => {
                let data = resp.result["data"]
                    .as_str()
                    .ok_or("No data in screenshot")?;
                Ok(decode_base64_png(data).map_err(|e| format!("Base64 error: {}", e))?)
            }
        }
    }

    pub async fn screenshot_element(
        &self,
        selector: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
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
        let resp = self
            .send(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "optimizeForSpeed": true,
                    "clip": { "x": x, "y": y, "width": w, "height": h, "scale": 1.0 },
                }),
            )
            .await?;
        match &resp.error {
            Some(err) => Err(format!("Screenshot error: {}", err).into()),
            None => {
                let data = resp.result["data"]
                    .as_str()
                    .ok_or("No data in screenshot")?;
                Ok(decode_base64_png(data).map_err(|e| format!("Base64 error: {}", e))?)
            }
        }
    }

    pub async fn create_isolated_world(
        &self,
        world_name: &str,
    ) -> Result<CdpResponse, Box<dyn std::error::Error + Send + Sync>> {
        self.send(
            "Page.createIsolatedWorld",
            json!({
                "frameId": "", "worldName": world_name,
            }),
        )
        .await
    }

    pub async fn close(&self) {
        let _ = self.sender.send(String::new()).await;
    }
}

struct ParsedCdp {
    id: Option<u64>,
    method: String,
    params: Value,
    result: Value,
    error: Option<Value>,
}

fn parse_cdp_message(text: &str) -> Result<ParsedCdp, String> {
    let msg: Value = serde_json::from_str(text).map_err(|e| format!("JSON parse error: {}", e))?;
    Ok(ParsedCdp {
        id: msg.get("id").and_then(|v| v.as_u64()),
        method: msg
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        // CDP command responses carry their payload in the top-level "result"
        // field; "params" is only populated on (unsolicited) events.
        params: msg.get("params").cloned().unwrap_or(Value::Null),
        result: msg.get("result").cloned().unwrap_or(Value::Null),
        error: msg.get("error").cloned(),
    })
}

/// Decode a base64-encoded PNG screenshot and validate it starts with the PNG
/// signature. Uses the `base64` crate to correctly handle `=` padding.
pub(crate) fn decode_base64_png(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| format!("invalid base64: {}", e))?;
    // PNG signature: 89 50 4E 47 0D 0A 1A 0A
    const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(format!(
            "decoded data is not a PNG (signature mismatch, {} bytes)",
            bytes.len()
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Top-level CDP "result" parsing (command responses)
    // ------------------------------------------------------------------
    #[test]
    fn parses_top_level_result_for_command_response() {
        let msg = r#"{"id":7,"result":{"data":"abc","type":"png"}}"#;
        let parsed = parse_cdp_message(msg).unwrap();
        assert_eq!(parsed.id, Some(7));
        assert_eq!(parsed.method, "");
        assert_eq!(parsed.result["data"], "abc");
        assert_eq!(parsed.result["type"], "png");
        assert_eq!(parsed.params, Value::Null); // no params on command responses
        assert!(parsed.error.is_none());
    }

    #[test]
    fn reports_unsolicited_event_in_params() {
        let msg = r#"{"method":"Page.loadEventFired","params":{"timestamp":123}}"#;
        let parsed = parse_cdp_message(msg).unwrap();
        assert_eq!(parsed.id, None);
        assert_eq!(parsed.method, "Page.loadEventFired");
        assert_eq!(parsed.params["timestamp"], 123);
        assert_eq!(parsed.result, Value::Null);
    }

    // ------------------------------------------------------------------
    // Padded base64 PNG decoding
    // ------------------------------------------------------------------
    #[test]
    fn decodes_padded_base64_png_and_validates_signature() {
        use base64::Engine as _;
        // Full PNG header plus a few body bytes.
        let png: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x01\x02\x03\x04";
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        // Ensure it actually carries '=' padding so the decoder must handle it.
        assert!(
            b64.ends_with('='),
            "test input should be padded, got: {}",
            b64
        );

        let decoded = decode_base64_png(&b64).expect("should decode padded base64");
        assert_eq!(decoded, png);
    }

    #[test]
    fn rejects_non_png_bytes() {
        use base64::Engine as _;
        let junk = base64::engine::general_purpose::STANDARD.encode(b"not a png at all");
        let err = decode_base64_png(&junk).unwrap_err();
        assert!(err.contains("not a PNG"), "unexpected error: {}", err);
    }

    #[test]
    fn rejects_invalid_base64() {
        assert!(decode_base64_png("!!!not-base64!!").is_err());
    }

    // ------------------------------------------------------------------
    // Request/response correlation over a mock WebSocket
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn correlates_request_responses_by_id() {
        // Bring up a local WebSocket echo/responder that answers each CDP command.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            use tokio_tungstenite::accept_async;
            let mut ws = accept_async(stream).await.unwrap();
            use futures_util::{SinkExt, StreamExt};
            while let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) =
                ws.next().await
            {
                let msg: Value = serde_json::from_str(&text).unwrap();
                let id = msg["id"].as_u64().unwrap();
                let method = msg["method"].as_str().unwrap_or("");
                // Echo a synthetic top-level result keyed to the method.
                let resp = json!({
                    "id": id,
                    "result": { "echo": method, "data": "qux", "params": msg["params"].clone() }
                });
                ws.send(tokio_tungstenite::tungstenite::Message::text(
                    resp.to_string(),
                ))
                .await
                .unwrap();
            }
        });

        let ws_url = format!("ws://{}/", addr);
        let client = CdpClient::connect(ws_url).await.unwrap();

        // Two sequential commands with distinct methods must each get their own
        // correlated response (proves the shared pending map + result parsing).
        let r1 = client
            .send("Page.navigate", json!({"url": "about:blank"}))
            .await
            .unwrap();
        assert_eq!(r1.id, Some(0));
        assert_eq!(r1.result["echo"], "Page.navigate");
        assert_eq!(r1.result["data"], "qux");

        let r2 = client
            .send("Runtime.evaluate", json!({"expression": "1+1"}))
            .await
            .unwrap();
        assert_eq!(r2.id, Some(1));
        assert_eq!(r2.result["echo"], "Runtime.evaluate");

        let r3 = client
            .execute_await("Promise.resolve('ready')")
            .await
            .unwrap();
        assert_eq!(r3.result["params"]["awaitPromise"], true);
        assert_eq!(r3.result["params"]["returnByValue"], true);
    }

    #[tokio::test]
    async fn times_out_and_cleans_up_pending_when_no_response() {
        // A server that accepts but never responds.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            use tokio_tungstenite::accept_async;
            let _ws = accept_async(stream).await.unwrap();
            // Keep the accepted socket alive beyond the client timeout while
            // intentionally sending no response.
            tokio::time::sleep(Duration::from_millis(150)).await;
        });

        let ws_url = format!("ws://{}/", addr);
        let client = CdpClient::connect(ws_url).await.unwrap();
        let err = client
            .send_timeout(
                "Page.navigate",
                json!({"url": "about:blank"}),
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("Page.navigate"),
            "expected method in error, got: {}",
            err
        );

        // Pending map should be empty after the timed-out cleanup.
        assert!(client.pending.lock().await.is_empty());
    }
}
