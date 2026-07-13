use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tungstenite::WebSocket;
use tungstenite::protocol::Message as WsMessage;

pub struct CdpClient {
    ws: WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    msg_id: AtomicU64,
}

impl CdpClient {
    pub fn connect(debug_url: &str) -> Result<Self> {
        let (ws, _) = tungstenite::connect(debug_url)
            .context("Failed to connect to Chrome DevTools Protocol")?;

        Ok(Self {
            ws,
            msg_id: AtomicU64::new(1),
        })
    }

    pub fn send_command(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.msg_id.fetch_add(1, Ordering::SeqCst);

        let msg = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        self.ws.write(WsMessage::Text(msg.to_string().into()))?;

        loop {
            let response = match self.ws.read() {
                Ok(WsMessage::Text(text)) => {
                    serde_json::from_str::<Value>(&text)
                        .context("Failed to parse CDP response")?
                }
                Ok(WsMessage::Close(_)) => {
                    anyhow::bail!("WebSocket closed");
                }
                Ok(_) => continue,
                Err(e) => {
                    anyhow::bail!("WebSocket error: {}", e);
                }
            };

            if let Some(msg_id) = response.get("id").and_then(|v| v.as_u64()) {
                if msg_id == id {
                    if let Some(error) = response.get("error") {
                        anyhow::bail!(
                            "CDP error: {} - {}",
                            error.get("code").unwrap_or(&json!(0)),
                            error.get("message").unwrap_or(&json!("unknown"))
                        );
                    }
                    return Ok(response.get("result").cloned().unwrap_or(json!(null)));
                }
            }
        }
    }

    pub fn close(&mut self) {
        let _ = self.ws.close(None);
    }
}

pub struct BrowserSession {
    child: Child,
    _debug_port: u16,
    client: std::sync::Mutex<CdpClient>,
}

impl BrowserSession {
    pub fn launch(chrome_path: &str) -> Result<Self> {
        let port = find_free_port()?;

        let child = Command::new(chrome_path)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-extensions")
            .arg("--window-size=1280,720")
            .arg(format!("--remote-debugging-port={}", port))
            .arg("about:blank")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to launch Chrome")?;

        std::thread::sleep(Duration::from_millis(500));

        let ws_url = get_ws_url(port).context("Failed to get WebSocket URL")?;
        let client = CdpClient::connect(&ws_url)?;

        Ok(Self {
            child,
            _debug_port: port,
            client: std::sync::Mutex::new(client),
        })
    }

    pub fn send_command(&self, method: &str, params: Value) -> Result<Value> {
        let mut client = self.client.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        client.send_command(method, params)
    }

    pub fn navigate(&self, url: &str) -> Result<()> {
        self.send_command("Page.navigate", json!({ "url": url }))?;
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    }

    pub fn screenshot(&self) -> Result<String> {
        let result = self.send_command("Page.captureScreenshot", json!({
            "format": "png"
        }))?;

        let data = result
            .get("data")
            .and_then(|v| v.as_str())
            .context("No screenshot data")?;

        Ok(data.to_string())
    }

    pub fn get_dom(&self) -> Result<String> {
        let result = self.send_command("Runtime.evaluate", json!({
            "expression": "document.documentElement.outerHTML",
            "returnByValue": true
        }))?;

        let value = result
            .get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .context("No DOM content")?;

        Ok(value.to_string())
    }

    pub fn click(&self, selector: &str) -> Result<()> {
        let result = self.send_command("Runtime.evaluate", json!({
            "expression": format!("document.querySelector('{}').click()", selector),
            "returnByValue": true
        }))?;

        if let Some(exception) = result.get("exceptionDetails") {
            anyhow::bail!(
                "Click failed: {}",
                exception.get("text").unwrap_or(&json!("unknown error"))
            );
        }

        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    pub fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        self.send_command("Runtime.evaluate", json!({
            "expression": format!("document.querySelector('{}').focus()", selector),
            "returnByValue": true
        }))?;

        for ch in text.chars() {
            self.send_command("Input.dispatchKeyEvent", json!({
                "type": "keyDown",
                "text": ch.to_string()
            }))?;
            self.send_command("Input.dispatchKeyEvent", json!({
                "type": "keyUp",
                "text": ch.to_string()
            }))?;
        }

        Ok(())
    }

