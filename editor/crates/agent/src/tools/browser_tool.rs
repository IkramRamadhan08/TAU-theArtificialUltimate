use std::sync::Arc;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema as acp;
use anyhow::Result;
use gpui::{App, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::SharedString;

fn find_chrome() -> Option<String> {
    // Check common Chrome/Chromium paths per platform
    let candidates: Vec<String> = if cfg!(target_os = "macos") {
        vec![
            // Google Chrome
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into(),
            // Chromium
            "/Applications/Chromium.app/Contents/MacOS/Chromium".into(),
            // Chrome Canary
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary".into(),
            // Brave
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".into(),
            // Edge
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into(),
        ]
    } else if cfg!(target_os = "windows") {
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let program_files = std::env::var("PROGRAMFILES").unwrap_or_default();
        let program_files_x86 = std::env::var("PROGRAMFILES(X86)").unwrap_or_default();
        vec![
            // Google Chrome
            format!("{}\\Google\\Chrome\\Application\\chrome.exe", local_app_data),
            format!("{}\\Google\\Chrome\\Application\\chrome.exe", program_files),
            format!("{}\\Google\\Chrome\\Application\\chrome.exe", program_files_x86),
            // Edge
            format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", program_files),
            format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", program_files_x86),
            // Brave
            format!("{}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", local_app_data),
        ]
    } else {
        // Linux / FreeBSD
        vec![
            // Google Chrome
            "/usr/bin/google-chrome".into(),
            "/usr/bin/google-chrome-stable".into(),
            "/usr/bin/google-chrome-beta".into(),
            "/usr/bin/google-chrome-dev".into(),
            // Chromium
            "/usr/bin/chromium".into(),
            "/usr/bin/chromium-browser".into(),
            // Snap
            "/snap/bin/chromium".into(),
            // Flatpak
            "/usr/bin/org.chromium.Chromium".into(),
            // Brave
            "/usr/bin/brave-browser".into(),
            "/usr/bin/brave-browser-stable".into(),
            // Edge
            "/usr/bin/microsoft-edge".into(),
            "/usr/bin/microsoft-edge-stable".into(),
            // Vendor-bundled Chrome (TAU agent-browser)
            "/home/eightarch/.agent-browser/browsers/chrome-149.0.7827.115/chrome".into(),
        ]
    };

    // Find first existing path
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.clone());
        }
    }

    // Fallback: try `which` / `where` to find chrome or chromium in PATH
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

/// Navigate to a URL and return the rendered page content as text.
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

            let url = if !input.url.starts_with("http://") && !input.url.starts_with("https://") {
                format!("https://{}", input.url)
            } else {
                input.url.clone()
            };

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
    let output = std::process::Command::new(chrome)
        .arg("--headless")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--disable-dev-shm-usage")
        .arg("--dump-dom")
        .arg(url)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Chrome exited with error: {}", stderr.trim());
    }

    let content = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if content.is_empty() {
        anyhow::bail!("Chrome returned empty page content");
    }

    let title = extract_title(&content);

    Ok((title, content))
}

fn extract_title(html: &str) -> String {
    if let Some(start) = html.find("<title>") {
        let after = &html[start + 7..];
        if let Some(end) = after.find("</title>") {
            return after[..end].to_string();
        }
    }
    String::new()
}

/// Take a screenshot of a URL and return it as base64-encoded PNG.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BrowserScreenshotToolInput {
    /// The URL to take a screenshot of
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserScreenshotOutput {
    Success {
        /// base64-encoded PNG image data
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
                format!("data:image/png;base64,{}", screenshot_base64).into()
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

            let url = if !input.url.starts_with("http://") && !input.url.starts_with("https://") {
                format!("https://{}", input.url)
            } else {
                input.url.clone()
            };

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

    let output = std::process::Command::new(chrome)
        .arg("--headless")
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--disable-dev-shm-usage")
        .arg("--screenshot")
        .arg(format!("--screenshot={}", screenshot_path.display()))
        .arg(url)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Chrome screenshot failed: {}", stderr.trim());
    }

    let image_data = std::fs::read(&screenshot_path)?;

    Ok(encode_base64(&image_data))
}

fn encode_base64(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let combined = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((combined >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((combined >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((combined >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(combined & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
