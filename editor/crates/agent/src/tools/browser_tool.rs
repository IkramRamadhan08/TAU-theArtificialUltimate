use std::io::Cursor;
use std::sync::Arc;
use std::sync::OnceLock;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema as acp;
use anyhow::Result;
use base64::Engine;
use gpui::{App, Task};
use html_to_markdown::convert_html_to_markdown;
use language_model::{LanguageModelImage, LanguageModelToolResultContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smol::process::Command;
use std::time::Duration;
use ui::SharedString;

const CHROME_TIMEOUT: Duration = Duration::from_secs(30);
const WINDOW_SIZE: &str = "1280,720";

static CACHED_CHROME_PATH: OnceLock<Option<String>> = OnceLock::new();

fn find_chrome() -> Option<String> {
    CACHED_CHROME_PATH
        .get_or_init(|| find_chrome_inner())
        .clone()
}

fn find_chrome_inner() -> Option<String> {
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

fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<title>") {
        let after = &html[start + 7..];
        if let Some(end) = after.find("</title>").or_else(|| after.find("</TITLE>")) {
            return after[..end].trim().to_string();
        }
    }
    String::new()
}

/// Navigate to a URL and return the rendered page content as markdown.
/// JavaScript is executed, so you get the full dynamic page content.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserNavigateToolInput {
    /// The URL to navigate to (e.g. https://example.com)
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
            let input = input
                .recv()
                .await
                .map_err(|e| BrowserToolOutput::Error {
                    error: e.to_string(),
                })?;

            let chrome = find_chrome().ok_or_else(|| BrowserToolOutput::Error {
                error: "No Chrome/Chromium browser found. Install Google Chrome, Chromium, or Brave:\n  - Linux: sudo apt install chromium-browser\n  - macOS: brew install --cask chromium\n  - Windows: https://www.google.com/chrome/".into(),
            })?;

            let url = validate_url(&input.url).map_err(|e| BrowserToolOutput::Error {
                error: e.to_string(),
            })?;

            match run_chrome_dump_dom(&chrome, &url).await {
                Ok((title, content)) => Ok(BrowserToolOutput::Success {
                    title,
                    content,
                    url,
                }),
                Err(e) => Err(BrowserToolOutput::Error {
                    error: e.to_string(),
                }),
            }
        })
    }
}

async fn run_chrome_dump_dom(chrome: &str, url: &str) -> Result<(String, String)> {
    let mut child = Command::new(chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--disable-dev-shm-usage")
        .arg("--disable-extensions")
        .arg("--window-size")
        .arg(WINDOW_SIZE)
        .arg("--dump-dom")
        .arg(url)
        .stdout(smol::process::Stdio::piped())
        .stderr(smol::process::Stdio::piped())
        .spawn()?;

    let output = smol::future::or(
        child.output(),
        async {
            smol::Timer::after(CHROME_TIMEOUT).await;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("Chrome timed out after {}s", CHROME_TIMEOUT.as_secs()),
            ))
        },
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Chrome exited with error: {}", stderr.trim());
    }

    let html = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if html.is_empty() {
        anyhow::bail!("Chrome returned empty page content");
    }

    let title = extract_title(&html);

    let content = match convert_html_to_markdown(Cursor::new(&html), &mut []) {
        Ok(md) => md,
        Err(_) => html,
    };

    Ok((title, content))
}

/// Take a screenshot of a URL and return it as a PNG image.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserScreenshotToolInput {
    /// The URL to take a screenshot of
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
            let input = input
                .recv()
                .await
                .map_err(|e| BrowserScreenshotOutput::Error {
                    error: e.to_string(),
                })?;

            let chrome = find_chrome().ok_or_else(|| BrowserScreenshotOutput::Error {
                error: "No Chrome/Chromium browser found. Install Google Chrome, Chromium, or Brave:\n  - Linux: sudo apt install chromium-browser\n  - macOS: brew install --cask chromium\n  - Windows: https://www.google.com/chrome/".into(),
            })?;

            let url = validate_url(&input.url).map_err(|e| BrowserScreenshotOutput::Error {
                error: e.to_string(),
            })?;

            match run_chrome_screenshot(&chrome, &url).await {
                Ok(base64_data) => Ok(BrowserScreenshotOutput::Success {
                    screenshot_base64: base64_data,
                }),
                Err(e) => Err(BrowserScreenshotOutput::Error {
                    error: e.to_string(),
                }),
            }
        })
    }
}

async fn run_chrome_screenshot(chrome: &str, url: &str) -> Result<String> {
    let temp_dir = tempfile::tempdir()?;
    let screenshot_path = temp_dir.path().join("screenshot.png");

    let mut child = Command::new(chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--disable-dev-shm-usage")
        .arg("--disable-extensions")
        .arg("--window-size")
        .arg(WINDOW_SIZE)
        .arg(format!("--screenshot={}", screenshot_path.display()))
        .arg(url)
        .stdout(smol::process::Stdio::piped())
        .stderr(smol::process::Stdio::piped())
        .spawn()?;

    let output = smol::future::or(
        child.output(),
        async {
            smol::Timer::after(CHROME_TIMEOUT).await;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("Chrome timed out after {}s", CHROME_TIMEOUT.as_secs()),
            ))
        },
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Chrome screenshot failed: {}", stderr.trim());
    }

    let image_data = std::fs::read(&screenshot_path)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&image_data))
}
