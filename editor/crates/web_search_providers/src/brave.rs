use anyhow::{Result, anyhow};
use http_client::HttpClientWithUrl;
use scraper::{Html, Selector};
use url::Url;
use web_search::{WebSearchResponse, WebSearchResult};

use crate::normalize_whitespace;

const SEARCH_URL: &str = "https://search.brave.com/search";

pub async fn search(http_client: &HttpClientWithUrl, query: &str) -> Result<WebSearchResponse> {
    let url =
        Url::parse_with_params(SEARCH_URL, &[("q", query)]).map_err(|err| anyhow!("error building url: {err}"))?;
    let html = crate::fetch_html(http_client, url.as_str()).await?;
    parse_results(&html)
}

fn parse_results(html: &str) -> Result<WebSearchResponse> {
    let document = Html::parse_document(html);
    let result_selector = selector(r#"div.snippet[data-type="web"]"#)?;
    let link_selector = selector(r#"a[href^="http"]"#)?;
    let title_selector = selector("div.title.search-snippet-title")?;
    let snippet_content_selector = selector("div.generic-snippet .content")?;
    let snippet_selector = selector("div.generic-snippet")?;

    let mut results = Vec::new();
    for result in document.select(&result_selector) {
        let Some(link) = result.select(&link_selector).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let title = result
            .select(&title_selector)
            .next()
            .and_then(|title| title.value().attr("title"))
            .filter(|title| !title.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                result
                    .select(&title_selector)
                    .next()
                    .map(|title| title.text().collect::<String>())
            })
            .unwrap_or_else(|| link.text().collect::<String>());
        let title = normalize_whitespace(&title);
        if title.is_empty() {
            continue;
        }
        let text = result
            .select(&snippet_content_selector)
            .next()
            .map(|snippet| snippet.text().collect::<String>())
            .or_else(|| {
                result
                    .select(&snippet_selector)
                    .next()
                    .map(|snippet| snippet.text().collect::<String>())
            })
            .map(|text| normalize_whitespace(&text))
            .unwrap_or_default();
        results.push(WebSearchResult {
            title,
            url: href.to_string(),
            text,
        });
    }
    Ok(WebSearchResponse { results })
}

fn selector(raw: &str) -> Result<Selector> {
    Selector::parse(raw).map_err(|err| anyhow!("invalid css selector {raw:?}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_results() {
        let html = r#"
            <div class="snippet svelte-jmfu5f" data-pos="1" data-type="web">
                <div class="title search-snippet-title line-clamp-1 svelte-14r20fy" title="Rust Programming Language">Rust Programming Language</div>
                <a href="https://www.rust-lang.org/" class="svelte-14r20fy l1">Rust Programming Language</a>
                <div class="generic-snippet svelte-1cwdgg3">
                    <div class="content">A language empowering everyone to build reliable and efficient software.</div>
                </div>
            </div>
            <div class="snippet svelte-jmfu5f" data-pos="2" data-type="news">
                <a href="https://news.example.com/story">A news story</a>
                <div class="generic-snippet svelte-1cwdgg3">
                    <div class="content">Not a web result.</div>
                </div>
            </div>
            <div class="snippet svelte-jmfu5f" data-pos="3" data-type="web">
                <a href="https://docs.rs/" class="svelte-14r20fy l1">docs.rs</a>
            </div>
        "#;
        let response = parse_results(html).unwrap();
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].title, "Rust Programming Language");
        assert_eq!(response.results[0].url, "https://www.rust-lang.org/");
        assert_eq!(
            response.results[0].text,
            "A language empowering everyone to build reliable and efficient software."
        );
        assert_eq!(response.results[1].title, "docs.rs");
        assert_eq!(response.results[1].text, "");
    }

    #[test]
    fn test_parse_results_ignores_shell_page() {
        let html = r#"<html><head><title>Brave Search</title></head><body>no results</body></html>"#;
        let response = parse_results(html).unwrap();
        assert!(response.results.is_empty());
    }
}
