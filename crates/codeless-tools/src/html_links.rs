//! Extract anchor hrefs from HTML, resolved against a base URL.
//!
//! Codeless-native (not ported) because moxxy's `html_text` does
//! readability + text + links in one pass — the crawl tool only
//! needs the link list, so a much smaller module suffices. Uses
//! `scraper` (already in the dep graph via reqwest's siblings).

use scraper::{Html, Selector};
use url::Url;

/// Extract all `<a href="...">` URLs from `html`, resolved against
/// `base_url`. Filters out:
/// - URLs that fail to parse against the base (mailto:, javascript:,
///   relative paths that don't combine, etc.)
/// - Fragment-only links (`#section`) — they don't navigate.
/// - Duplicates: first occurrence wins, order preserved.
pub fn extract_links(html: &str, base_url: &str) -> Vec<String> {
    let base = match Url::parse(base_url) {
        Ok(u) => u,
        Err(_) => return Vec::new(),
    };
    let doc = Html::parse_document(html);
    let selector = Selector::parse("a[href]").expect("a[href] selector parses");

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();

    for el in doc.select(&selector) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        if href.starts_with('#') {
            continue;
        }
        let Ok(resolved) = base.join(href) else {
            continue;
        };
        let mut s = resolved.to_string();
        if let Some(stripped) = s.split_once('#') {
            s = stripped.0.to_string();
        }
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_absolute_and_relative_links() {
        let html = r##"
            <html><body>
                <a href="https://other.example/a">absolute</a>
                <a href="/b">root-relative</a>
                <a href="c">doc-relative</a>
                <a href="#x">fragment-only (skipped)</a>
                <a href="mailto:x@y">mailto (rejected)</a>
            </body></html>
        "##;
        let links = extract_links(html, "https://site.example/path/page");
        assert_eq!(
            links,
            vec![
                "https://other.example/a",
                "https://site.example/b",
                "https://site.example/path/c",
                "mailto:x@y",
            ]
        );
    }

    #[test]
    fn deduplicates_and_strips_fragments() {
        let html = r##"
            <a href="https://e/foo">1</a>
            <a href="https://e/foo">2</a>
            <a href="https://e/foo#section">3</a>
        "##;
        let links = extract_links(html, "https://e/");
        assert_eq!(links, vec!["https://e/foo"]);
    }

    #[test]
    fn empty_html_returns_no_links() {
        assert!(extract_links("", "https://e/").is_empty());
        assert!(extract_links("<p>no links</p>", "https://e/").is_empty());
    }

    #[test]
    fn bad_base_url_returns_empty() {
        assert!(extract_links("<a href='/a'>x</a>", "not a url").is_empty());
    }
}
