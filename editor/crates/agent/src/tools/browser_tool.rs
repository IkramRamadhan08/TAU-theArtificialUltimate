use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::io::{Read, Write};

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema as acp;
use anyhow::{Context, Result};
use gpui::{App, AppContext, AsyncApp, Task};
use html_to_markdown::convert_html_to_markdown;
use language_model::{LanguageModelImage, LanguageModelToolResultContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use ui::SharedString;

use super::browser_session::{BrowserSession, format_accessibility_tree};

fn global_session() -> &'static std::sync::Mutex<Option<BrowserSession>> {
    static SESSION: OnceLock<std::sync::Mutex<Option<BrowserSession>>> = OnceLock::new();
    SESSION.get_or_init(|| std::sync::Mutex::new(None))
}

#[derive(Debug, Clone)]
struct BrowserDetection {
    path: String,
    name: &'static str,
    os: &'static str,
}

impl BrowserDetection {
    fn human_name(&self) -> &str {
        self.name
    }

    fn install_command(&self) -> &str {
        match self.os {
            "linux" => "sudo apt install chromium-browser",
            "macos" => "brew install --cask chromium",
            "windows" => "https://www.google.com/chrome/",
            _ => "install a Chromium-based browser",
        }
    }
}

#[allow(clippy::disallowed_methods, reason = "Browser tool uses blocking command execution for Chrome detection")]
fn detect_os() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

#[allow(clippy::disallowed_methods, reason = "Browser tool uses blocking command execution for Chrome detection")]
fn find_browser_candidates() -> Vec<BrowserDetection> {
    let os = detect_os();
    let mut candidates = Vec::new();

    if cfg!(target_os = "macos") {
        candidates.extend([
            BrowserDetection { path: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into(), name: "Google Chrome", os },
            BrowserDetection { path: "/Applications/Chromium.app/Contents/MacOS/Chromium".into(), name: "Chromium", os },
            BrowserDetection { path: "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary".into(), name: "Chrome Canary", os },
            BrowserDetection { path: "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".into(), name: "Brave Browser", os },
            BrowserDetection { path: "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into(), name: "Microsoft Edge", os },
            BrowserDetection { path: "/Applications/Arc.app/Contents/MacOS/Arc".into(), name: "Arc", os },
        ]);
    } else if cfg!(target_os = "windows") {
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let program_files = std::env::var("PROGRAMFILES").unwrap_or_default();
        let program_files_x86 = std::env::var("PROGRAMFILES(X86)").unwrap_or_default();
        candidates.extend([
            BrowserDetection { path: format!("{}\\Google\\Chrome\\Application\\chrome.exe", local_app_data), name: "Google Chrome", os },
            BrowserDetection { path: format!("{}\\Google\\Chrome\\Application\\chrome.exe", program_files), name: "Google Chrome", os },
            BrowserDetection { path: format!("{}\\Google\\Chrome\\Application\\chrome.exe", program_files_x86), name: "Google Chrome", os },
            BrowserDetection { path: format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", program_files), name: "Microsoft Edge", os },
            BrowserDetection { path: format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", program_files_x86), name: "Microsoft Edge", os },
            BrowserDetection { path: format!("{}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", local_app_data), name: "Brave Browser", os },
        ]);
    } else {
        candidates.extend([
            BrowserDetection { path: "/usr/bin/google-chrome".into(), name: "Google Chrome", os },
            BrowserDetection { path: "/usr/bin/google-chrome-stable".into(), name: "Google Chrome", os },
            BrowserDetection { path: "/usr/bin/google-chrome-beta".into(), name: "Chrome Beta", os },
            BrowserDetection { path: "/usr/bin/google-chrome-dev".into(), name: "Chrome Dev", os },
            BrowserDetection { path: "/usr/bin/chromium".into(), name: "Chromium", os },
            BrowserDetection { path: "/usr/bin/chromium-browser".into(), name: "Chromium", os },
            BrowserDetection { path: "/snap/bin/chromium".into(), name: "Chromium (Snap)", os },
            BrowserDetection { path: "/usr/bin/org.chromium.Chromium".into(), name: "Chromium", os },
            BrowserDetection { path: "/usr/bin/brave-browser".into(), name: "Brave Browser", os },
            BrowserDetection { path: "/usr/bin/brave-browser-stable".into(), name: "Brave Browser", os },
            BrowserDetection { path: "/usr/bin/microsoft-edge".into(), name: "Microsoft Edge", os },
            BrowserDetection { path: "/usr/bin/microsoft-edge-stable".into(), name: "Microsoft Edge", os },
        ]);
    }

    candidates
}

#[allow(clippy::disallowed_methods, reason = "Browser tool uses blocking command execution for Chrome detection")]
fn find_chrome() -> Option<BrowserDetection> {
    if let Ok(custom_path) = std::env::var("TAU_CHROME_PATH") {
        let path = custom_path.trim().to_string();
        if !path.is_empty() && std::path::Path::new(&path).exists() {
            let os = detect_os();
            let detection = BrowserDetection {
                path,
                name: "Custom Browser",
                os,
            };
            if test_chrome_headless(&detection.path).is_ok() {
                return Some(detection);
            }
            log::warn!(
                "TAU_CHROME_PATH is set but browser cannot run headless: {}",
                detection.path
            );
        }
    }

    let candidates = find_browser_candidates();

    let mut found_but_failed: Vec<String> = Vec::new();

    for candidate in &candidates {
        if std::path::Path::new(&candidate.path).exists() {
            if test_chrome_headless(&candidate.path).is_ok() {
                return Some(candidate.clone());
            }
            found_but_failed.push(format!("{} ({})", candidate.human_name(), candidate.path));
        }
    }

    let which_cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
    for name in &["google-chrome", "chromium", "chromium-browser", "chrome", "brave-browser", "microsoft-edge"] {
        if let Ok(output) = std::process::Command::new(which_cmd).arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && std::path::Path::new(&path).exists() {
                    let os = detect_os();
                    let detection = BrowserDetection {
                        path,
                        name: match *name {
                            "google-chrome" | "chrome" => "Google Chrome",
                            "chromium" | "chromium-browser" => "Chromium",
                            "brave-browser" => "Brave Browser",
                            "microsoft-edge" => "Microsoft Edge",
                            _ => "Chromium-based browser",
                        },
                        os,
                    };
                    if test_chrome_headless(&detection.path).is_ok() {
                        return Some(detection);
                    }
                    found_but_failed.push(format!("{} ({})", detection.human_name(), detection.path));
                }
            }
        }
    }

    if !found_but_failed.is_empty() {
        log::warn!(
            "Found browsers but none can run headless: {}",
            found_but_failed.join(", ")
        );
    }

    None
}

#[allow(clippy::disallowed_methods, reason = "Browser tool uses blocking command execution for Chrome detection")]
fn test_chrome_headless(chrome_path: &str) -> Result<()> {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("Failed to bind test port")?;
        let p = listener.local_addr()?.port();
        drop(listener);
        p
    };

    let mut child = std::process::Command::new(chrome_path)
        .arg("--headless=new")
        .arg("--no-sandbox")
        .arg("--disable-gpu")
        .arg("--disable-dev-shm-usage")
        .arg("--disable-extensions")
        .arg(format!("--remote-debugging-port={}", port))
        .arg("about:blank")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn Chrome headless test")?;

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(ref mut pipe) = child.stderr {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stderr);
                }
                anyhow::bail!(
                    "Chrome headless test failed (exit {}): {}",
                    status.code().unwrap_or(-1),
                    stderr.lines().take(3).collect::<Vec<_>>().join(" | ")
                );
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    anyhow::bail!("Chrome headless test timed out after 10s — browser may be hanging");
                }
                if let Ok(mut probe) = std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
                    let _ = probe.write_all(b"GET /json/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
                    let mut buf = [0u8; 4096];
                    if let Ok(n) = probe.read(&mut buf) {
                        let resp = String::from_utf8_lossy(&buf[..n]);
                        if resp.contains("webSocketDebuggerUrl") {
                            let _ = child.kill();
                            return Ok(());
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                anyhow::bail!("Failed to check Chrome headless test status: {}", e);
            }
        }
    }
}

