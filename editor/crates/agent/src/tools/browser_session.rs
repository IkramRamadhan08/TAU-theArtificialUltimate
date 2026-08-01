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
    downloads: std::sync::Mutex<HashMap<String, Value>>,
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
            downloads: std::sync::Mutex::new(HashMap::new()),
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

        // Tungstenite buffers small messages internally and only writes to the
        // socket on `flush()`; without it the command never reaches Chrome and
        // the subsequent read blocks until the deadline.
        self.ws.write(WsMessage::Text(msg.to_string().into()))?;
        self.ws.flush()?;

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
            } else if let Some(method) = response.get("method").and_then(|v| v.as_str()) {
                // Capture download events so `get_downloads` can report real
                // downloads instead of always returning an empty list.
                if let Some(params) = response.get("params") {
                    match method {
                        "Browser.downloadWillBegin" | "Page.downloadWillBegin" => {
                            let mut entry = params.clone();
                            entry["state"] = json!("in_progress");
                            if let Some(guid) = entry.get("guid").and_then(|v| v.as_str()) {
                                self.downloads
                                    .lock()
                                    .map_err(|e| anyhow::anyhow!("{}", e))?
                                    .insert(guid.to_string(), entry);
                            }
                        }
                        "Browser.downloadProgress" | "Page.downloadProgress" => {
                            if let Some(guid) = params.get("guid").and_then(|v| v.as_str()) {
                                let mut downloads = self
                                    .downloads
                                    .lock()
                                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                                let entry = downloads
                                    .entry(guid.to_string())
                                    .or_insert_with(|| json!({ "guid": guid }));
                                for key in ["state", "receivedBytes", "totalBytes"] {
                                    if let Some(value) = params.get(key) {
                                        entry[key] = value.clone();
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
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
        let mut client = CdpClient::connect(&ws_url).inspect_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
        })?;
        client
            .send_command("Browser.getVersion", serde_json::json!({}))
            .inspect_err(|_| {
                let _ = child.kill();
                let _ = child.wait();
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
        let mut nodes_by_id: HashMap<i64, Value> = HashMap::new();

        for node in nodes {
            if let Some(node_id) = node.get("nodeId").and_then(ax_id) {
                nodes_by_id.insert(node_id, node.clone());
            }
        }

        let root_id = result
            .get("root")
            .and_then(|v| v.get("nodeId"))
            .and_then(ax_id)
            .or_else(|| {
                // Chrome no longer returns a `root` key from getFullAXTree;
                // the root is the node without a `parentId`.
                nodes_by_id
                    .iter()
                    .find(|(_, node)| node.get("parentId").is_none())
                    .map(|(node_id, _)| *node_id)
            });

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
        // Target.getTargets only exposes iframes when they run as separate
        // (out-of-process) targets, which localhost and many same-site frames
        // never do. Page.getFrameTree lists every frame, so it is the reliable
        // source for enumerating iframes.
        let result = self.send_command("Page.getFrameTree", json!({}))?;

        fn walk_frames(frame_tree: &Value, out: &mut Vec<TabInfo>) {
            if let Some(frame) = frame_tree.get("frame") {
                if let Some(frame_id) = frame.get("id").and_then(|v| v.as_str()) {
                    out.push(TabInfo {
                        target_id: frame_id.to_string(),
                        title: frame
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        url: frame
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        is_attached: true,
                    });
                }
            }
            if let Some(children) = frame_tree
                .get("childFrames")
                .and_then(|v| v.as_array())
            {
                for child in children {
                    walk_frames(child, out);
                }
            }
        }

        let mut iframes = Vec::new();
        walk_frames(result.get("frameTree").unwrap_or(&json!({})), &mut iframes);
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
        // Network.getCookies with an empty urls list returns no cookies, so
        // fall back to Network.getAllCookies when no urls are requested.
        let result = match urls {
            Some(urls) => self.send_command("Network.getCookies", json!({ "urls": urls }))?,
            None => self.send_command("Network.getAllCookies", json!({}))?,
        };

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

        let result = self.send_command("Network.setCookie", json!({
            "name": name,
            "value": value,
            "domain": domain,
            "path": path,
            "secure": secure,
            "httpOnly": http_only,
            "sameSite": same_site,
            "expires": expires
        }))?;

        // Network.setCookie can silently reject a cookie; surface that.
        let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
        if !success {
            anyhow::bail!("Failed to set cookie '{}' for domain '{}'", name, domain);
        }

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
            .get("result")
            .and_then(|v| v.get("value"))
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
        self.start_download_monitoring_in(
            &dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
        )
    }

    pub fn start_download_monitoring_in(&self, download_path: &std::path::Path) -> Result<()> {
        // Page.enable is required for Chrome to emit the
        // Page.downloadWillBegin/Page.downloadProgress events that
        // `get_downloads` relies on.
        self.send_command("Page.enable", json!({}))?;
        self.send_command("Page.setDownloadBehavior", json!({
            "behavior": "allow",
            "downloadPath": download_path.to_string_lossy(),
            "eventsEnabled": true
        }))?;
        Ok(())
    }

    pub fn get_downloads(&self) -> Result<Value> {
        // Events only reach the read loop while a command is in flight, so drain
        // any buffered download events with a trivial evaluate before reading.
        self.send_command("Runtime.evaluate", json!({ "expression": "1" }))?;

        let downloads = self
            .client
            .lock()
            .map_err(|e| anyhow::anyhow!("{}", e))?
            .downloads
            .lock()
            .map_err(|e| anyhow::anyhow!("{}", e))?
            .values()
            .cloned()
            .collect::<Vec<_>>();

        Ok(serde_json::Value::Array(downloads))
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

/// Returns true once the full HTTP response has been received, judged by
/// `Content-Length`. Headers that arrive in separate reads are accumulated
/// until we know the expected body size; we then stop as soon as that many
/// body bytes have arrived. When the server omits `Content-Length` (no
/// chunked encoding), this returns false and the caller falls back to
/// reading until EOF or the deadline.
fn http_response_complete(response: &[u8]) -> bool {
    let headers_end = response.windows(4).position(|window| window == b"\r\n\r\n");
    let Some(headers_end) = headers_end else {
        return false;
    };
    let expected_length = String::from_utf8_lossy(&response[..headers_end])
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        });
    match expected_length {
        Some(expected) => response.len() >= headers_end + 4 + expected,
        None => false,
    }
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
            Ok(n) => {
                response.extend_from_slice(&buf[..n]);
                if http_response_complete(&response) {
                    break;
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Chrome hasn't sent the whole response yet — retry quickly.
                // Chrome keeps the TCP connection open after responding
                // (ignoring `Connection: close`), so we must not wait for
                // EOF; we stop once Content-Length bytes have been read.
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

// Accessibility node ids come from Chrome as numbers or numeric strings, and
// can be negative for synthetic nodes (e.g. InlineTextBox).
fn ax_id(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
}

fn build_accessibility_tree(node_id: i64, nodes_by_id: &HashMap<i64, Value>) -> AccessibilityNode {
    let node = nodes_by_id.get(&node_id).cloned().unwrap_or(json!({}));

    let backend_dom_node_id = node
        .get("backendDOMNodeId")
        .and_then(|v| v.as_u64());

    let child_node_ids: Vec<i64> = node
        .get("childIds")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(ax_id).collect())
        .unwrap_or_default();

    let child_nodes: Vec<AccessibilityNode> = child_node_ids
        .iter()
        .filter_map(|child_id| nodes_by_id.get(child_id))
        .map(|child_node| {
            let child_node_id = child_node
                .get("nodeId")
                .and_then(ax_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    // Serializes the end-to-end browser tests. Launching several headless
    // Chromium instances concurrently makes `find_free_port` (bind, read port,
    // release) vulnerable to races where two instances claim the same debug
    // port. Production only launches one session at a time behind the global
    // session mutex, so the lock is test-only.
    static BROWSER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // Recover from a poisoned lock (a prior test panicked while holding it) so
    // one failure does not cascade into the other browser tests.
    fn browser_test_guard() -> std::sync::MutexGuard<'static, ()> {
        BROWSER_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn available_browser() -> &'static str {
        [
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
        ]
        .iter()
        .find(|path| std::path::Path::new(path).exists())
        .copied()
        .unwrap_or("")
    }

    // Chrome's /json/list and /json/version endpoints respond with 200 OK and
    // a Content-Length, then keep the TCP connection open indefinitely
    // (ignoring `Connection: close`). http_get must stop once Content-Length
    // bytes have arrived instead of waiting for EOF, or every browser tool
    // call blocks for the full 30s read deadline.
    #[test]
    fn json_endpoint_returns_body_when_connection_stays_open() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = r#"[{"description":"","id":"FAKE","title":"about:blank","type":"page","url":"about:blank","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/FAKE"}]"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                // Hold the socket open well past the assertion window so a
                // regression that waits for EOF hits the read deadline
                // instead of an early EOF.
                std::thread::sleep(Duration::from_secs(60));
            }
        });

        let start = Instant::now();
        let ws_url = try_get_ws_url_from_json_list(port).expect("should return the WS URL");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "http_get blocked on keep-alive connection for {}s",
            start.elapsed().as_secs()
        );
        assert_eq!(
            ws_url,
            "ws://127.0.0.1:9222/devtools/page/FAKE"
        );
    }

    #[test]
    fn http_response_complete_detects_full_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        assert!(http_response_complete(response));
        assert!(!http_response_complete(&response[..response.len() - 1]));
        assert!(!http_response_complete(b"HTTP/1.1 200 OK\r\n"));
        assert!(!http_response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhell"
        ));
    }

    // End-to-end run of the real launch flow against system Chrome/Chromium:
    // launch (which fetches the WS URL over the keep-alive HTTP endpoint),
    // navigate, evaluate, screenshot, and close. Skipped when no browser is
    // installed. This is the flow behind every browser_* tool, so it guards
    // against regressions in get_ws_url/http_get hanging on the keep-alive
    // connection.
    #[test]
    fn browser_session_launch_navigate_screenshot() {
        let browser_path = available_browser();
        if browser_path.is_empty() {
            return;
        }
        let _guard = browser_test_guard();

        let start = Instant::now();
        let mut session = BrowserSession::launch(browser_path)
            .expect("browser should launch and connect");
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "launch took {}s",
            start.elapsed().as_secs()
        );

        session
            .navigate("data:text/html,<title>TAU Browser Test</title><h1>ok</h1>")
            .expect("navigate should complete");
        let title = session.get_page_title().expect("get title");
        assert!(title.contains("TAU Browser Test"), "unexpected title: {title}");

        let dom = session.get_dom().expect("get dom");
        assert!(dom.contains("<h1>ok</h1>"), "unexpected dom: {dom}");

        let screenshot = session.screenshot().expect("screenshot");
        assert!(!screenshot.is_empty(), "screenshot should not be empty");

        session.close();
    }

    // ------------------------------------------------------------------------
    // Local HTTP test server + comprehensive coverage of the remaining
    // BrowserSession methods (and therefore the browser_* tools that call
    // them).
    // ------------------------------------------------------------------------

    struct TestServer {
        addr: String,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    const MAIN_PAGE: &str = r#"<!DOCTYPE html>
    <html>
    <head>
      <title>TAU E2E Test</title>
      <style>
        #scroll-box { overflow: auto; height: 80px; width: 200px; }
        #scroll-inner { height: 400px; width: 200px; }
        #spacer { height: 2000px; }
      </style>
    </head>
    <body>
      <h1 id="heading">Test Page</h1>
      <input id="name-input" type="text">
      <input id="file-input" type="file">
      <button id="click-button">Click</button>
      <select id="select-box"><option value="a">A</option><option value="b">B</option></select>
      <div id="scroll-box"><div id="scroll-inner">scroll target</div></div>
      <div id="shadow-host"></div>
      <iframe id="test-iframe" src="/iframe" style="width:300px;height:150px"></iframe>
      <a id="download-link" href="/download">Download</a>
      <div id="spacer"></div>
      <script>
        window.__TAU_CLICKED = false;
        window.__TAU_SHADOW_VALUE = null;
        window.__TAU_KEYS = [];
        document.addEventListener('keydown', function (e) { window.__TAU_KEYS.push(e.key); });
        document.getElementById('click-button').addEventListener('click', function () {
          window.__TAU_CLICKED = true;
        });
        var host = document.getElementById('shadow-host');
        var root = host.attachShadow({ mode: 'open' });
        root.innerHTML = '<input id="shadow-input"><button id="shadow-button">Shadow Click</button>';
        root.querySelector('#shadow-button').addEventListener('click', function () {
          window.__TAU_SHADOW_VALUE = 'clicked';
        });
      </script>
    </body>
    </html>"#;

    fn http_response(status: u16, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn handle_test_connection(stream: &mut std::net::TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut buf = [0u8; 8192];
        let mut request = String::new();
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => {
                    request.push_str(&String::from_utf8_lossy(&buf[..read]));
                    if request.contains("\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let path = path.split('?').next().unwrap_or("/");

        let response = match path {
            "/" => http_response(200, "text/html", MAIN_PAGE),
            "/iframe" => http_response(
                200,
                "text/html",
                "<!DOCTYPE html><html><head><title>Iframe</title></head><body><h1 id='inner'>iframe</h1><input id='iframe-input'></body></html>",
            ),
            "/download" => {
                let body = "hello download";
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"tau-test.txt\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            }
            _ => http_response(404, "text/plain", "not found"),
        };

        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    impl TestServer {
        fn start() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
            let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = shutdown.clone();
            let handle = std::thread::spawn(move || {
                let _ = listener.set_nonblocking(true);
                while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => handle_test_connection(&mut stream),
                        Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(25));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                addr,
                shutdown,
                handle: Some(handle),
            }
        }

        fn stop(mut self) {
            self.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn poll_with_timeout<F: FnMut() -> bool>(mut check: F, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if check() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    fn flatten_ax<'a>(
        nodes: &'a [AccessibilityNode],
        out: &mut Vec<(usize, &'a AccessibilityNode)>,
    ) {
        let mut counter = 0;
        flatten_ax_recursive(nodes, &mut counter, out);
    }

    fn flatten_ax_recursive<'a>(
        nodes: &'a [AccessibilityNode],
        counter: &mut usize,
        out: &mut Vec<(usize, &'a AccessibilityNode)>,
    ) {
        for node in nodes {
            let index = *counter;
            *counter += 1;
            out.push((index, node));
            if !node.child_nodes.is_empty() {
                flatten_ax_recursive(&node.child_nodes, counter, out);
            }
        }
    }

    #[test]
    fn browser_session_navigation_and_interaction() {
        let browser_path = available_browser();
        if browser_path.is_empty() {
            return;
        }
        let _guard = browser_test_guard();
        let server = TestServer::start();
        let mut session = BrowserSession::launch(browser_path).expect("launch");
        session.navigate(&server.addr).expect("navigate");

        let title = session.get_page_title().expect("title");
        assert_eq!(title, "TAU E2E Test", "unexpected title: {title}");
        assert!(
            session.get_page_url().expect("url").starts_with(&server.addr),
            "unexpected url"
        );
        let dom = session.get_dom().expect("dom");
        assert!(dom.contains("id=\"click-button\""), "dom missing button");
        let info = session.get_page_info().expect("page info");
        assert_eq!(
            info.get("title").and_then(|v| v.as_str()),
            Some("TAU E2E Test"),
            "unexpected page info"
        );
        assert!(session.wait_for_load(5000).expect("wait_for_load"));

        let value = session.evaluate("2 + 2").expect("evaluate");
        assert_eq!(value.get("value").and_then(|v| v.as_i64()), Some(4));

        session.click("#click-button").expect("click");
        let clicked = session.evaluate("window.__TAU_CLICKED").expect("clicked state");
        assert_eq!(clicked.get("value").and_then(|v| v.as_bool()), Some(true));

        session.type_text("#name-input", "hello").expect("type");
        let typed = session.evaluate("document.getElementById('name-input').value").expect("typed value");
        assert_eq!(typed.get("value").and_then(|v| v.as_str()), Some("hello"));

        session.fill("#name-input", "world").expect("fill");
        let filled = session.evaluate("document.getElementById('name-input').value").expect("filled value");
        assert_eq!(filled.get("value").and_then(|v| v.as_str()), Some("world"));

        session.press_key("ArrowDown").expect("press_key");
        let keys = session.evaluate("window.__TAU_KEYS").expect("keys");
        assert!(
            keys.get("value")
                .and_then(|v| v.as_array())
                .map(|keys| keys.iter().any(|key| key == "ArrowDown"))
                .unwrap_or(false),
            "ArrowDown not recorded: {keys}"
        );

        session.dispatch_key_event("#name-input", "Enter", "keydown").expect("dispatch key");
        let keys = session.evaluate("window.__TAU_KEYS").expect("keys 2");
        assert!(
            keys.get("value")
                .and_then(|v| v.as_array())
                .map(|keys| keys.iter().any(|key| key == "Enter"))
                .unwrap_or(false),
            "Enter not recorded: {keys}"
        );

        session.evaluate("window.__TAU_CLICKED = false").expect("reset clicked");
        let rect = session
            .evaluate(
                "(() => { const r = document.getElementById('click-button').getBoundingClientRect(); \
                 return JSON.stringify({x: r.x + r.width / 2, y: r.y + r.height / 2}); })()",
            )
            .expect("rect");
        let rect: Value =
            serde_json::from_str(rect.get("value").and_then(|v| v.as_str()).expect("rect string"))
                .expect("rect json");
        let x = rect.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let y = rect.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        session.click_at_xy(x, y, "left", 1).expect("click at xy");
        let clicked = session.evaluate("window.__TAU_CLICKED").expect("clicked after xy");
        assert_eq!(clicked.get("value").and_then(|v| v.as_bool()), Some(true));

        session.scroll(0, 800).expect("scroll");
        session.scroll(0, 800).expect("scroll 2");
        let scrolled = poll_with_timeout(
            || {
                session
                    .evaluate("window.scrollY")
                    .map(|value| {
                        value
                            .get("value")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0)
                            > 0.0
                    })
                    .unwrap_or(false)
            },
            Duration::from_secs(3),
        );
        assert!(scrolled, "page did not scroll");

        session.wait_for_element("#name-input", 5000).expect("wait for element");
        let idle = session.wait_for_network_idle(5000, 500).expect("wait network idle");
        assert!(idle, "network never went idle");

        let tabs = session.get_tabs().expect("tabs");
        assert!(!tabs.is_empty(), "expected at least one tab");
        let create = session
            .send_command("Target.createTarget", json!({ "url": "about:blank" }))
            .expect("create tab");
        let new_id = create
            .get("targetId")
            .and_then(|v| v.as_str())
            .expect("new target id")
            .to_string();
        assert_eq!(session.get_tabs().expect("tabs 2").len(), 2, "expected two tabs");
        session.switch_tab(&new_id).expect("switch tab");
        session.close_tab(&new_id).expect("close tab");
        assert_eq!(session.get_tabs().expect("tabs 3").len(), 1, "expected one tab");

        session.close();
        server.stop();
    }

    #[test]
    fn browser_session_advanced_features() {
        let browser_path = available_browser();
        if browser_path.is_empty() {
            return;
        }
        let _guard = browser_test_guard();
        let server = TestServer::start();
        let mut session = BrowserSession::launch(browser_path).expect("launch");
        session.navigate(&server.addr).expect("navigate");

        let tree = session.get_accessibility_tree().expect("ax tree");
        assert!(!tree.is_empty(), "expected non-empty accessibility tree");
        let mut flat = Vec::new();
        flatten_ax(&tree, &mut flat);

        let click_index = flat
            .iter()
            .find(|(_, node)| node.name.as_deref().map(|n| n.contains("Click")).unwrap_or(false))
            .map(|(index, _)| *index)
            .expect("no Click button in accessibility tree");
        session.click_by_index(click_index).expect("click_by_index");
        let clicked = session.evaluate("window.__TAU_CLICKED").expect("clicked by index");
        assert_eq!(clicked.get("value").and_then(|v| v.as_bool()), Some(true));

        let textbox_index = flat
            .iter()
            .find(|(_, node)| node.role.as_deref() == Some("textbox"))
            .map(|(index, _)| *index)
            .expect("no textbox in accessibility tree");
        session.type_by_index(textbox_index, "abc").expect("type_by_index");
        let typed = session.evaluate("document.getElementById('name-input').value").expect("typed by index");
        assert_eq!(typed.get("value").and_then(|v| v.as_str()), Some("abc"));
        session.fill_by_index(textbox_index, "xyz", true).expect("fill_by_index");
        let filled = session.evaluate("document.getElementById('name-input').value").expect("filled by index");
        assert_eq!(filled.get("value").and_then(|v| v.as_str()), Some("xyz"));

        let iframe_found = poll_with_timeout(
            || {
                session
                    .get_iframe_targets()
                    .map(|targets| targets.iter().any(|info| info.url.contains("/iframe")))
                    .unwrap_or(false)
            },
            Duration::from_secs(10),
        );
        assert!(iframe_found, "no iframe target found");

        let html = session
            .query_shadow_dom("#shadow-host", "#shadow-button")
            .expect("query shadow");
        assert!(html.contains("shadow-button"), "shadow query: {html}");
        session
            .click_in_shadow_dom("#shadow-host", "#shadow-button")
            .expect("click shadow");
        let shadow_state = session.evaluate("window.__TAU_SHADOW_VALUE").expect("shadow state");
        assert_eq!(shadow_state.get("value").and_then(|v| v.as_str()), Some("clicked"));
        session
            .fill_in_shadow_dom("#shadow-host", "#shadow-input", "secret")
            .expect("fill shadow");
        let shadow_value = session
            .evaluate("document.querySelector('#shadow-host').shadowRoot.querySelector('#shadow-input').value")
            .expect("shadow value");
        assert_eq!(shadow_value.get("value").and_then(|v| v.as_str()), Some("secret"));

        let temp = tempfile::tempdir().expect("tempdir for upload");
        let upload_path = temp.path().join("hello.txt");
        std::fs::write(&upload_path, "file content").expect("write upload file");
        session
            .upload_file("#file-input", upload_path.to_str().unwrap())
            .expect("upload");
        let file_name = session
            .evaluate("document.getElementById('file-input').files[0].name")
            .expect("uploaded name");
        assert_eq!(file_name.get("value").and_then(|v| v.as_str()), Some("hello.txt"));

        session
            .set_cookie(&json!({
                "name": "tau_test",
                "value": "42",
                "domain": "127.0.0.1",
                "path": "/",
                "sameSite": "Lax"
            }))
            .expect("set cookie");
        let cookies = session.get_cookies(None).expect("get cookies");
        assert!(
            cookies
                .iter()
                .any(|cookie| cookie.get("name").and_then(|v| v.as_str()) == Some("tau_test")),
            "cookie not set: {cookies:?}"
        );
        session
            .delete_cookies("tau_test", "127.0.0.1", "/")
            .expect("delete cookie");
        let cookies = session.get_cookies(None).expect("get cookies 2");
        assert!(
            !cookies
                .iter()
                .any(|cookie| cookie.get("name").and_then(|v| v.as_str()) == Some("tau_test")),
            "cookie not deleted: {cookies:?}"
        );

        session.close();
        server.stop();
    }

    #[test]
    fn browser_session_downloads() {
        let browser_path = available_browser();
        if browser_path.is_empty() {
            return;
        }
        let _guard = browser_test_guard();
        let server = TestServer::start();
        let download_dir = tempfile::tempdir().expect("tempdir for downloads");
        let mut session = BrowserSession::launch(browser_path).expect("launch");
        session.navigate(&server.addr).expect("navigate");

        session
            .start_download_monitoring_in(download_dir.path())
            .expect("start monitoring");
        session.click("#download-link").expect("click download link");

        let download_seen = poll_with_timeout(
            || {
                session
                    .get_downloads()
                    .map(|downloads| {
                        downloads
                            .as_array()
                            .map(|items| {
                                items
                                    .iter()
                                    .any(|item| {
                                        item.get("state").and_then(|v| v.as_str())
                                            == Some("completed")
                                    })
                            })
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            },
            Duration::from_secs(20),
        );
        assert!(download_seen, "download never completed");

        let file_landed = poll_with_timeout(
            || download_dir.path().join("tau-test.txt").exists(),
            Duration::from_secs(10),
        );
        assert!(file_landed, "downloaded file not found in monitored directory");

        session.close();
        server.stop();
    }
}
