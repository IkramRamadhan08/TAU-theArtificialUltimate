use anyhow::{Result, anyhow};
use http_client::HttpClientWithUrl;
use scraper::{Html, Selector};
use url::Url;
use web_search::{WebSearchResponse, WebSearchResult};

use crate::normalize_whitespace;

const HTML_SEARCH_URL: &str = "https://html.duckduckgo.com/html/";
const LITE_SEARCH_URL: &str = "https://lite.duckduckgo.com/lite/";

pub async fn search_html(
    http_client: &HttpClientWithUrl,
    query: &str,
) -> Result<WebSearchResponse> {
    let url = build_search_url(HTML_SEARCH_URL, query)?;
    let html = crate::fetch_html(http_client, url.as_str()).await?;
    parse_html_results(&html)
}

pub async fn search_lite(
    http_client: &HttpClientWithUrl,
    query: &str,
) -> Result<WebSearchResponse> {
    let url = build_search_url(LITE_SEARCH_URL, query)?;
    let html = crate::fetch_html(http_client, url.as_str()).await?;
    parse_lite_results(&html)
}

fn build_search_url(base: &str, query: &str) -> Result<Url> {
    Url::parse_with_params(base, &[("q", query)]).map_err(|err| anyhow!("error building url: {err}"))
}

fn parse_html_results(html: &str) -> Result<WebSearchResponse> {
    let document = Html::parse_document(html);
    let result_selector = selector("div.result")?;
    let link_selector = selector("a.result__a")?;
    let snippet_selector = selector("a.result__snippet, .result__snippet")?;

    let mut results = Vec::new();
    for result in document.select(&result_selector) {
        let Some(link) = result.select(&link_selector).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let title = normalize_whitespace(&link.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let url = decode_ddg_url(href)?;
        let text = result
            .select(&snippet_selector)
            .next()
            .map(|snippet| normalize_whitespace(&snippet.text().collect::<String>()))
            .unwrap_or_default();
        results.push(WebSearchResult { title, url, text });
    }
    Ok(WebSearchResponse { results })
}

fn parse_lite_results(html: &str) -> Result<WebSearchResponse> {
    let document = Html::parse_document(html);
    let result_selector = selector("div.result")?;
    let link_selector = selector("a.result-link")?;
    let snippet_selector = selector("td.result-snippet")?;

    let mut results = Vec::new();
    for result in document.select(&result_selector) {
        let Some(link) = result.select(&link_selector).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let title = normalize_whitespace(&link.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let url = decode_ddg_url(href)?;
        let text = result
            .select(&snippet_selector)
            .next()
            .map(|snippet| normalize_whitespace(&snippet.text().collect::<String>()))
            .unwrap_or_default();
        results.push(WebSearchResult { title, url, text });
    }
    Ok(WebSearchResponse { results })
}

/// DuckDuckGo encodes result URLs in the `uddg` query parameter of a relative
/// `/l/` redirect. Falls back to the raw href when no `uddg` parameter exists.
fn decode_ddg_url(href: &str) -> Result<String> {
    let url = Url::parse(href)
        .or_else(|_| Url::parse(&format!("https://duckduckgo.com{href}")))?;
    if let Some((_, target)) = url.query_pairs().find(|(key, _)| key == "uddg") {
        return Ok(target.into_owned());
    }
    Ok(url.to_string())
}

fn selector(raw: &str) -> Result<Selector> {
    Selector::parse(raw).map_err(|err| anyhow!("invalid css selector {raw:?}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_ddg_url_returns_uddg_target() {
        let href = "/l/?uddg=https%3A%2F%2Fexample.com%2Fpath%3Fa%3D1&rut=abc";
        assert_eq!(decode_ddg_url(href).unwrap(), "https://example.com/path?a=1");
    }

    #[test]
    fn test_decode_ddg_url_falls_back_to_raw_href() {
        assert_eq!(
            decode_ddg_url("https://example.com/plain").unwrap(),
            "https://example.com/plain"
        );
    }

    #[test]
    fn test_parse_html_results() {
        let html = r#"
            <div class="result">
                <a class="result__a" href="/l/?uddg=https%3A%2F%2Frust-lang.org&amp;rut=1">The Rust Programming Language</a>
                <a class="result__snippet" href="/l/?uddg=https%3A%2F%2Frust-lang.org&amp;rut=2">A language empowering everyone to build reliable software.</a>
            </div>
            <div class="result">
                <a class="result__a" href="/l/?uddg=https%3A%2F%2Fdocs.rs&amp;rut=3">docs.rs</a>
            </div>
        "#;
        let response = parse_html_results(html).unwrap();
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].title, "The Rust Programming Language");
        assert_eq!(response.results[0].url, "https://rust-lang.org");
        assert_eq!(
            response.results[0].text,
            "A language empowering everyone to build reliable software."
        );
        assert_eq!(response.results[1].title, "docs.rs");
        assert_eq!(response.results[1].text, "");
    }

    #[test]
    fn test_parse_lite_results() {
        let html = r#"
            <div class='result'>
                <table>
                    <tr class='result-link'>
                        <td><a rel="nofollow" class='result-link' href='//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&amp;rut=1'>Example Domain</a></td>
                    </tr>
                    <tr class='result-snippet'>
                        <td class='result-snippet'>This domain is for use in illustrative examples.</td>
                    </tr>
                </table>
            </div>
        "#;
        let response = parse_lite_results(html).unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Example Domain");
        assert_eq!(response.results[0].url, "https://example.com");
        assert_eq!(
            response.results[0].text,
            "This domain is for use in illustrative examples."
        );
    }
}
