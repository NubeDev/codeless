// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/html_text.rs
//
// HTML to clean readable text (markdown-flavoured) + link extraction.
// One pass per call where possible — `extract_text_and_links` shares
// the parse across both outputs so the crawl + read tools don't pay
// twice.
//
// Differences from upstream: link extraction uses LinkInfo { url,
// text } which carries the anchor text alongside the URL. The
// earlier codeless-only html_links module returned bare URLs; this
// module supersedes it.

use scraper::{Html, Node, Selector};
use url::Url;

/// Extracted link with resolved URL and anchor text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    pub url: String,
    pub text: String,
}

/// Tags whose entire subtree is dropped — noise / chrome / non-content.
const SKIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "svg", "iframe", "nav", "header", "footer", "aside",
];

/// Convert HTML to clean readable text with markdown structure.
///
/// Prefers `<main>` or `<article>` as the content root when present;
/// falls back to `<body>` then the whole document. Drops chrome
/// elements (nav/header/footer/aside) and JS/CSS subtrees.
pub fn html_to_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut buf = String::with_capacity(html.len() / 4);
    walk_root(&document, &mut buf);
    collapse_whitespace(&buf)
}

/// Resolve every `<a href>` against `base_url`. Skips fragment-only,
/// javascript:, mailto:, tel:, and data: URIs. Deduplicates on the
/// resolved URL; first occurrence wins (its anchor text is the one
/// returned).
pub fn extract_links(html: &str, base_url: &str) -> Vec<LinkInfo> {
    let document = Html::parse_document(html);
    let base = Url::parse(base_url).ok();
    extract_links_from_doc(&document, base.as_ref())
}

/// One parse, two outputs. Use this from any caller that needs both
/// (the browser.read tool with mode=markdown, and the crawl loop).
pub fn extract_text_and_links(html: &str, base_url: &str) -> (String, Vec<LinkInfo>) {
    let document = Html::parse_document(html);
    let base = Url::parse(base_url).ok();

    let mut buf = String::with_capacity(html.len() / 4);
    walk_root(&document, &mut buf);
    let text = collapse_whitespace(&buf);

    let links = extract_links_from_doc(&document, base.as_ref());
    (text, links)
}

// Internals ------------------------------------------------------------------

/// Locate the focused content root and walk it. `<main>` and
/// `<article>` win over `<body>` because chrome-heavy pages bury
/// real content under nav + ads.
fn walk_root(document: &Html, buf: &mut String) {
    let root_id = ["main", "article"]
        .iter()
        .filter_map(|tag| Selector::parse(tag).ok())
        .find_map(|sel| document.select(&sel).next().map(|el| el.id()));

    let root_node = match root_id {
        Some(id) => document.tree.get(id).unwrap(),
        None => Selector::parse("body")
            .ok()
            .and_then(|sel| document.select(&sel).next())
            .map(|el| document.tree.get(el.id()).unwrap())
            .unwrap_or(document.tree.root()),
    };

    walk_node(root_node, buf, false);
}

