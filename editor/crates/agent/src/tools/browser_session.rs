use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tungstenite::WebSocket;
use tungstenite::protocol::Message as WsMessage;

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

        self.type_text("", text)
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

        let escaped_value = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        let _result = self.send_command("Runtime.callFunctionOn", json!({
            "objectId": object_id,
            "functionDeclaration": &format!("function() {{ this.value = '{}'; this.dispatchEvent(new Event('input', {{ bubbles: true }})); this.dispatchEvent(new Event('change', {{ bubbles: true }})); }}", escaped_value),
            "returnByValue": true
        }))?;

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
                return Ok(false);
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

    pub fn query_shadow_dom(&self, shadow_selector: &str, inner_selector: &str) -> Result<Value> {
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

        Ok(result.get("result").cloned().unwrap_or(json!(null)))
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
        let escaped_value = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");

        let expression = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                if (!el || !el.shadowRoot) return false;
                const inner = el.shadowRoot.querySelector('{}');
                if (!inner) return false;
                inner.focus();
                inner.value = '{}';
                inner.dispatchEvent(new Event('input', {{ bubbles: true }}));
                inner.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return true;
            }})()"#,
            escaped_shadow, escaped_inner, escaped_value
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
            anyhow::bail!("Failed to fill element in shadow DOM");
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

    pub fn is_alive(&self) -> bool {
        self.child.id() != 0
    }

    pub fn close(&mut self) {
        let alive = self.child.id() != 0;

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
        let alive = self.child.id() != 0;

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
