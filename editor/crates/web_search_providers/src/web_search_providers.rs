mod brave;
mod duckduckgo;
mod fallback;

use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use futures::AsyncReadExt as _;
use gpui::App;
use http_client::{AsyncBody, HttpClientWithUrl};
use web_search::WebSearchRegistry;

pub fn init(http_client: Arc<HttpClientWithUrl>, cx: &mut App) {
    let registry = WebSearchRegistry::global(cx);
    registry.update(cx, |registry, cx| {
        registry.register_provider(fallback::FallbackWebSearchProvider::new(http_client), cx);
    });
}

const MAX_HTML_BODY_BYTES: usize = 5 * 1024 * 1024;

/// Fetches the HTML at `url`, enforcing a size cap before parsing.
async fn fetch_html(http_client: &HttpClientWithUrl, url: &str) -> Result<String> {
    let mut response = http_client
        .get(url, AsyncBody::default(), true)
        .await
        .with_context(|| format!("error fetching {url}"))?;
    if !response.status().is_success() {
        anyhow::bail!(
            "error fetching {url}: unexpected status {}",
            response.status()
        );
    }

    let mut body = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let bytes_read = response
            .body_mut()
            .read(&mut buffer)
            .await
            .context("error reading response body")?;
        if bytes_read == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..bytes_read]);
        if body.len() > MAX_HTML_BODY_BYTES {
            bail!(
                "response body from {url} exceeded {} bytes",
                MAX_HTML_BODY_BYTES
            );
        }
    }

    String::from_utf8(body).map_err(|_| anyhow!("response body from {url} was not valid UTF-8"))
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