fn walk_node(node: ego_tree::NodeRef<'_, Node>, buf: &mut String, in_pre: bool) {
    for child in node.children() {
        match child.value() {
            Node::Text(text) => {
                let s: &str = text;
                if in_pre {
                    buf.push_str(s);
                } else {
                    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !collapsed.is_empty() {
                        buf.push_str(&collapsed);
                    }
                }
            }
            Node::Element(el) => {
                let tag = el.name();
                if SKIP_TAGS.contains(&tag) {
                    continue;
                }
                match tag {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let level = tag[1..].parse::<usize>().unwrap_or(1);
                        buf.push_str("\n\n");
                        for _ in 0..level {
                            buf.push('#');
                        }
                        buf.push(' ');
                        walk_node(child, buf, false);
                        buf.push_str("\n\n");
                    }
                    "p" | "div" | "section" | "article" | "main" => {
                        buf.push_str("\n\n");
                        walk_node(child, buf, false);
                        buf.push_str("\n\n");
                    }
                    "br" => buf.push('\n'),
                    "li" => {
                        buf.push_str("\n- ");
                        walk_node(child, buf, false);
                    }
                    "blockquote" => {
                        buf.push_str("\n\n> ");
                        walk_node(child, buf, false);
                        buf.push_str("\n\n");
                    }
                    "pre" => {
                        buf.push_str("\n\n```\n");
                        walk_node(child, buf, true);
                        buf.push_str("\n```\n\n");
                    }
                    "code" => {
                        if in_pre {
                            walk_node(child, buf, true);
                        } else {
                            buf.push('`');
                            walk_node(child, buf, false);
                            buf.push('`');
                        }
                    }
                    "img" => {
                        if let Some(alt) = el.attr("alt").filter(|a| !a.is_empty()) {
                            buf.push_str(&format!("[image: {alt}]"));
                        }
                    }
                    "a" => walk_node(child, buf, in_pre),
                    "table" => {
                        buf.push_str("\n\n");
                        walk_node(child, buf, false);
                        buf.push_str("\n\n");
                    }
                    "tr" => {
                        buf.push('\n');
                        walk_node(child, buf, false);
                    }
                    "td" | "th" => {
                        buf.push_str(" | ");
                        walk_node(child, buf, false);
                    }
                    _ => walk_node(child, buf, in_pre),
                }
            }
            _ => {} // comments, processing instructions
        }
    }
}

fn extract_links_from_doc(document: &Html, base: Option<&Url>) -> Vec<LinkInfo> {
    let Ok(a_selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };

    let mut seen = std::collections::HashSet::new();
    let mut links = Vec::new();

    for el in document.select(&a_selector) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        let href = href.trim();
        if href.is_empty()
            || href.starts_with('#')
            || href.starts_with("javascript:")
            || href.starts_with("mailto:")
            || href.starts_with("tel:")
            || href.starts_with("data:")
        {
            continue;
        }

        let resolved = if let Ok(abs) = Url::parse(href) {
            abs.to_string()
        } else if let Some(base) = base {
            match base.join(href) {
                Ok(abs) => abs.to_string(),
                Err(_) => continue,
            }
        } else {
            continue;
        };

        if !seen.insert(resolved.clone()) {
            continue;
        }

        let text: String = el
            .text()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        links.push(LinkInfo {
            url: resolved,
            text,
        });
    }

    links
}