    pub fn fill(&self, selector: &str, value: &str) -> Result<()> {
        self.send_command("Runtime.evaluate", json!({
            "expression": format!(
                "(() => {{ const el = document.querySelector('{}'); el.value = '{}'; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }})); }})()",
                selector.replace('\'', "\\'"),
                value.replace('\'', "\\'").replace('\n', "\\n")
            ),
            "returnByValue": true
        }))?;

        Ok(())
    }

    pub fn scroll(&self, x: i64, y: i64) -> Result<()> {
        self.send_command("Input.dispatchMouseEvent", json!({
            "type": "mouseWheel",
            "x": 640,
            "y": 360,
            "deltaX": x,
            "deltaY": y
        }))?;

        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    pub fn press_key(&self, key: &str) -> Result<()> {
        let (key_code, code) = match key {
            "Enter" => (13, "Enter"),
            "Tab" => (9, "Tab"),
            "Escape" => (27, "Escape"),
            "Backspace" => (8, "Backspace"),
            "Delete" => (46, "Delete"),
            "ArrowUp" => (38, "ArrowUp"),
            "ArrowDown" => (40, "ArrowDown"),
            "ArrowLeft" => (37, "ArrowLeft"),
            "ArrowRight" => (39, "ArrowRight"),
            "Space" => (32, "Space"),
            _ => (0, key),
        };

        self.send_command("Input.dispatchKeyEvent", json!({
            "type": "keyDown",
            "windowsVirtualKeyCode": key_code,
            "code": code,
            "key": key
        }))?;

        self.send_command("Input.dispatchKeyEvent", json!({
            "type": "keyUp",
            "windowsVirtualKeyCode": key_code,
            "code": code,
            "key": key
        }))?;

        Ok(())
    }

    pub fn wait_for_element(&self, selector: &str, timeout_ms: u64) -> Result<()> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        loop {
            let result = self.send_command("Runtime.evaluate", json!({
                "expression": format!("document.querySelector('{}') !== null", selector.replace('\'', "\\'")),
                "returnByValue": true
            }))?;

            let found = result
                .get("result")
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if found {
                return Ok(());
            }

            if start.elapsed() > timeout {
                anyhow::bail!(
                    "Timeout waiting for element '{}' after {}ms",
                    selector,
                    timeout_ms
                );
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn get_page_title(&self) -> Result<String> {
        let result = self.send_command("Runtime.evaluate", json!({
            "expression": "document.title",
            "returnByValue": true
        }))?;

        let title = result
            .get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(title)
    }

    pub fn get_page_url(&self) -> Result<String> {
        let result = self.send_command("Runtime.evaluate", json!({
            "expression": "window.location.href",
            "returnByValue": true
        }))?;

        let url = result
            .get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(url)
    }

    pub fn is_alive(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.child.id() != 0
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.child.id().is_some()
        }
    }

    pub fn close(&mut self) {
        #[cfg(target_os = "linux")]
        let alive = self.child.id() != 0;
        #[cfg(not(target_os = "linux"))]
        let alive = self.child.id().is_some();

        if alive {
            if let Ok(client) = self.client.get_mut() {
                client.close();
            }
            let _ = self.child.kill();
        }
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        let alive = self.child.id() != 0;
        #[cfg(not(target_os = "linux"))]
        let alive = self.child.id().is_some();

        if alive {
            let _ = self.child.kill();
        }
    }
}

fn find_free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn get_ws_url(port: u16) -> Result<String> {
    let mut client = TcpStream::connect(format!("127.0.0.1:{}", port))?;

    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        port
    );

    client.write_all(request.as_bytes())?;

    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = client.read(&mut buf)?;
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
    }

    let response = String::from_utf8_lossy(&response);

    let body = response
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("");

    let json: Value = serde_json::from_str(body)
        .context("Failed to parse Chrome version JSON")?;

    let ws_url = json
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .context("No webSocketDebuggerUrl in response")?;

    Ok(ws_url.to_string())
}
