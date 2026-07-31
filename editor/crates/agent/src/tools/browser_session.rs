use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tungstenite::WebSocket;
use tungstenite::client::{IntoClientRequest, client_with_config};
use tungstenite::protocol::Message as WsMessage;
use tungstenite::stream::MaybeTlsStream;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccessibilityNode {
    pub backend_dom_node_id: Option<u64>,
    pub child_nodes: Vec<AccessibilityNode>,
    pub chrome_role: Option<String>,
    pub class_name: Option<String>,
    pub name: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TabInfo {
    pub target_id: String,
    pub title: String,
    pub url: String,
    pub is_attached: bool,
}

const CDP_READ_TIMEOUT: Duration = Duration::from_secs(30);
const CDP_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const CDP_COMMAND_DEADLINE: Duration = Duration::from_secs(30);

pub struct CdpClient {
    ws: WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    msg_id: AtomicU64,
}

impl CdpClient {
    pub fn connect(debug_url: &str) -> Result<Self> {
        let request = debug_url
            .into_client_request()
            .context("Failed to parse CDP URL")?;

        let uri = request.uri().clone();
        let host = uri.host().unwrap_or("127.0.0.1").to_string();
        let port = uri.port_u16().unwrap_or(9222);

        let addr = format!("{}:{}", host, port);
        let tcp = TcpStream::connect_timeout(
            &addr.parse().context("Invalid address")?,
            CDP_COMMAND_DEADLINE,
        )
        .with_context(|| format!("Failed to connect to Chrome debug port at {addr}"))?;
        tcp.set_read_timeout(Some(CDP_READ_TIMEOUT))
            .context("Failed to set read timeout")?;
        tcp.set_write_timeout(Some(CDP_WRITE_TIMEOUT))
            .context("Failed to set write timeout")?;

        let (ws, _) = client_with_config(request, MaybeTlsStream::Plain(tcp), None)
            .context("Failed to complete WebSocket handshake with Chrome")?;

        Ok(Self {
            ws,
            msg_id: AtomicU64::new(1),
        })
    }

    pub fn send_command(&mut self, method: &str, params: Value) -> Result<Value> {
        self.send_command_with_deadline(method, params, CDP_COMMAND_DEADLINE)
    }

    fn send_command_with_deadline(
        &mut self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value> {
        let id = self.msg_id.fetch_add(1, Ordering::SeqCst);

        let msg = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        self.ws.write(WsMessage::Text(msg.to_string().into()))?;

        let deadline = Instant::now() + deadline;

        loop {
            if Instant::now() > deadline {
                anyhow::bail!(
                    "CDP command '{}' timed out after {}s",
                    method,
                    CDP_COMMAND_DEADLINE.as_secs()
                );
            }

            let response = match self.ws.read() {
                Ok(WsMessage::Text(text)) => {
                    serde_json::from_str::<Value>(&text)
                        .context("Failed to parse CDP response")?
                }
                Ok(WsMessage::Close(_)) => {
                    anyhow::bail!("WebSocket closed");
                }
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
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

    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self.ws.get_ref() {
            MaybeTlsStream::Plain(tcp) => tcp.set_read_timeout(timeout),
            MaybeTlsStream::Rustls(stream) => stream.get_ref().set_read_timeout(timeout),
            #[allow(unreachable_patterns)]
            _ => Ok(()),
        }
    }

    /// Lightweight liveness probe with a short deadline so a dead WebSocket
    /// doesn't block session-pool reuse for the full command timeout. The
    /// socket's read timeout must be lowered to match, otherwise a
    /// dead-but-open socket blocks in `ws.read` for `CDP_READ_TIMEOUT`
    /// before the deadline can fire.
    pub fn ping(&mut self) -> Result<()> {
        let probe_timeout = Duration::from_secs(3);
        self.set_read_timeout(Some(probe_timeout))?;
        let result =
            self.send_command_with_deadline("Browser.getVersion", json!({}), probe_timeout)
                .map(|_| ());
        self.set_read_timeout(Some(CDP_READ_TIMEOUT))?;
        result
    }
}

pub struct BrowserSession {
    child: Option<Child>,
    _debug_port: u16,
    client: std::sync::Mutex<CdpClient>,
    _temp_dir: Option<tempfile::TempDir>,
}

impl BrowserSession {
    fn new(client: CdpClient, child: Option<Child>, debug_port: u16, temp_dir: Option<tempfile::TempDir>) -> Self {
        Self {
            child,
            _debug_port: debug_port,
            client: std::sync::Mutex::new(client),
            _temp_dir: temp_dir,
        }
    }

    pub fn launch(browser_path: &str) -> Result<Self> {
        let port = find_free_port()?;
        let temp_dir = tempfile::tempdir().context("Failed to create temp dir for browser")?;

        let mut child = Command::new(browser_path)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-extensions")
            .arg("--window-size=1280,720")
            .arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", temp_dir.path().display()))
            .arg("about:blank")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to launch browser")?;

        // Drain stderr on a background thread so the pipe buffer doesn't fill up
        // and stall the child process, and so we can log crash diagnostics.
        let stderr = child.stderr.take();
        std::thread::spawn(move || {
            if let Some(stderr) = stderr {
                let mut reader = std::io::BufReader::new(stderr);
                let mut buf = String::new();
                while reader.read_line(&mut buf).is_ok() {
                    let line = buf.trim().to_string();
                    if !line.is_empty() {
                        log::error!("[browser stderr] {line}");
                    }
                    buf.clear();
                }
            }
        });

        // Read stdout on a background thread to prevent pipe deadlocks.
        let stdout = child.stdout.take();
        std::thread::spawn(move || {
            if let Some(stdout) = stdout {
                let mut reader = std::io::BufReader::new(stdout);
                let mut buf = String::new();
                while reader.read_line(&mut buf).is_ok() {
                    buf.clear();
                }
            }
        });

        let mut last_error = String::from("no attempts made");
        let mut ws_url = None;
        for attempt in 0..30 {
            if let Ok(Some(_)) = child.try_wait() {
                anyhow::bail!(
                    "Browser process exited early during startup on port {port}. \
                     Check that '{}' is a valid Chrome/Chromium binary.",
                    browser_path
                );
            }
            match get_ws_url(port) {
                Ok(url) => {
                    ws_url = Some(url);
                    break;
                }
                Err(e) => {
                    last_error = e.to_string();
                    if attempt < 29 {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            }
        }
        let ws_url = ws_url.with_context(|| {
            // Kill the orphaned browser child before surfacing the error.
            let _ = child.kill();
            let _ = child.wait();
            format!(
                "Failed to get WebSocket URL from browser debug port {port}. \
                 Last error: {last_error}"
            )
        })?;
        let mut client = CdpClient::connect(&ws_url).map_err(|e| {
            let _ = child.kill();
            let _ = child.wait();
            e
        })?;
        client
            .send_command("Browser.getVersion", serde_json::json!({}))
            .map_err(|e| {
                let _ = child.kill();
                let _ = child.wait();
                e
            })
            .context("Browser process is unresponsive after connecting")?;

        Ok(Self::new(client, Some(child), port, Some(temp_dir)))
    }

    pub fn send_command(&self, method: &str, params: Value) -> Result<Value> {
        let mut client = self.client.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        client.send_command(method, params)
    }

    pub fn navigate(&self, url: &str) -> Result<()> {
        let result = self.send_command("Page.navigate", json!({ "url": url }))?;

        if let Some(error_text) = result.get("errorText").and_then(|v| v.as_str()) {
            if !error_text.is_empty() {
                anyhow::bail!("Navigation failed: {error_text}");
            }
        }

        let start = Instant::now();
        let timeout = Duration::from_secs(30);
        loop {
            let ready = self.send_command("Runtime.evaluate", json!({
                "expression": "document.readyState",
                "returnByValue": true
            }))?;

            let state = ready
                .get("result")
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // "complete": fully loaded.
            // "interactive": DOM ready, subresources may still load (SPA).
            // Treat both as done to avoid timing out on JS-heavy pages.
            if state == "complete" || state == "interactive" {
                // Small grace period for the SPA to attach event listeners.
                std::thread::sleep(Duration::from_millis(300));
                return Ok(());
            }

            if start.elapsed() > timeout {
                anyhow::bail!(
                    "Page did not reach readyState 'complete' or 'interactive' within {}s (last state: '{state}')",
                    timeout.as_secs()
                );
            }

            std::thread::sleep(Duration::from_millis(100));
        }
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
        if !selector.is_empty() {
            self.send_command("Runtime.evaluate", json!({
                "expression": format!("document.querySelector('{}').focus()", selector),
                "returnByValue": true
            }))?;
        }

        if !text.is_empty() {
            self.send_command("Input.insertText", json!({
                "text": text
            }))?;
        }

        Ok(())
    }

    pub fn fill(&self, selector: &str, value: &str) -> Result<()> {
        if selector.is_empty() {
            anyhow::bail!("fill: selector cannot be empty");
        }
        let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");
        self.send_command("Runtime.evaluate", json!({
            "expression": format!(
                r#"(() => {{
                    const el = document.querySelector('{}');
                    if (!el) return;
                    el.focus();
                    el.select();
                }})()"#,
                escaped
            ),
            "returnByValue": true
        }))?;

        if !value.is_empty() {
            self.send_command("Input.insertText", json!({
                "text": value
            }))?;
        }

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
        // Handle modifier+key combinations (e.g., "Control+C", "Control+V").
        if let Some((modifier, rest)) = key.split_once('+') {
            let modifier = modifier.trim();
            let rest = rest.trim();
            let (mod_code, mod_name) = match modifier {
                "Control" | "Ctrl" => (17, "Control"),
                "Shift" => (16, "Shift"),
                "Alt" => (18, "Alt"),
                "Meta" | "Command" | "Cmd" => (91, "Meta"),
                _ => (0, modifier),
            };
            let (key_code, code, key_name): (u32, &str, &str) = match rest {
                "c" | "C" => (67, "KeyC", "c"),
                "v" | "V" => (86, "KeyV", "v"),
                "a" | "A" => (65, "KeyA", "a"),
                "x" | "X" => (88, "KeyX", "x"),
                "z" | "Z" => (90, "KeyZ", "z"),
                other => (other.chars().next().unwrap_or('\0') as u32, other, other),
            };

            self.send_command("Input.dispatchKeyEvent", json!({
                "type": "rawKeyDown",
                "windowsVirtualKeyCode": mod_code,
                "code": mod_name,
                "key": mod_name
            }))?;
            self.send_command("Input.dispatchKeyEvent", json!({
                "type": "char",
                "text": key_name,
                "windowsVirtualKeyCode": key_code,
                "code": code,
                "key": key_name
            }))?;
            self.send_command("Input.dispatchKeyEvent", json!({
                "type": "keyUp",
                "windowsVirtualKeyCode": mod_code,
                "code": mod_name,
                "key": mod_name
            }))?;
            return Ok(());
        }

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

    pub fn get_accessibility_tree(&self) -> Result<Vec<AccessibilityNode>> {
        let result = self.send_command("Accessibility.getFullAXTree", json!({}))?;

        let nodes = result
            .get("nodes")
            .and_then(|v| v.as_array())
            .context("No accessibility nodes returned")?;

        let mut parsed = Vec::new();
        let mut nodes_by_id: HashMap<u64, Value> = HashMap::new();

        for node in nodes {
            if let Some(node_id) = node.get("nodeId").and_then(|v| v.as_u64()) {
                nodes_by_id.insert(node_id, node.clone());
            }
        }

        let root_id = result
            .get("root")
            .and_then(|v| v.get("nodeId"))
            .and_then(|v| v.as_u64());

        if let Some(root_id) = root_id {
            if nodes_by_id.contains_key(&root_id) {
                let tree = build_accessibility_tree(root_id, &nodes_by_id);
                parsed.push(tree);
            }
        }

        Ok(parsed)
    }

    pub fn click_by_index(&self, index: usize) -> Result<()> {
        let tree = self.get_accessibility_tree()?;
        let node = find_node_by_index(&tree, index)
            .context(format!("No element found at index {}", index))?;

        let backend_id = node
            .backend_dom_node_id
            .context("Element has no backend DOM node ID")?;

        let resolved = self.send_command("DOM.resolveNode", json!({
            "backendNodeId": backend_id
        }))?;

        let object_id = resolved
            .get("object")
            .and_then(|v| v.get("objectId"))
            .and_then(|v| v.as_str())
            .context("Failed to resolve node")?;

        let result = self.send_command("Runtime.callFunctionOn", json!({
            "objectId": object_id,
            "functionDeclaration": "function() { this.click(); }",
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

    pub fn type_by_index(&self, index: usize, text: &str) -> Result<()> {
        let tree = self.get_accessibility_tree()?;
        let node = find_node_by_index(&tree, index)
            .context(format!("No element found at index {}", index))?;

        let backend_id = node
            .backend_dom_node_id
            .context("Element has no backend DOM node ID")?;

        let resolved = self.send_command("DOM.resolveNode", json!({
            "backendNodeId": backend_id
        }))?;

        let object_id = resolved
            .get("object")
            .and_then(|v| v.get("objectId"))
            .and_then(|v| v.as_str())
            .context("Failed to resolve node")?;

        let _result = self.send_command("Runtime.callFunctionOn", json!({
            "objectId": object_id,
            "functionDeclaration": "function() { this.focus(); }",
            "returnByValue": true
        }))?;

        if !text.is_empty() {
            self.send_command("Input.insertText", json!({
                "text": text
            }))?;
        }

        Ok(())
    }

    pub fn fill_by_index(&self, index: usize, value: &str, clear: bool) -> Result<()> {
        let tree = self.get_accessibility_tree()?;
        let node = find_node_by_index(&tree, index)
            .context(format!("No element found at index {}", index))?;

        let backend_id = node
            .backend_dom_node_id
            .context("Element has no backend DOM node ID")?;

        let resolved = self.send_command("DOM.resolveNode", json!({
            "backendNodeId": backend_id
        }))?;

        let object_id = resolved
            .get("object")
            .and_then(|v| v.get("objectId"))
            .and_then(|v| v.as_str())
            .context("Failed to resolve node")?;

        if clear {
            let _result = self.send_command("Runtime.callFunctionOn", json!({
                "objectId": object_id,
                "functionDeclaration": "function() { this.value = ''; this.dispatchEvent(new Event('input', { bubbles: true })); }",
                "returnByValue": true
            }))?;
        }

        let _result = self.send_command("Runtime.callFunctionOn", json!({
            "objectId": object_id,
            "functionDeclaration": "function() { this.focus(); this.select(); }",
            "returnByValue": true
        }))?;

        if !value.is_empty() {
            self.send_command("Input.insertText", json!({
                "text": value
            }))?;
        }

        Ok(())
    }

    pub fn evaluate(&self, expression: &str) -> Result<Value> {
        let result = self.send_command("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true
        }))?;

        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("JavaScript evaluation failed: {}", text);
        }

        Ok(result.get("result").cloned().unwrap_or(json!(null)))
    }

    pub fn get_tabs(&self) -> Result<Vec<TabInfo>> {
        let result = self.send_command("Target.getTargets", json!({}))?;

        let target_infos = result
            .get("targetInfos")
            .and_then(|v| v.as_array())
            .context("No target infos returned")?;

        let mut tabs = Vec::new();
        for info in target_infos {
            if info.get("type").and_then(|v| v.as_str()) == Some("page") {
                tabs.push(TabInfo {
                    target_id: info
                        .get("targetId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    title: info
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: info
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    is_attached: info
                        .get("attached")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                });
            }
        }

        Ok(tabs)
    }

    pub fn switch_tab(&self, target_id: &str) -> Result<()> {
        self.send_command("Target.activateTarget", json!({
            "targetId": target_id
        }))?;
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    pub fn close_tab(&self, target_id: &str) -> Result<()> {
        self.send_command("Target.closeTarget", json!({
            "targetId": target_id
        }))?;
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    pub fn get_page_info(&self) -> Result<Value> {
        let expression = "JSON.stringify({url:location.href,title:document.title,w:innerWidth,h:innerHeight,sx:scrollX,sy:scrollY,pw:document.documentElement.scrollWidth,ph:document.documentElement.scrollHeight})";
        let result = self.send_command("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true
        }))?;

        let value_str = result
            .get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .context("No page info")?;

        let info: Value = serde_json::from_str(value_str)
            .context("Failed to parse page info JSON")?;

        Ok(info)
    }

    pub fn click_at_xy(&self, x: f64, y: f64, button: &str, clicks: u32) -> Result<()> {
        self.send_command("Input.dispatchMouseEvent", json!({
            "type": "mousePressed",
            "x": x,
            "y": y,
            "button": button,
            "clickCount": clicks
        }))?;

        self.send_command("Input.dispatchMouseEvent", json!({
            "type": "mouseReleased",
            "x": x,
            "y": y,
            "button": button,
            "clickCount": clicks
        }))?;

        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    pub fn wait_for_load(&self, timeout_ms: u64) -> Result<bool> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        loop {
            let result = self.send_command("Runtime.evaluate", json!({
                "expression": "document.readyState",
                "returnByValue": true
            }))?;

            let state = result
                .get("result")
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if state == "complete" {
                return Ok(true);
            }

            if start.elapsed() > timeout {
                anyhow::bail!("Page did not reach readyState 'complete' within {}ms", timeout_ms);
            }

            std::thread::sleep(Duration::from_millis(300));
        }
    }

    pub fn wait_for_network_idle(&self, timeout_ms: u64, idle_ms: u64) -> Result<bool> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);
        let idle_duration = Duration::from_millis(idle_ms);
        let mut last_activity = std::time::Instant::now();

        loop {
            let expression = r#"
                (() => {
                    const entries = performance.getEntriesByType('resource');
                    if (entries.length === 0) return 'idle';
                    const lastEntry = entries[entries.length - 1];
                    const elapsed = Date.now() - lastEntry.responseEnd;
                    return elapsed > 500 ? 'idle' : 'busy';
                })()
            "#;

            let result = self.send_command("Runtime.evaluate", json!({
                "expression": expression,
                "returnByValue": true
            }))?;

            let state = result
                .get("result")
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("busy");

            if state == "idle" {
                if last_activity.elapsed() >= idle_duration {
                    return Ok(true);
                }
            } else {
                last_activity = std::time::Instant::now();
            }

            if start.elapsed() > timeout {
                return Ok(false);
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn upload_file(&self, selector: &str, file_path: &str) -> Result<()> {
        let doc = self.send_command("DOM.getDocument", json!({"depth": -1}))?;

        let root_node_id = doc
            .get("root")
            .and_then(|v| v.get("nodeId"))
            .and_then(|v| v.as_u64())
            .context("No root node")?;

        let result = self.send_command("DOM.querySelector", json!({
            "nodeId": root_node_id,
            "selector": selector
        }))?;

        let node_id = result
            .get("nodeId")
            .and_then(|v| v.as_u64())
            .context("Element not found")?;

        if node_id == 0 {
            anyhow::bail!("No element found for selector: {}", selector);
        }

        self.send_command("DOM.setFileInputFiles", json!({
            "nodeId": node_id,
            "files": vec![file_path]
        }))?;

        Ok(())
    }

    pub fn get_iframe_targets(&self) -> Result<Vec<TabInfo>> {
        let result = self.send_command("Target.getTargets", json!({}))?;

        let target_infos = result
            .get("targetInfos")
            .and_then(|v| v.as_array())
            .context("No target infos returned")?;

        let mut iframes = Vec::new();
        for info in target_infos {
            if info.get("type").and_then(|v| v.as_str()) == Some("iframe") {
                iframes.push(TabInfo {
                    target_id: info
                        .get("targetId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    title: info
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: info
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    is_attached: info
                        .get("attached")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                });
            }
        }

        Ok(iframes)
    }

    pub fn dispatch_key_event(&self, selector: &str, key: &str, event_type: &str) -> Result<()> {
        let key_code = match key {
            "Enter" => 13,
            "Tab" => 9,
            "Escape" => 27,
            "Backspace" => 8,
            "Delete" => 46,
            " " => 32,
            "ArrowLeft" => 37,
            "ArrowUp" => 38,
            "ArrowRight" => 39,
            "ArrowDown" => 40,
            _ => key.chars().next().unwrap_or('\0') as u32,
        };

        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");
        let escaped_selector = selector.replace('\\', "\\\\").replace('\'', "\\'");

        let expression = format!(
            r#"(() => {{
                const e = document.querySelector('{}');
                if (e) {{
                    e.focus();
                    e.dispatchEvent(new KeyboardEvent('{}', {{
                        key: '{}',
                        code: '{}',
                        keyCode: {},
                        which: {},
                        bubbles: true
                    }}));
                }}
            }})()"#,
            escaped_selector, event_type, escaped_key, escaped_key, key_code, key_code
        );

        self.send_command("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true
        }))?;

        Ok(())
    }

    pub fn get_cookies(&self, urls: Option<Vec<String>>) -> Result<Vec<Value>> {
        let result = self.send_command("Network.getCookies", json!({
            "urls": urls.unwrap_or_default()
        }))?;

        let cookies = result
            .get("cookies")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(cookies)
    }

    pub fn set_cookie(&self, cookie: &Value) -> Result<()> {
        let name = cookie
            .get("name")
            .and_then(|v| v.as_str())
            .context("Cookie name required")?;

        let value = cookie
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let domain = cookie
            .get("domain")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let path = cookie
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("/");

        let secure = cookie
            .get("secure")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let http_only = cookie
            .get("httpOnly")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let same_site = cookie
            .get("sameSite")
            .and_then(|v| v.as_str())
            .unwrap_or("None");

        let expires = cookie
            .get("expires")
            .and_then(|v| v.as_f64())
            .unwrap_or(-1.0);

        self.send_command("Network.setCookie", json!({
            "name": name,
            "value": value,
            "domain": domain,
            "path": path,
            "secure": secure,
            "httpOnly": http_only,
            "sameSite": same_site,
            "expires": expires
        }))?;

        Ok(())
    }

    pub fn delete_cookies(&self, name: &str, domain: &str, path: &str) -> Result<()> {
        self.send_command("Network.deleteCookies", json!({
            "name": name,
            "domain": domain,
            "path": path
        }))?;

        Ok(())
    }

    pub fn query_shadow_dom(&self, shadow_selector: &str, inner_selector: &str) -> Result<String> {
        let escaped_shadow = shadow_selector.replace('\\', "\\\\").replace('\'', "\\'");
        let escaped_inner = inner_selector.replace('\\', "\\\\").replace('\'', "\\'");

        let expression = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el || !el.shadowRoot) return null;
                const inner = el.shadowRoot.querySelector('{}');
                if (!inner) return null;
                return inner.outerHTML;
            }})()"#,
            escaped_shadow, escaped_inner
        );

        let result = self.send_command("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true
        }))?;

        let html = result
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Shadow DOM element not found"))?;

        Ok(html.to_string())
    }

    pub fn click_in_shadow_dom(&self, shadow_selector: &str, inner_selector: &str) -> Result<()> {
        let escaped_shadow = shadow_selector.replace('\\', "\\\\").replace('\'', "\\'");
        let escaped_inner = inner_selector.replace('\\', "\\\\").replace('\'', "\\'");

        let expression = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el || !el.shadowRoot) return false;
                const inner = el.shadowRoot.querySelector('{}');
                if (!inner) return false;
                inner.click();
                return true;
            }})()"#,
            escaped_shadow, escaped_inner
        );

        let result = self.send_command("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true
        }))?;

        let success = result
            .get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !success {
            anyhow::bail!("Failed to click element in shadow DOM");
        }

        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    pub fn fill_in_shadow_dom(&self, shadow_selector: &str, inner_selector: &str, value: &str) -> Result<()> {
        let escaped_shadow = shadow_selector.replace('\\', "\\\\").replace('\'', "\\'");
        let escaped_inner = inner_selector.replace('\\', "\\\\").replace('\'', "\\'");

        let expression = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el || !el.shadowRoot) return false;
                const inner = el.shadowRoot.querySelector('{}');
                if (!inner) return false;
                inner.focus();
                inner.select();
                return true;
            }})()"#,
            escaped_shadow, escaped_inner
        );

        let result = self.send_command("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true
        }))?;

        let success = result
            .get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !success {
            anyhow::bail!("Failed to focus element in shadow DOM");
        }

        if !value.is_empty() {
            self.send_command("Input.insertText", json!({
                "text": value
            }))?;
        }

        Ok(())
    }

    pub fn start_download_monitoring(&self) -> Result<()> {
        self.send_command("Page.setDownloadBehavior", json!({
            "behavior": "allow",
            "downloadPath": dirs::download_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .to_string_lossy()
        }))?;
        Ok(())
    }

    pub fn get_downloads(&self) -> Result<Value> {
        let expression = r#"(() => {
            const downloads = [];
            if (window.__TAU_DOWNLOADS) {
                downloads.push(...window.__TAU_DOWNLOADS);
            }
            return JSON.stringify(downloads);
        })()"#;

        let result = self.send_command("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true
        }))?;

        Ok(result.get("result").cloned().unwrap_or(json!(null)))
    }

    pub fn is_alive(&mut self) -> bool {
        let child_alive = match &mut self.child {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        };
        if !child_alive {
            return false;
        }
        // Verify the WebSocket connection is still responsive with a lightweight ping.
        match self.client.lock() {
            Ok(mut client) => client.ping().is_ok(),
            Err(_) => false,
        }
    }

    pub fn close(&mut self) {
        if let Ok(client) = self.client.get_mut() {
            client.close();
        }
        if let Some(ref mut child) = self.child {
            if child.id() != 0 {
                for _ in 0..50 {
                    if let Ok(Some(_)) = child.try_wait() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            if child.id() != 0 {
                let _ = child.kill();
                let _ = child.wait();
            }
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
    // Prefer the page-level WebSocket from /json/list: the target is explicit,
    // so page-level commands (Page.navigate, Runtime.evaluate) are deterministic.
    match try_get_ws_url_from_json_list(port) {
        Ok(url) => return Ok(url),
        Err(e) => {
            log::warn!("/json/list on port {port} failed: {e}, trying /json/version");
        }
    }

    try_get_ws_url_from_json_version(port)
}

fn http_get(port: u16, path: &str) -> Result<String> {
    let addr = format!("127.0.0.1:{port}");
    let mut client = TcpStream::connect_timeout(
        &addr.parse().context("Invalid address")?,
        CDP_COMMAND_DEADLINE,
    )
    .with_context(|| format!("Failed to connect to {addr}"))?;
    // Use a short per-read timeout; the deadline below provides total timeout.
    client.set_read_timeout(Some(Duration::from_secs(2)))?;
    client.set_write_timeout(Some(CDP_WRITE_TIMEOUT))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",
    );

    client.write_all(request.as_bytes())?;

    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    let deadline = Instant::now() + CDP_COMMAND_DEADLINE;
    loop {
        if Instant::now() > deadline {
            anyhow::bail!("Timed out waiting for HTTP response from {addr}");
        }
        match client.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Chrome hasn't sent the response yet — retry quickly.
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    let response = String::from_utf8_lossy(&response);

    let status_line = response.lines().next().unwrap_or("");
    if !status_line.contains("200") {
        anyhow::bail!("HTTP {status_line}");
    }

    // Read exactly Content-Length bytes when present, otherwise read until
    // the end of the headers (after the blank line).
    let body = response
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("")
        .to_string();

    Ok(body)
}

fn try_get_ws_url_from_json_version(port: u16) -> Result<String> {
    let body = http_get(port, "/json/version")?;

    let json: Value = serde_json::from_str(&body)
        .context("Failed to parse /json/version JSON")?;

    let ws_url = json
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .context("No webSocketDebuggerUrl in /json/version response")?;

    Ok(ws_url.to_string())
}

fn try_get_ws_url_from_json_list(port: u16) -> Result<String> {
    let body = http_get(port, "/json/list")?;

    let targets: Vec<Value> = serde_json::from_str(&body)
        .context("Failed to parse /json/list JSON")?;

    if targets.is_empty() {
        anyhow::bail!("/json/list returned no debuggable targets yet");
    }

    let ws_url_from_target = |t: &Value| {
        t.get("webSocketDebuggerUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let is_page = |t: &Value| t.get("type").and_then(|v| v.as_str()) == Some("page");

    // Prefer page targets; only fall back to any target if no page exists yet.
    let ws_url = targets
        .iter()
        .find(|t| is_page(t) && ws_url_from_target(t).is_some())
        .or_else(|| {
            targets
                .iter()
                .find(|t| ws_url_from_target(t).is_some())
        })
        .and_then(ws_url_from_target)
        .context("No webSocketDebuggerUrl found in /json/list targets")?;

    Ok(ws_url)
}

fn build_accessibility_tree(node_id: u64, nodes_by_id: &HashMap<u64, Value>) -> AccessibilityNode {
    let node = nodes_by_id.get(&node_id).cloned().unwrap_or(json!({}));

    let backend_dom_node_id = node
        .get("backendDOMNodeId")
        .and_then(|v| v.as_u64());

    let child_node_ids: Vec<u64> = node
        .get("childIds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_u64())
                .collect()
        })
        .unwrap_or_default();

    let child_nodes: Vec<AccessibilityNode> = child_node_ids
        .iter()
        .filter_map(|child_id| nodes_by_id.get(child_id))
        .map(|child_node| {
            let child_node_id = child_node
                .get("nodeId")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            build_accessibility_tree(child_node_id, nodes_by_id)
        })
        .collect();

    AccessibilityNode {
        backend_dom_node_id,
        child_nodes,
        chrome_role: node
            .get("chromeRole")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        class_name: node
            .get("className")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        name: node
            .get("name")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        role: node
            .get("role")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    }
}

fn find_node_by_index(nodes: &[AccessibilityNode], target_index: usize) -> Option<AccessibilityNode> {
    let mut counter = 0;
    find_node_by_index_recursive(nodes, target_index, &mut counter)
}

fn find_node_by_index_recursive(
    nodes: &[AccessibilityNode],
    target_index: usize,
    counter: &mut usize,
) -> Option<AccessibilityNode> {
    for node in nodes {
        let current = *counter;
        *counter += 1;

        if current == target_index {
            return Some(node.clone());
        }

        if !node.child_nodes.is_empty() {
            if let Some(found) = find_node_by_index_recursive(&node.child_nodes, target_index, counter) {
                return Some(found);
            }
        }
    }

    None
}

fn accessibility_node_to_text(node: &AccessibilityNode, index: &mut usize, output: &mut String) {
    let current_index = *index;
    *index += 1;

    let name = node.name.as_deref().unwrap_or("");
    let role = node.role.as_deref().unwrap_or("");

    let label = if name.is_empty() && role.is_empty() {
        format!("{}\n", current_index)
    } else if name.is_empty() {
        format!("{}\n  role: {}\n", current_index, role)
    } else if role.is_empty() {
        format!("{}\n  name: {}\n", current_index, name)
    } else {
        format!("{}\n  role: {}, name: {}\n", current_index, role, name)
    };

    output.push_str(&label);

    for child in &node.child_nodes {
        accessibility_node_to_text(child, index, output);
    }
}

pub fn format_accessibility_tree(nodes: &[AccessibilityNode]) -> String {
    let mut output = String::new();
    let mut index = 0;

    for node in nodes {
        accessibility_node_to_text(node, &mut index, &mut output);
    }

    output
}