fn collapse_whitespace(raw: &str) -> String {
    // Cap consecutive newlines at 2 so paragraph breaks stay
    // visible without runaway blank-line stretches.
    let mut result = String::with_capacity(raw.len());
    let mut newline_count = 0u32;
    for ch in raw.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push('\n');
            }
        } else {
            newline_count = 0;
            result.push(ch);
        }
    }
    result
        .lines()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_script_and_style_tags() {
        let html = "<html><head><style>body{color:red}</style></head>\
            <body><script>alert('hi')</script><p>Hello world</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello world"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("color:red"));
    }

    #[test]
    fn strips_nav_header_footer_aside() {
        let html = "<html><body>\
            <nav>Skip nav</nav>\
            <header>Skip header</header>\
            <p>Content here</p>\
            <footer>Skip footer</footer>\
            <aside>Skip aside</aside>\
        </body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Content here"));
        assert!(!text.contains("Skip nav"));
        assert!(!text.contains("Skip header"));
        assert!(!text.contains("Skip footer"));
        assert!(!text.contains("Skip aside"));
    }

    #[test]
    fn converts_headings() {
        let html = "<html><body><h1>Title</h1><h2>Sub</h2><h3>Deep</h3></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("# Title"));
        assert!(text.contains("## Sub"));
        assert!(text.contains("### Deep"));
    }

    #[test]
    fn converts_lists() {
        let html = "<html><body><ul><li>Alpha</li><li>Beta</li><li>Gamma</li></ul></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("- Alpha"));
        assert!(text.contains("- Beta"));
        assert!(text.contains("- Gamma"));
    }

    #[test]
    fn prefers_article_over_chrome() {
        let html = "<html><body>\
            <nav>Noise</nav>\
            <article><p>Main content</p></article>\
            <footer>More noise</footer>\
        </body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Main content"));
        assert!(!text.contains("Noise"));
        assert!(!text.contains("More noise"));
    }

    #[test]
    fn prefers_main_over_chrome() {
        let html = "<html><body>\
            <header>Nav stuff</header>\
            <main><h1>Hello</h1><p>World</p></main>\
            <footer>Footer</footer>\
        </body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("Nav stuff"));
        assert!(!text.contains("Footer"));
    }

    #[test]
    fn handles_code_blocks() {
        let html =
            "<html><body><pre><code>fn main() {\n    println!(\"hello\");\n}</code></pre></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("```"));
        assert!(text.contains("fn main()"));
    }

    #[test]
    fn handles_inline_code() {
        let html = "<html><body><p>Use <code>cargo build</code> to compile.</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("`cargo build`"));
    }

    #[test]
    fn handles_images_with_alt() {
        let html = "<html><body><img alt=\"A cat\" src=\"cat.jpg\"></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("[image: A cat]"));
    }

    #[test]
    fn handles_blockquote() {
        let html = "<html><body><blockquote>Important note</blockquote></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("> Important note"));
    }

    #[test]
    fn handles_empty_html() {
        assert_eq!(html_to_text(""), "");
        assert_eq!(html_to_text("<html></html>"), "");
    }

    #[test]
    fn collapses_excessive_whitespace() {
        let html = "<html><body><p>A</p><p>B</p><p>C</p><p>D</p></body></html>";
        let text = html_to_text(html);
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn extracts_absolute_links() {
        let html = "<html><body><a href=\"https://example.com/page\">Example</a></body></html>";
        let links = extract_links(html, "https://base.com");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://example.com/page");
        assert_eq!(links[0].text, "Example");
    }

    #[test]
    fn resolves_relative_links() {
        let html =
            "<html><body><a href=\"/about\">About</a><a href=\"contact\">Contact</a></body></html>";
        let links = extract_links(html, "https://example.com/page");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].url, "https://example.com/about");
        assert_eq!(links[1].url, "https://example.com/contact");
    }

    #[test]
    fn skips_javascript_mailto_tel_fragment() {
        let html = "<html><body>\
            <a href=\"javascript:void(0)\">JS</a>\
            <a href=\"mailto:test@test.com\">Email</a>\
            <a href=\"tel:+1234567890\">Phone</a>\
            <a href=\"#section\">Fragment</a>\
            <a href=\"data:image/png;base64,xyz\">Data</a>\
            <a href=\"https://real.com\">Real</a>\
        </body></html>";
        let links = extract_links(html, "https://base.com");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://real.com/");
    }

    #[test]
    fn deduplicates_links_keeping_first_text() {
        let html = "<html><body>\
            <a href=\"https://example.com\">First</a>\
            <a href=\"https://example.com\">Second</a>\
        </body></html>";
        let links = extract_links(html, "https://base.com");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].text, "First");
    }

    #[test]
    fn combined_extraction_one_parse() {
        let html = "<html><body>\
            <h1>Title</h1>\
            <p>Some text with a <a href=\"https://link.com\">link</a>.</p>\
        </body></html>";
        let (text, links) = extract_text_and_links(html, "https://base.com");
        assert!(text.contains("# Title"));
        assert!(text.contains("Some text"));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://link.com/");
    }

    #[test]
    fn handles_table_content() {
        let html = "<html><body><table>\
            <tr><th>Name</th><th>Age</th></tr>\
            <tr><td>Alice</td><td>30</td></tr>\
        </table></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Name"));
        assert!(text.contains("Alice"));
        assert!(text.contains("30"));
    }
}
