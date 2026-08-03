use std::sync::Arc;

use anyhow::{Result, bail};
use gpui::{App, AppContext as _, Task};
use http_client::HttpClientWithUrl;
use web_search::{WebSearchProvider, WebSearchProviderId, WebSearchResponse};

use crate::{brave, duckduckgo};

pub struct FallbackWebSearchProvider {
    http_client: Arc<HttpClientWithUrl>,
}

impl FallbackWebSearchProvider {
    pub fn new(http_client: Arc<HttpClientWithUrl>) -> Self {
        Self { http_client }
    }
}

impl WebSearchProvider for FallbackWebSearchProvider {
    fn id(&self) -> WebSearchProviderId {
        WebSearchProviderId("fallback".into())
    }

    fn search(&self, query: String, cx: &mut App) -> Task<Result<WebSearchResponse>> {
        let http_client = self.http_client.clone();
        cx.background_spawn(async move { run_chain(http_client.as_ref(), &query).await })
    }
}

/// Runs each engine in order, returning the first response with results.
/// An engine is skipped when it errors, returns a non-success status, or
/// yields no results; if every engine fails, the error lists what was tried.
async fn run_chain(http_client: &HttpClientWithUrl, query: &str) -> Result<WebSearchResponse> {
    let mut errors = Vec::new();
    if let Some(response) = try_engine(
        query,
        "DuckDuckGo (html)",
        duckduckgo::search_html(http_client, query),
        &mut errors,
    )
    .await
    {
        return Ok(response);
    }
    if let Some(response) = try_engine(
        query,
        "DuckDuckGo (lite)",
        duckduckgo::search_lite(http_client, query),
        &mut errors,
    )
    .await
    {
        return Ok(response);
    }
    if let Some(response) = try_engine(
        query,
        "Brave",
        brave::search(http_client, query),
        &mut errors,
    )
    .await
    {
        return Ok(response);
    }
    bail!("all web search engines failed: {}", errors.join("; "))
}

async fn try_engine(
    query: &str,
    name: &str,
    engine: impl std::future::Future<Output = Result<WebSearchResponse>>,
    errors: &mut Vec<String>,
) -> Option<WebSearchResponse> {
    match engine.await {
        Ok(response) if !response.results.is_empty() => Some(response),
        Ok(_) => {
            errors.push(format!("{name} returned no results for query {query:?}"));
            None
        }
        Err(err) => {
            errors.push(format!("{name} failed: {err:#}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use http_client::{AsyncBody, Response, FakeHttpClient};

    #[gpui::test]
    async fn test_returns_first_engine_with_results() {
        let request_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let request_count_for_client = request_count.clone();
        let responses = [
            "<html>no results here</html>".to_string(),
            "<html>no results here</html>".to_string(),
            r#"<div class="snippet svelte-jmfu5f" data-pos="1" data-type="web"><a href="https://rust-lang.org/" class="svelte-14r20fy l1">The Rust Programming Language</a><div class="generic-snippet svelte-1cwdgg3"><div class="content">A language empowering everyone.</div></div></div>"#.to_string(),
        ];
        let client = FakeHttpClient::create(move |_request| {
            let index = request_count_for_client.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let body = responses.get(index).cloned().unwrap_or_default();
            async move {
                Ok(Response::builder()
                    .status(200)
                    .body(AsyncBody::from(body.into_bytes()))
                    .unwrap())
            }
        });
        let http_client = Arc::new(http_client::HttpClientWithUrl::new_url(
            client,
            "http://test.example",
            None,
        ));

        let response = run_chain(&http_client, "rust").await.unwrap();
        assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "The Rust Programming Language");
        assert_eq!(response.results[0].url, "https://rust-lang.org/");
    }

    #[gpui::test]
    async fn test_stops_at_first_success() {
        let request_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let request_count_for_client = request_count.clone();
        let client = FakeHttpClient::create(move |_request| {
            let index = request_count_for_client.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let body = if index == 0 {
                r#"<div class="result"><a class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com&amp;rut=1">Example</a></div>"#.to_string()
            } else {
                "<html>unexpected</html>".to_string()
            };
            async move {
                Ok(Response::builder()
                    .status(200)
                    .body(AsyncBody::from(body.into_bytes()))
                    .unwrap())
            }
        });
        let http_client = Arc::new(http_client::HttpClientWithUrl::new_url(
            client,
            "http://test.example",
            None,
        ));

        let response = run_chain(&http_client, "example").await.unwrap();
        assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(response.results[0].title, "Example");
    }

    #[gpui::test]
    async fn test_returns_error_when_all_engines_fail() {
        let client = FakeHttpClient::create(move |_request| async move {
            Err(anyhow!("connection refused"))
        });
        let http_client = Arc::new(http_client::HttpClientWithUrl::new_url(
            client,
            "http://test.example",
            None,
        ));

        let err = run_chain(&http_client, "rust").await.unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("DuckDuckGo (html) failed"));
        assert!(message.contains("DuckDuckGo (lite) failed"));
        assert!(message.contains("Brave failed"));
        assert!(message.contains("connection refused"));
    }
}
