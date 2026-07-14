use std::sync::Arc;
use std::sync::OnceLock;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema as acp;
use anyhow::Result;
use gpui::{App, Task};
use html_to_markdown::convert_html_to_markdown;
use language_model::{LanguageModelImage, LanguageModelToolResultContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::SharedString;

use super::browser_session::BrowserSession;

fn global_session() -> &'static std::sync::Mutex<Option<BrowserSession>> {
    static SESSION: OnceLock<std::sync::Mutex<Option<BrowserSession>>> = OnceLock::new();
    SESSION.get_or_init(|| std::sync::Mutex::new(None))
}

fn with_session<R>(chrome_path: &str, f: impl FnOnce(&mut BrowserSession) -> Result<R> + Send + 'static) -> Result<R>
where
    R: Send + 'static,
{
    let chrome = chrome_path.to_string();
    let (tx, rx) = async_channel::bounded(1);

    std::thread::spawn(move || {
        let result = (|| -> Result<R> {
            let mut guard = global_session().lock().map_err(|e| anyhow::anyhow!("{}", e))?;

            if guard.is_none() || !guard.as_ref().unwrap().is_alive() {
                if let Some(mut old) = guard.take() {
                    old.close();
                }
                let session = BrowserSession::launch(&chrome)?;
                *guard = Some(session);
            }

            f(guard.as_mut().unwrap())
        })();

        let _ = tx.send_blocking(result);
    });

    rx.recv_blocking().map_err(|e| anyhow::anyhow!("{}", e))?
}

fn close_session_sync() {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        if let Ok(mut guard) = global_session().lock() {
            if let Some(mut session) = guard.take() {
                session.close();
            }
        }
        let _ = tx.send_blocking(());
    });
    let _ = rx.recv_blocking();
}

#[allow(clippy::disallowed_methods, reason = "Browser tool uses blocking command execution for Chrome detection")]
fn find_chrome() -> Option<String> {
    let candidates: Vec<String> = if cfg!(target_os = "macos") {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into(),
            "/Applications/Chromium.app/Contents/MacOS/Chromium".into(),
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary".into(),
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".into(),
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into(),
        ]
    } else if cfg!(target_os = "windows") {
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let program_files = std::env::var("PROGRAMFILES").unwrap_or_default();
        let program_files_x86 = std::env::var("PROGRAMFILES(X86)").unwrap_or_default();
        vec![
            format!("{}\\Google\\Chrome\\Application\\chrome.exe", local_app_data),
            format!("{}\\Google\\Chrome\\Application\\chrome.exe", program_files),
            format!("{}\\Google\\Chrome\\Application\\chrome.exe", program_files_x86),
            format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", program_files),
            format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", program_files_x86),
            format!("{}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", local_app_data),
        ]
    } else {
        vec![
            "/usr/bin/google-chrome".into(),
            "/usr/bin/google-chrome-stable".into(),
            "/usr/bin/google-chrome-beta".into(),
            "/usr/bin/google-chrome-dev".into(),
            "/usr/bin/chromium".into(),
            "/usr/bin/chromium-browser".into(),
            "/snap/bin/chromium".into(),
            "/usr/bin/org.chromium.Chromium".into(),
            "/usr/bin/brave-browser".into(),
            "/usr/bin/brave-browser-stable".into(),
            "/usr/bin/microsoft-edge".into(),
            "/usr/bin/microsoft-edge-stable".into(),
        ]
    };

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.clone());
        }
    }

    let which_cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
    for name in &["google-chrome", "chromium", "chromium-browser", "chrome", "brave-browser", "microsoft-edge"] {
        if let Ok(output) = std::process::Command::new(which_cmd).arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && std::path::Path::new(&path).exists() {
                    return Some(path);
                }
            }
        }
    }

    None
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

fn chrome_error() -> String {
    "No Chrome/Chromium browser found. Install Google Chrome, Chromium, or Brave:\n  - Linux: sudo apt install chromium-browser\n  - macOS: brew install --cask chromium\n  - Windows: https://www.google.com/chrome/".into()
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserToolOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserToolOutput::Error {
                error: chrome_error(),
            })?;

            let url = validate_url(&input.url).map_err(|e| BrowserToolOutput::Error {
                error: e.to_string(),
            })?;

            with_session(&chrome, move |session| {
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserScreenshotOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserScreenshotOutput::Error {
                error: chrome_error(),
            })?;

            let url = validate_url(&input.url).map_err(|e| BrowserScreenshotOutput::Error {
                error: e.to_string(),
            })?;

            with_session(&chrome, move |session| {
                session.navigate(&url)?;
                let base64 = session.screenshot()?;
                Ok(BrowserScreenshotOutput::Success {
                    screenshot_base64: base64,
                })
            })
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserClickOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserClickOutput::Error {
                error: chrome_error(),
            })?;

            let selector = input.selector;

            with_session(&chrome, move |session| {
                session.click(&selector)?;
                Ok(BrowserClickOutput::Success { success: true })
            })
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserTypeOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserTypeOutput::Error {
                error: chrome_error(),
            })?;

            let selector = input.selector;
            let text = input.text;

            with_session(&chrome, move |session| {
                session.type_text(&selector, &text)?;
                Ok(BrowserTypeOutput::Success { success: true })
            })
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserFillOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserFillOutput::Error {
                error: chrome_error(),
            })?;

            let selector = input.selector;
            let value = input.value;

            with_session(&chrome, move |session| {
                session.fill(&selector, &value)?;
                Ok(BrowserFillOutput::Success { success: true })
            })
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserScrollOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserScrollOutput::Error {
                error: chrome_error(),
            })?;

            let delta_x = input.delta_x;
            let delta_y = input.delta_y;

            with_session(&chrome, move |session| {
                session.scroll(delta_x, delta_y)?;
                Ok(BrowserScrollOutput::Success { success: true })
            })
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserPressKeyOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserPressKeyOutput::Error {
                error: chrome_error(),
            })?;

            let key = input.key;

            with_session(&chrome, move |session| {
                session.press_key(&key)?;
                Ok(BrowserPressKeyOutput::Success { success: true })
            })
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
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| BrowserWaitOutput::Error {
                error: e.to_string(),
            })?;

            let chrome = find_chrome().ok_or_else(|| BrowserWaitOutput::Error {
                error: chrome_error(),
            })?;

            let timeout = input.timeout_ms.unwrap_or(10000);
            let selector = input.selector;

            with_session(&chrome, move |session| {
                session.wait_for_element(&selector, timeout)?;
                Ok(BrowserWaitOutput::Success { found: true })
            })
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
        cx.spawn(async move |_cx| {
            close_session_sync();
            Ok(BrowserCloseOutput::Success { success: true })
        })
    }
}
