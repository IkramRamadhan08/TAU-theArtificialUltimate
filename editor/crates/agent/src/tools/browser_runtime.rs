use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::AsyncReadExt;
use http_client::HttpClientWithUrl;
use serde::Deserialize;
use util::archive::extract_zip;
use util::fs::make_file_executable;

fn find_system_chrome() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(target_os = "windows") {
        &["chrome.exe", "google-chrome.exe"]
    } else if cfg!(target_os = "macos") {
        &["google-chrome", "google-chrome-stable", "chromium"]
    } else {
        &["google-chrome", "google-chrome-stable", "chromium-browser", "chromium"]
    };

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in names {
                let full = dir.join(name);
                if full.is_file() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(m) = std::fs::metadata(&full) {
                            if m.permissions().mode() & 0o111 != 0 {
                                return Some(full);
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    return Some(full);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        for p in &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ] {
            if Path::new(p).exists() {
                return Some(PathBuf::from(p));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for var in &["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(var) {
                for p in &[
                    format!("{base}\\Google\\Chrome\\Application\\chrome.exe"),
                    format!("{base}\\Chromium\\Application\\chrome.exe"),
                ] {
                    if Path::new(p).exists() {
                        return Some(PathBuf::from(p));
                    }
                }
            }
        }
    }

    None
}

const CFT_VERSION_LIST_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/known-good-versions-with-downloads.json";

const FALLBACK_VERSION: &str = "131.0.6778.85";

pub struct BrowserRuntime {
    pub binary_path: PathBuf,
    #[allow(dead_code)]
    pub version: String,
}

#[derive(Deserialize)]
struct KnownGoodVersionsResponse {
    versions: Vec<KnownGoodVersionEntry>,
}

#[derive(Deserialize)]
struct KnownGoodVersionEntry {
    version: String,
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    #[serde(rename = "chrome-headless-shell")]
    chrome_headless_shell: Vec<DownloadEntry>,
}

#[derive(Deserialize)]
struct DownloadEntry {
    platform: String,
    #[allow(dead_code)]
    url: String,
}

fn platform_string() -> Result<&'static str> {
    match std::env::consts::OS {
        "linux" => Ok("linux64"),
        "macos" => {
            if std::env::consts::ARCH == "aarch64" {
                Ok("mac-arm64")
            } else {
                Ok("mac-x64")
            }
        }
        "windows" => {
            if std::env::consts::ARCH == "x86_64" {
                Ok("win64")
            } else {
                Ok("win32")
            }
        }
        other => anyhow::bail!("Unsupported OS: {other}"),
    }
}

fn cft_download_urls(version: &str, platform: &str) -> Vec<String> {
    vec![
        format!("https://storage.googleapis.com/chrome-for-testing-public/{version}/{platform}/chrome-headless-shell-{platform}.zip"),
        format!("https://playwright.azureedge.net/chrome-for-testing-public/{version}/{platform}/chrome-headless-shell-{platform}.zip"),
    ]
}

fn install_dir() -> PathBuf {
    paths::data_dir().join("browser")
}

fn binary_path_for_version(version: &str) -> Result<PathBuf> {
    let platform = platform_string()?;
    let dir_name = format!("chrome-headless-shell-{platform}");
    Ok(install_dir().join(version).join(dir_name).join(
        if cfg!(target_os = "windows") {
            "chrome-headless-shell.exe"
        } else {
            "chrome-headless-shell"
        },
    ))
}

async fn validate_binary(binary_path: &Path) -> bool {
    let result = util::command::new_command(binary_path)
        .arg("--version")
        .output()
        .await;
    match result {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

async fn fetch_latest_version(http: &Arc<HttpClientWithUrl>) -> Result<String> {
    let mut response = http
        .get(CFT_VERSION_LIST_URL, Default::default(), true)
        .await
        .context("Failed to fetch Chrome for Testing version list")?;

    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .context("Failed to read version list response")?;

    let data: KnownGoodVersionsResponse =
        serde_json::from_slice(&body).context("Failed to parse version list JSON")?;

    let platform = platform_string()?;

    let version = data
        .versions
        .iter()
        .rev()
        .find(|entry| {
            entry
                .downloads
                .chrome_headless_shell
                .iter()
                .any(|d| d.platform == platform)
        })
        .map(|entry| entry.version.clone())
        .context("No chrome-headless-shell version found for this platform")?;

    Ok(version)
}

pub async fn ensure_browser_installed(http: &Arc<HttpClientWithUrl>) -> Result<BrowserRuntime> {
    let platform = platform_string()?;

    if let Some(system_chrome) = find_system_chrome() {
        log::info!("Using system Chrome at {}", system_chrome.display());
        return Ok(BrowserRuntime {
            binary_path: system_chrome,
            version: "system".into(),
        });
    }

    let version = match fetch_latest_version(http).await {
        Ok(v) => {
            log::info!("Latest Chrome for Testing version: {v}");
            v
        }
        Err(e) => {
            log::warn!(
                "Failed to fetch latest Chrome for Testing version, using fallback {FALLBACK_VERSION}: {e}"
            );
            FALLBACK_VERSION.to_string()
        }
    };

    let binary = binary_path_for_version(&version)?;

    if binary.exists() && validate_binary(&binary).await {
        log::info!("Using cached Chrome for Testing at {}", binary.display());
        return Ok(BrowserRuntime {
            binary_path: binary,
            version,
        });
    }

    log::info!(
        "Downloading Chrome for Testing {version} for {platform}..."
    );
    download_and_extract(http, &version, platform).await?;

    let binary = binary_path_for_version(&version)?;
    if !validate_binary(&binary).await {
        anyhow::bail!(
            "Downloaded Chrome for Testing binary at {} is not functional",
            binary.display()
        );
    }

    log::info!(
        "Chrome for Testing {version} installed at {}",
        binary.display()
    );
    Ok(BrowserRuntime {
        binary_path: binary,
        version,
    })
}

async fn download_and_extract(
    http: &Arc<HttpClientWithUrl>,
    version: &str,
    platform: &str,
) -> Result<()> {
    let urls = cft_download_urls(version, platform);
    let mut last_error = String::new();
    let mut response_body = None;

    for url in &urls {
        log::info!("Downloading from {url}");
        match http.get(url, Default::default(), true).await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    response_body = Some(resp.into_body());
                    break;
                }
                last_error = format!("HTTP {status}");
                log::warn!("Download from {url} failed: {last_error}");
            }
            Err(e) => {
                last_error = e.to_string();
                log::warn!("Download from {url} failed: {last_error}");
            }
        }
    }

    let mut body = response_body.context(format!(
        "Failed to download Chrome for Testing from all sources. Last error: {last_error}. \
         Try installing Chrome/Chromium on your system so the browser tool can use it directly."
    ))?;

    let dest = install_dir().join(version);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .context("Failed to remove old installation")?;
    }
    std::fs::create_dir_all(&dest)
        .context("Failed to create installation directory")?;

    extract_zip(&dest, &mut body)
        .await
        .context("Failed to extract Chrome for Testing archive")?;

    if cfg!(target_os = "macos") {
        let shell_dir = dest.join(format!("chrome-headless-shell-{platform}"));
        let _ = std::process::Command::new("xattr")
            .args(["-cr", &shell_dir.to_string_lossy()])
            .output();
    }

    let binary = binary_path_for_version(version)?;
    make_file_executable(&binary)
        .await
        .context("Failed to make Chrome for Testing executable")?;

    Ok(())
}