fn chrome_error(detection: Option<&BrowserDetection>) -> String {
    let os = detect_os();
    let env_hint = "\n\nTip: Set TAU_CHROME_PATH environment variable to the full path of your \
                    browser executable to bypass auto-detection.\n\
                    Example: export TAU_CHROME_PATH=/usr/bin/chromium-browser";
    match detection {
        Some(browser) => {
            format!(
                "Found {} at {} but it cannot run in headless mode.\n\n\
                 Possible causes:\n\
                 - Missing system libraries (common on headless/Linux servers)\n\
                 - Chrome is too old and does not support --headless=new (requires Chrome 112+)\n\
                 - Another Chrome instance is blocking the debug port\n\n\
                 Debug: try running this command to check:\n\
                 {} --headless=new --no-sandbox --disable-gpu --dump-dom about:blank\n\n\
                 Install command: {}\n\n\
                 Alternative: install Chromium which has fewer dependencies:\n\
                 {}{}",
                browser.human_name(),
                browser.path,
                browser.path,
                browser.install_command(),
                if os == "linux" { "sudo apt install chromium-browser" } else { browser.install_command() },
                env_hint,
            )
        }
        None => {
            let mut msg = "No Chromium-based browser found or browser cannot run in headless mode.\n\n".to_string();
            msg.push_str(&format!("Detected OS: {}\n\n", os));
            msg.push_str("Install one of these:\n");
            match os {
                "linux" => {
                    msg.push_str("  - Chromium: sudo apt install chromium-browser\n");
                    msg.push_str("  - Google Chrome: wget https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb && sudo dpkg -i google-chrome-stable_current_amd64.deb\n");
                    msg.push_str("  - Brave: sudo apt install brave-browser\n");
                }
                "macos" => {
                    msg.push_str("  - Chromium: brew install --cask chromium\n");
                    msg.push_str("  - Google Chrome: brew install --cask google-chrome\n");
                    msg.push_str("  - Brave: brew install --cask brave-browser\n");
                }
                "windows" => {
                    msg.push_str("  - Google Chrome: https://www.google.com/chrome/\n");
                    msg.push_str("  - Microsoft Edge: https://www.microsoft.com/edge\n");
                    msg.push_str("  - Brave: https://brave.com/download/\n");
                }
                _ => {
                    msg.push_str("  - Any Chromium-based browser (Chrome, Chromium, Brave, Edge)\n");
                }
            }
            msg.push_str("\nIf a browser is already installed, it may be missing headless dependencies.");
            msg.push_str(env_hint);
            msg
        }
    }
}

async fn with_session<R>(cx: &AsyncApp, chrome_path: &str, f: impl FnOnce(&mut BrowserSession) -> Result<R> + Send + 'static) -> Result<R>
where
    R: Send + 'static,
{
    let chrome = chrome_path.to_string();
    let task: Task<Result<R>> = cx.background_spawn(async move {
        let mut guard = global_session().lock().map_err(|e| anyhow::anyhow!("{}", e))?;

        if guard.is_none() || !guard.as_ref().unwrap().is_alive() {
            if let Some(mut old) = guard.take() {
                old.close();
            }
            let session = BrowserSession::launch(&chrome)?;
            *guard = Some(session);
        }

        f(guard.as_mut().unwrap())
    });

    task.await
}

async fn close_session(cx: &AsyncApp) {
    let task = cx.background_spawn(async move {
        if let Ok(mut guard) = global_session().lock() {
            if let Some(mut session) = guard.take() {
                session.close();
            }
        }
    });
    let _ = task.await;
}

fn validate_url(url: &str) -> Result<String> {
    let normalized = if !url.starts_with("http://") && !url.starts_with("https://") {
        format!("https://{}", url)
    } else {
        url.to_string()
    };

    if normalized.starts_with("file://")
        || normalized.starts_with("chrome://")
        || normalized.starts_with("chrome-extension://")
        || normalized.starts_with("data:")
        || normalized.starts_with("javascript:")
    {
        anyhow::bail!(
            "URL scheme not allowed: {}. Only http:// and https:// URLs are supported.",
            normalized.split(':').next().unwrap_or("unknown")
        );
    }

    Ok(normalized)
}

// ============================================================================
// browser_navigate
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserNavigateToolInput {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserToolOutput {
    Success {
        title: String,
        content: String,
        url: String,
    },
    Error {
        error: String,
    },
}

impl From<BrowserToolOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserToolOutput) -> Self {
        match value {
            BrowserToolOutput::Success {
                title,
                content,
                url,
            } => {
                let text = format!("# {}\n\nURL: {}\n\n{}", title, url, content);
                text.into()
            }
            BrowserToolOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserNavigateTool;

impl AgentTool for BrowserNavigateTool {
    type Input = BrowserNavigateToolInput;
    type Output = BrowserToolOutput;

    const NAME: &'static str = "browser_navigate";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Browsing the web".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserToolOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserToolOutput::Error {
                error: chrome_error(None),
            })?.path;

            let url = validate_url(&input.url).map_err(|e| BrowserToolOutput::Error {
                error: e.to_string(),
            })?;

            with_session(cx, &chrome, move |session| {
                session.navigate(&url)?;
                let title = session.get_page_title().unwrap_or_default();
                let dom = session.get_dom().unwrap_or_default();
                let content = match convert_html_to_markdown(std::io::Cursor::new(&dom), &mut []) {
                    Ok(md) => md,
                    Err(_) => dom,
                };
                let current_url = session.get_page_url().unwrap_or(url);
                Ok(BrowserToolOutput::Success {
                    title,
                    content,
                    url: current_url,
                })
            })
            .await
            .map_err(|e| BrowserToolOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_screenshot
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserScreenshotToolInput {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserScreenshotOutput {
    Success {
        screenshot_base64: String,
    },
    Error {
        error: String,
    },
}

impl From<BrowserScreenshotOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserScreenshotOutput) -> Self {
        match value {
            BrowserScreenshotOutput::Success { screenshot_base64 } => {
                LanguageModelToolResultContent::Image(LanguageModelImage {
                    source: screenshot_base64.into(),
                })
            }
            BrowserScreenshotOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserScreenshotTool;

impl AgentTool for BrowserScreenshotTool {
    type Input = BrowserScreenshotToolInput;
    type Output = BrowserScreenshotOutput;

    const NAME: &'static str = "browser_screenshot";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Taking screenshot".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserScreenshotOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserScreenshotOutput::Error {
                error: chrome_error(None),
            })?.path;

            let url = validate_url(&input.url).map_err(|e| BrowserScreenshotOutput::Error {
                error: e.to_string(),
            })?;

            with_session(cx, &chrome, move |session| {
                session.navigate(&url)?;
                let base64 = session.screenshot()?;
                Ok(BrowserScreenshotOutput::Success {
                    screenshot_base64: base64,
                })
            })
            .await
            .map_err(|e| BrowserScreenshotOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_click
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserClickToolInput {
    pub selector: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserClickOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserClickOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserClickOutput) -> Self {
        match value {
            BrowserClickOutput::Success { .. } => "Element clicked successfully".into(),
            BrowserClickOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserClickTool;

impl AgentTool for BrowserClickTool {
    type Input = BrowserClickToolInput;
    type Output = BrowserClickOutput;

    const NAME: &'static str = "browser_click";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Clicking element".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserClickOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserClickOutput::Error {
                error: chrome_error(None),
            })?.path;

            let selector = input.selector;

            with_session(cx, &chrome, move |session| {
                session.click(&selector)?;
                Ok(BrowserClickOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserClickOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_type
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserTypeToolInput {
    pub selector: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserTypeOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserTypeOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserTypeOutput) -> Self {
        match value {
            BrowserTypeOutput::Success { .. } => "Text typed successfully".into(),
            BrowserTypeOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserTypeTool;

impl AgentTool for BrowserTypeTool {
    type Input = BrowserTypeToolInput;
    type Output = BrowserTypeOutput;

    const NAME: &'static str = "browser_type";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Typing text".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserTypeOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserTypeOutput::Error {
                error: chrome_error(None),
            })?.path;

            let selector = input.selector;
            let text = input.text;

            with_session(cx, &chrome, move |session| {
                session.type_text(&selector, &text)?;
                Ok(BrowserTypeOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserTypeOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_fill
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserFillToolInput {
    pub selector: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserFillOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserFillOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserFillOutput) -> Self {
        match value {
            BrowserFillOutput::Success { .. } => "Field filled successfully".into(),
            BrowserFillOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserFillTool;

impl AgentTool for BrowserFillTool {
    type Input = BrowserFillToolInput;
    type Output = BrowserFillOutput;

    const NAME: &'static str = "browser_fill";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Filling form field".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserFillOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserFillOutput::Error {
                error: chrome_error(None),
            })?.path;

            let selector = input.selector;
            let value = input.value;

            with_session(cx, &chrome, move |session| {
                session.fill(&selector, &value)?;
                Ok(BrowserFillOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserFillOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_scroll
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserScrollToolInput {
    pub delta_x: i64,
    pub delta_y: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserScrollOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserScrollOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserScrollOutput) -> Self {
        match value {
            BrowserScrollOutput::Success { .. } => "Page scrolled successfully".into(),
            BrowserScrollOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserScrollTool;

impl AgentTool for BrowserScrollTool {
    type Input = BrowserScrollToolInput;
    type Output = BrowserScrollOutput;

    const NAME: &'static str = "browser_scroll";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Scrolling page".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserScrollOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserScrollOutput::Error {
                error: chrome_error(None),
            })?.path;

            let delta_x = input.delta_x;
            let delta_y = input.delta_y;

            with_session(cx, &chrome, move |session| {
                session.scroll(delta_x, delta_y)?;
                Ok(BrowserScrollOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserScrollOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_press_key
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserPressKeyToolInput {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserPressKeyOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserPressKeyOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserPressKeyOutput) -> Self {
        match value {
            BrowserPressKeyOutput::Success { .. } => "Key pressed successfully".into(),
            BrowserPressKeyOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserPressKeyTool;

impl AgentTool for BrowserPressKeyTool {
    type Input = BrowserPressKeyToolInput;
    type Output = BrowserPressKeyOutput;

    const NAME: &'static str = "browser_press_key";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Pressing key".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserPressKeyOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserPressKeyOutput::Error {
                error: chrome_error(None),
            })?.path;

            let key = input.key;

            with_session(cx, &chrome, move |session| {
                session.press_key(&key)?;
                Ok(BrowserPressKeyOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserPressKeyOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_wait
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserWaitToolInput {
    pub selector: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserWaitOutput {
    Success { found: bool },
    Error { error: String },
}

impl From<BrowserWaitOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserWaitOutput) -> Self {
        match value {
            BrowserWaitOutput::Success { .. } => "Element found".into(),
            BrowserWaitOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserWaitTool;

impl AgentTool for BrowserWaitTool {
    type Input = BrowserWaitToolInput;
    type Output = BrowserWaitOutput;

    const NAME: &'static str = "browser_wait";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Waiting for element".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserWaitOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserWaitOutput::Error {
                error: chrome_error(None),
            })?.path;

            let timeout = input.timeout_ms.unwrap_or(10000);
            let selector = input.selector;

            with_session(cx, &chrome, move |session| {
                session.wait_for_element(&selector, timeout)?;
                Ok(BrowserWaitOutput::Success { found: true })
            })
            .await
            .map_err(|e| BrowserWaitOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_close
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserCloseToolInput {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserCloseOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserCloseOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserCloseOutput) -> Self {
        match value {
            BrowserCloseOutput::Success { .. } => "Browser closed".into(),
            BrowserCloseOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserCloseTool;

impl AgentTool for BrowserCloseTool {
    type Input = BrowserCloseToolInput;
    type Output = BrowserCloseOutput;

    const NAME: &'static str = "browser_close";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Closing browser".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        _input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            close_session(cx).await;
            Ok(BrowserCloseOutput::Success { success: true })
        })
    }
}

// ============================================================================
// browser_accessibility_tree
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserAccessibilityTreeToolInput {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserAccessibilityTreeOutput {
    Success { tree: String },
    Error { error: String },
}

impl From<BrowserAccessibilityTreeOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserAccessibilityTreeOutput) -> Self {
        match value {
            BrowserAccessibilityTreeOutput::Success { tree } => tree.into(),
            BrowserAccessibilityTreeOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserAccessibilityTreeTool;

impl AgentTool for BrowserAccessibilityTreeTool {
    type Input = BrowserAccessibilityTreeToolInput;
    type Output = BrowserAccessibilityTreeOutput;

    const NAME: &'static str = "browser_accessibility_tree";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Getting accessibility tree".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        _input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let chrome = find_chrome().ok_or_else(|| BrowserAccessibilityTreeOutput::Error {
                error: chrome_error(None),
            })?.path;

            with_session(cx, &chrome, move |session| {
                let tree = session.get_accessibility_tree()?;
                let formatted = format_accessibility_tree(&tree);
                Ok(BrowserAccessibilityTreeOutput::Success { tree: formatted })
            })
            .await
            .map_err(|e| BrowserAccessibilityTreeOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_click_by_index
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserClickByIndexToolInput {
    pub index: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserClickByIndexOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserClickByIndexOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserClickByIndexOutput) -> Self {
        match value {
            BrowserClickByIndexOutput::Success { .. } => "Element clicked successfully".into(),
            BrowserClickByIndexOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserClickByIndexTool;

impl AgentTool for BrowserClickByIndexTool {
    type Input = BrowserClickByIndexToolInput;
    type Output = BrowserClickByIndexOutput;

    const NAME: &'static str = "browser_click_by_index";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Clicking element by index".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserClickByIndexOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserClickByIndexOutput::Error {
                error: chrome_error(None),
            })?.path;

            let index = input.index;

            with_session(cx, &chrome, move |session| {
                session.click_by_index(index)?;
                Ok(BrowserClickByIndexOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserClickByIndexOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_type_by_index
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserTypeByIndexToolInput {
    pub index: usize,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserTypeByIndexOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserTypeByIndexOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserTypeByIndexOutput) -> Self {
        match value {
            BrowserTypeByIndexOutput::Success { .. } => "Text typed successfully".into(),
            BrowserTypeByIndexOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserTypeByIndexTool;

impl AgentTool for BrowserTypeByIndexTool {
    type Input = BrowserTypeByIndexToolInput;
    type Output = BrowserTypeByIndexOutput;

    const NAME: &'static str = "browser_type_by_index";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Typing text by index".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserTypeByIndexOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserTypeByIndexOutput::Error {
                error: chrome_error(None),
            })?.path;

            let index = input.index;
            let text = input.text;

            with_session(cx, &chrome, move |session| {
                session.type_by_index(index, &text)?;
                Ok(BrowserTypeByIndexOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserTypeByIndexOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_fill_by_index
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserFillByIndexToolInput {
    pub index: usize,
    pub value: String,
    pub clear: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserFillByIndexOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserFillByIndexOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserFillByIndexOutput) -> Self {
        match value {
            BrowserFillByIndexOutput::Success { .. } => "Field filled successfully".into(),
            BrowserFillByIndexOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserFillByIndexTool;

impl AgentTool for BrowserFillByIndexTool {
    type Input = BrowserFillByIndexToolInput;
    type Output = BrowserFillByIndexOutput;

    const NAME: &'static str = "browser_fill_by_index";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Filling form field by index".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserFillByIndexOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserFillByIndexOutput::Error {
                error: chrome_error(None),
            })?.path;

            let index = input.index;
            let value = input.value;
            let clear = input.clear.unwrap_or(false);

            with_session(cx, &chrome, move |session| {
                session.fill_by_index(index, &value, clear)?;
                Ok(BrowserFillByIndexOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserFillByIndexOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_evaluate
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserEvaluateToolInput {
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserEvaluateOutput {
    Success { result: serde_json::Value },
    Error { error: String },
}

impl From<BrowserEvaluateOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserEvaluateOutput) -> Self {
        match value {
            BrowserEvaluateOutput::Success { result } => result.to_string().into(),
            BrowserEvaluateOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserEvaluateTool;

impl AgentTool for BrowserEvaluateTool {
    type Input = BrowserEvaluateToolInput;
    type Output = BrowserEvaluateOutput;

    const NAME: &'static str = "browser_evaluate";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Evaluating JavaScript".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserEvaluateOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserEvaluateOutput::Error {
                error: chrome_error(None),
            })?.path;

            let expression = input.expression;

            with_session(cx, &chrome, move |session| {
                let result = session.evaluate(&expression)?;
                Ok(BrowserEvaluateOutput::Success { result })
            })
            .await
            .map_err(|e| BrowserEvaluateOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_tabs
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserTabsToolInput {
    pub action: String,
    pub target_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserTabsOutput {
    Success { tabs: Vec<super::browser_session::TabInfo> },
    Error { error: String },
}

impl From<BrowserTabsOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserTabsOutput) -> Self {
        match value {
            BrowserTabsOutput::Success { tabs } => {
                let text = tabs
                    .iter()
                    .map(|tab| format!("- {} ({}): {}", tab.target_id, tab.title, tab.url))
                    .collect::<Vec<_>>()
                    .join("\n");
                text.into()
            }
            BrowserTabsOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserTabsTool;

impl AgentTool for BrowserTabsTool {
    type Input = BrowserTabsToolInput;
    type Output = BrowserTabsOutput;

    const NAME: &'static str = "browser_tabs";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Managing browser tabs".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserTabsOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserTabsOutput::Error {
                error: chrome_error(None),
            })?.path;

            let action = input.action;
            let target_id = input.target_id;

            with_session(cx, &chrome, move |session| {
                match action.as_str() {
                    "list" => {
                        let tabs = session.get_tabs()?;
                        Ok(BrowserTabsOutput::Success { tabs })
                    }
                    "switch" => {
                        let id = target_id.ok_or_else(|| anyhow::anyhow!("target_id required for switch"))?;
                        session.switch_tab(&id)?;
                        Ok(BrowserTabsOutput::Success { tabs: vec![] })
                    }
                    "close" => {
                        let id = target_id.ok_or_else(|| anyhow::anyhow!("target_id required for close"))?;
                        session.close_tab(&id)?;
                        Ok(BrowserTabsOutput::Success { tabs: vec![] })
                    }
                    _ => Err(anyhow::anyhow!("Unknown action: {}. Use 'list', 'switch', or 'close'", action))
                }
            })
            .await
            .map_err(|e| BrowserTabsOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_page_info
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserPageInfoToolInput {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserPageInfoOutput {
    Success { info: serde_json::Value },
    Error { error: String },
}

impl From<BrowserPageInfoOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserPageInfoOutput) -> Self {
        match value {
            BrowserPageInfoOutput::Success { info } => info.to_string().into(),
            BrowserPageInfoOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserPageInfoTool;

impl AgentTool for BrowserPageInfoTool {
    type Input = BrowserPageInfoToolInput;
    type Output = BrowserPageInfoOutput;

    const NAME: &'static str = "browser_page_info";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Getting page info".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        _input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let chrome = find_chrome().ok_or_else(|| BrowserPageInfoOutput::Error {
                error: chrome_error(None),
            })?.path;

            with_session(cx, &chrome, move |session| {
                let info = session.get_page_info()?;
                Ok(BrowserPageInfoOutput::Success { info })
            })
            .await
            .map_err(|e| BrowserPageInfoOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_click_at_xy
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserClickAtXyToolInput {
    pub x: f64,
    pub y: f64,
    pub button: Option<String>,
    pub clicks: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserClickAtXyOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserClickAtXyOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserClickAtXyOutput) -> Self {
        match value {
            BrowserClickAtXyOutput::Success { .. } => "Clicked at coordinates successfully".into(),
            BrowserClickAtXyOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserClickAtXyTool;

impl AgentTool for BrowserClickAtXyTool {
    type Input = BrowserClickAtXyToolInput;
    type Output = BrowserClickAtXyOutput;

    const NAME: &'static str = "browser_click_at_xy";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Clicking at coordinates".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserClickAtXyOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserClickAtXyOutput::Error {
                error: chrome_error(None),
            })?.path;

            let x = input.x;
            let y = input.y;
            let button = input.button.unwrap_or_else(|| "left".into());
            let clicks = input.clicks.unwrap_or(1);

            with_session(cx, &chrome, move |session| {
                session.click_at_xy(x, y, &button, clicks)?;
                Ok(BrowserClickAtXyOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserClickAtXyOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_wait_for_load
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserWaitForLoadToolInput {
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserWaitForLoadOutput {
    Success { loaded: bool },
    Error { error: String },
}

impl From<BrowserWaitForLoadOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserWaitForLoadOutput) -> Self {
        match value {
            BrowserWaitForLoadOutput::Success { loaded } => {
                if loaded {
                    "Page loaded successfully".into()
                } else {
                    "Page load timed out".into()
                }
            }
            BrowserWaitForLoadOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserWaitForLoadTool;

impl AgentTool for BrowserWaitForLoadTool {
    type Input = BrowserWaitForLoadToolInput;
    type Output = BrowserWaitForLoadOutput;

    const NAME: &'static str = "browser_wait_for_load";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Waiting for page load".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserWaitForLoadOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserWaitForLoadOutput::Error {
                error: chrome_error(None),
            })?.path;

            let timeout = input.timeout_ms.unwrap_or(15000);

            with_session(cx, &chrome, move |session| {
                let loaded = session.wait_for_load(timeout)?;
                Ok(BrowserWaitForLoadOutput::Success { loaded })
            })
            .await
            .map_err(|e| BrowserWaitForLoadOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_wait_for_network_idle
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserWaitForNetworkIdleToolInput {
    pub timeout_ms: Option<u64>,
    pub idle_ms: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserWaitForNetworkIdleOutput {
    Success { idle: bool },
    Error { error: String },
}

impl From<BrowserWaitForNetworkIdleOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserWaitForNetworkIdleOutput) -> Self {
        match value {
            BrowserWaitForNetworkIdleOutput::Success { idle } => {
                if idle {
                    "Network is idle".into()
                } else {
                    "Network idle timed out".into()
                }
            }
            BrowserWaitForNetworkIdleOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserWaitForNetworkIdleTool;

impl AgentTool for BrowserWaitForNetworkIdleTool {
    type Input = BrowserWaitForNetworkIdleToolInput;
    type Output = BrowserWaitForNetworkIdleOutput;

    const NAME: &'static str = "browser_wait_for_network_idle";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Waiting for network idle".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserWaitForNetworkIdleOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserWaitForNetworkIdleOutput::Error {
                error: chrome_error(None),
            })?.path;

            let timeout = input.timeout_ms.unwrap_or(10000);
            let idle = input.idle_ms.unwrap_or(500);

            with_session(cx, &chrome, move |session| {
                let is_idle = session.wait_for_network_idle(timeout, idle)?;
                Ok(BrowserWaitForNetworkIdleOutput::Success { idle: is_idle })
            })
            .await
            .map_err(|e| BrowserWaitForNetworkIdleOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_upload_file
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserUploadFileToolInput {
    pub selector: String,
    pub file_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserUploadFileOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserUploadFileOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserUploadFileOutput) -> Self {
        match value {
            BrowserUploadFileOutput::Success { .. } => "File uploaded successfully".into(),
            BrowserUploadFileOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserUploadFileTool;

impl AgentTool for BrowserUploadFileTool {
    type Input = BrowserUploadFileToolInput;
    type Output = BrowserUploadFileOutput;

    const NAME: &'static str = "browser_upload_file";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Uploading file".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserUploadFileOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserUploadFileOutput::Error {
                error: chrome_error(None),
            })?.path;

            let selector = input.selector;
            let file_path = input.file_path;

            with_session(cx, &chrome, move |session| {
                session.upload_file(&selector, &file_path)?;
                Ok(BrowserUploadFileOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserUploadFileOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_dispatch_key
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserDispatchKeyToolInput {
    pub selector: String,
    pub key: String,
    pub event_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserDispatchKeyOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserDispatchKeyOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserDispatchKeyOutput) -> Self {
        match value {
            BrowserDispatchKeyOutput::Success { .. } => "Key event dispatched successfully".into(),
            BrowserDispatchKeyOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserDispatchKeyTool;

impl AgentTool for BrowserDispatchKeyTool {
    type Input = BrowserDispatchKeyToolInput;
    type Output = BrowserDispatchKeyOutput;

    const NAME: &'static str = "browser_dispatch_key";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Dispatching key event".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserDispatchKeyOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserDispatchKeyOutput::Error {
                error: chrome_error(None),
            })?.path;

            let selector = input.selector;
            let key = input.key;
            let event_type = input.event_type.unwrap_or_else(|| "keypress".into());

            with_session(cx, &chrome, move |session| {
                session.dispatch_key_event(&selector, &key, &event_type)?;
                Ok(BrowserDispatchKeyOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserDispatchKeyOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_iframe
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserIframeToolInput {
    pub url_substring: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserIframeOutput {
    Success { iframe: Option<super::browser_session::TabInfo> },
    Error { error: String },
}

impl From<BrowserIframeOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserIframeOutput) -> Self {
        match value {
            BrowserIframeOutput::Success { iframe } => {
                match iframe {
                    Some(info) => format!("Found iframe: {} ({})", info.title, info.url).into(),
                    None => "No iframe found matching the URL substring".into(),
                }
            }
            BrowserIframeOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserIframeTool;

impl AgentTool for BrowserIframeTool {
    type Input = BrowserIframeToolInput;
    type Output = BrowserIframeOutput;

    const NAME: &'static str = "browser_iframe";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Finding iframe".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserIframeOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserIframeOutput::Error {
                error: chrome_error(None),
            })?.path;

            let url_substring = input.url_substring;

            with_session(cx, &chrome, move |session| {
                let iframes = session.get_iframe_targets()?;
                let found = iframes.iter().find(|iframe| iframe.url.contains(&url_substring)).cloned();
                Ok(BrowserIframeOutput::Success { iframe: found })
            })
            .await
            .map_err(|e| BrowserIframeOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_skill_read
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserSkillReadToolInput {
    pub site: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserSkillReadOutput {
    Success { skills: Vec<super::browser_skills::DomainSkill> },
    Error { error: String },
}

impl From<BrowserSkillReadOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserSkillReadOutput) -> Self {
        match value {
            BrowserSkillReadOutput::Success { skills } => {
                if skills.is_empty() {
                    "No skills found for this site".into()
                } else {
                    let text = skills
                        .iter()
                        .map(|s| format!("## {}\n\n{}", s.name, s.content))
                        .collect::<Vec<_>>()
                        .join("\n\n---\n\n");
                    text.into()
                }
            }
            BrowserSkillReadOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserSkillReadTool;

impl AgentTool for BrowserSkillReadTool {
    type Input = BrowserSkillReadToolInput;
    type Output = BrowserSkillReadOutput;

    const NAME: &'static str = "browser_skill_read";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Reading domain skills".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserSkillReadOutput::Error {
                error: e.to_string(),
            })?;

            let skills = super::browser_skills::get_skills_for_site(&input.site)
                .map_err(|e| BrowserSkillReadOutput::Error { error: e.to_string() })?;

            let filtered = if let Some(name) = input.name {
                skills.into_iter().filter(|s| s.name == name).collect()
            } else {
                skills
            };

            Ok(BrowserSkillReadOutput::Success { skills: filtered })
        })
    }
}

// ============================================================================
// browser_skill_write
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserSkillWriteToolInput {
    pub site: String,
    pub name: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserSkillWriteOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserSkillWriteOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserSkillWriteOutput) -> Self {
        match value {
            BrowserSkillWriteOutput::Success { .. } => "Skill saved successfully".into(),
            BrowserSkillWriteOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserSkillWriteTool;

impl AgentTool for BrowserSkillWriteTool {
    type Input = BrowserSkillWriteToolInput;
    type Output = BrowserSkillWriteOutput;

    const NAME: &'static str = "browser_skill_write";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Writing domain skill".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserSkillWriteOutput::Error {
                error: e.to_string(),
            })?;

            super::browser_skills::save_skill(&input.site, &input.name, &input.content)
                .map_err(|e| BrowserSkillWriteOutput::Error { error: e.to_string() })?;

            Ok(BrowserSkillWriteOutput::Success { success: true })
        })
    }
}

// ============================================================================
// browser_skill_list
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserSkillListToolInput {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserSkillListOutput {
    Success { sites: Vec<String> },
    Error { error: String },
}

impl From<BrowserSkillListOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserSkillListOutput) -> Self {
        match value {
            BrowserSkillListOutput::Success { sites } => {
                if sites.is_empty() {
                    "No domain skills saved yet".into()
                } else {
                    let text = sites
                        .iter()
                        .map(|s| format!("- {}", s))
                        .collect::<Vec<_>>()
                        .join("\n");
                    text.into()
                }
            }
            BrowserSkillListOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserSkillListTool;

impl AgentTool for BrowserSkillListTool {
    type Input = BrowserSkillListToolInput;
    type Output = BrowserSkillListOutput;

    const NAME: &'static str = "browser_skill_list";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Listing domain skills".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        _input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let sites = super::browser_skills::list_sites()
                .map_err(|e| BrowserSkillListOutput::Error { error: e.to_string() })?;

            Ok(BrowserSkillListOutput::Success { sites })
        })
    }
}

// ============================================================================
// browser_cookies_get
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserCookiesGetToolInput {
    pub urls: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserCookiesGetOutput {
    Success { cookies: Vec<serde_json::Value> },
    Error { error: String },
}

impl From<BrowserCookiesGetOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserCookiesGetOutput) -> Self {
        match value {
            BrowserCookiesGetOutput::Success { cookies } => {
                if cookies.is_empty() {
                    "No cookies found".into()
                } else {
                    serde_json::to_string_pretty(&cookies).unwrap_or_default().into()
                }
            }
            BrowserCookiesGetOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserCookiesGetTool;

impl AgentTool for BrowserCookiesGetTool {
    type Input = BrowserCookiesGetToolInput;
    type Output = BrowserCookiesGetOutput;

    const NAME: &'static str = "browser_cookies_get";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Getting cookies".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserCookiesGetOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserCookiesGetOutput::Error {
                error: chrome_error(None),
            })?.path;

            with_session(cx, &chrome, move |session| {
                let cookies = session.get_cookies(input.urls)?;
                Ok(BrowserCookiesGetOutput::Success { cookies })
            })
            .await
            .map_err(|e| BrowserCookiesGetOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_cookies_set
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserCookiesSetToolInput {
    pub cookie: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserCookiesSetOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserCookiesSetOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserCookiesSetOutput) -> Self {
        match value {
            BrowserCookiesSetOutput::Success { .. } => "Cookie set successfully".into(),
            BrowserCookiesSetOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserCookiesSetTool;

impl AgentTool for BrowserCookiesSetTool {
    type Input = BrowserCookiesSetToolInput;
    type Output = BrowserCookiesSetOutput;

    const NAME: &'static str = "browser_cookies_set";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Setting cookie".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserCookiesSetOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserCookiesSetOutput::Error {
                error: chrome_error(None),
            })?.path;

            with_session(cx, &chrome, move |session| {
                session.set_cookie(&input.cookie)?;
                Ok(BrowserCookiesSetOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserCookiesSetOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_cookies_delete
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserCookiesDeleteToolInput {
    pub name: String,
    pub domain: String,
    pub path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserCookiesDeleteOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserCookiesDeleteOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserCookiesDeleteOutput) -> Self {
        match value {
            BrowserCookiesDeleteOutput::Success { .. } => "Cookie deleted successfully".into(),
            BrowserCookiesDeleteOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserCookiesDeleteTool;

impl AgentTool for BrowserCookiesDeleteTool {
    type Input = BrowserCookiesDeleteToolInput;
    type Output = BrowserCookiesDeleteOutput;

    const NAME: &'static str = "browser_cookies_delete";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Deleting cookie".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserCookiesDeleteOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserCookiesDeleteOutput::Error {
                error: chrome_error(None),
            })?.path;

            let path = input.path.unwrap_or_else(|| "/".into());

            with_session(cx, &chrome, move |session| {
                session.delete_cookies(&input.name, &input.domain, &path)?;
                Ok(BrowserCookiesDeleteOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserCookiesDeleteOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_shadow_dom_query
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserShadowDomQueryToolInput {
    pub shadow_selector: String,
    pub inner_selector: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserShadowDomQueryOutput {
    Success { result: String },
    Error { error: String },
}

impl From<BrowserShadowDomQueryOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserShadowDomQueryOutput) -> Self {
        match value {
            BrowserShadowDomQueryOutput::Success { result } => result.into(),
            BrowserShadowDomQueryOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserShadowDomQueryTool;

impl AgentTool for BrowserShadowDomQueryTool {
    type Input = BrowserShadowDomQueryToolInput;
    type Output = BrowserShadowDomQueryOutput;

    const NAME: &'static str = "browser_shadow_dom_query";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Querying shadow DOM".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserShadowDomQueryOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserShadowDomQueryOutput::Error {
                error: chrome_error(None),
            })?.path;

            with_session(cx, &chrome, move |session| {
                let result = session.query_shadow_dom(&input.shadow_selector, &input.inner_selector)?;
                let html = result.get("result")
                    .and_then(|v| v.get("value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Element not found")
                    .to_string();
                Ok(BrowserShadowDomQueryOutput::Success { result: html })
            })
            .await
            .map_err(|e| BrowserShadowDomQueryOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_shadow_dom_click
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserShadowDomClickToolInput {
    pub shadow_selector: String,
    pub inner_selector: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserShadowDomClickOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserShadowDomClickOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserShadowDomClickOutput) -> Self {
        match value {
            BrowserShadowDomClickOutput::Success { .. } => "Clicked in shadow DOM".into(),
            BrowserShadowDomClickOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserShadowDomClickTool;

impl AgentTool for BrowserShadowDomClickTool {
    type Input = BrowserShadowDomClickToolInput;
    type Output = BrowserShadowDomClickOutput;

    const NAME: &'static str = "browser_shadow_dom_click";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Clicking in shadow DOM".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserShadowDomClickOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserShadowDomClickOutput::Error {
                error: chrome_error(None),
            })?.path;

            with_session(cx, &chrome, move |session| {
                session.click_in_shadow_dom(&input.shadow_selector, &input.inner_selector)?;
                Ok(BrowserShadowDomClickOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserShadowDomClickOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_shadow_dom_fill
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserShadowDomFillToolInput {
    pub shadow_selector: String,
    pub inner_selector: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserShadowDomFillOutput {
    Success { success: bool },
    Error { error: String },
}

impl From<BrowserShadowDomFillOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserShadowDomFillOutput) -> Self {
        match value {
            BrowserShadowDomFillOutput::Success { .. } => "Filled in shadow DOM".into(),
            BrowserShadowDomFillOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserShadowDomFillTool;

impl AgentTool for BrowserShadowDomFillTool {
    type Input = BrowserShadowDomFillToolInput;
    type Output = BrowserShadowDomFillOutput;

    const NAME: &'static str = "browser_shadow_dom_fill";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Filling in shadow DOM".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserShadowDomFillOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserShadowDomFillOutput::Error {
                error: chrome_error(None),
            })?.path;

            with_session(cx, &chrome, move |session| {
                session.fill_in_shadow_dom(&input.shadow_selector, &input.inner_selector, &input.value)?;
                Ok(BrowserShadowDomFillOutput::Success { success: true })
            })
            .await
            .map_err(|e| BrowserShadowDomFillOutput::Error { error: e.to_string() })
        })
    }
}

// ============================================================================
// browser_downloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserDownloadsToolInput {
    pub action: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserDownloadsOutput {
    Success { result: serde_json::Value },
    Error { error: String },
}

impl From<BrowserDownloadsOutput> for LanguageModelToolResultContent {
    fn from(value: BrowserDownloadsOutput) -> Self {
        match value {
            BrowserDownloadsOutput::Success { result } => result.to_string().into(),
            BrowserDownloadsOutput::Error { error } => error.into(),
        }
    }
}

pub struct BrowserDownloadsTool;

impl AgentTool for BrowserDownloadsTool {
    type Input = BrowserDownloadsToolInput;
    type Output = BrowserDownloadsOutput;

    const NAME: &'static str = "browser_downloads";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Fetch
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Managing downloads".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(|e| BrowserDownloadsOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserDownloadsOutput::Error {
                error: chrome_error(None),
            })?.path;

            let action = input.action.unwrap_or_else(|| "list".into());

            with_session(cx, &chrome, move |session| {
                match action.as_str() {
                    "start" => {
                        session.start_download_monitoring()?;
                        Ok(BrowserDownloadsOutput::Success { result: serde_json::json!({"status": "monitoring started"}) })
                    }
                    "list" => {
                        let downloads = session.get_downloads()?;
                        Ok(BrowserDownloadsOutput::Success { result: downloads })
                    }
                    _ => Err(anyhow::anyhow!("Unknown action: {}. Use 'start' or 'list'", action))
                }
            })
            .await
            .map_err(|e| BrowserDownloadsOutput::Error { error: e.to_string() })
        })
    }
}
