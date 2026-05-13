// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/crawl.rs
//
// BFS crawler that drives the Playwright sidecar via the manager.
// Opens an ephemeral session, reuses a single tab across
// navigations, walks discovered links breadth-first respecting
// depth/page caps, closes the session at the end best-effort.
//
// Differences from upstream:
// - NetworkMode + allowlist gating uses ctx (codeless-side helpers)
//   rather than the manager's bundled allowlist file.
// - Link extraction uses html_text::extract_links. The crawl tool
//   only consumes the URL field; anchor text is dropped before the
//   per-page result is built.
// - Per-page result is { url, final_url, status, depth, title,
//   links_found } — no text body. Callers that want the body can
//   call codeless.browser.read per URL.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::html_text;
use crate::tool::Tool;

const HARD_MAX_DEPTH: u64 = 10;
const HARD_MAX_PAGES: u64 = 200;
const PER_PAGE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct BrowserCrawlTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserCrawlTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Starting URL." },
                    "max_depth": {
                        "type": "integer",
                        "description": "Default 1, hard cap 10."
                    },
                    "max_pages": {
                        "type": "integer",
                        "description": "Default 5, hard cap 200."
                    },
                    "same_domain": {
                        "type": "boolean",
                        "description": "Default true. Only follow links to the start host."
                    },
                    "wait_until": {
                        "type": "string",
                        "enum": ["load", "domcontentloaded", "networkidle"]
                    }
                },
                "required": ["url"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserCrawlTool {
    fn name(&self) -> &str {
        "codeless.browser.crawl"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let start_url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'url'"))?
            .to_string();
        let start_host = super::url_host(&start_url)
            .ok_or_else(|| ToolError::invalid_args("start URL has no host"))?;
        super::check_network_policy(ctx, &start_host, &start_url)?;

        let max_depth = args
            .get("max_depth")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .min(HARD_MAX_DEPTH) as u32;
        let max_pages = args
            .get("max_pages")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .min(HARD_MAX_PAGES) as u32;
        let same_domain = args
            .get("same_domain")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let wait_until = args
            .get("wait_until")
            .and_then(Value::as_str)
            .unwrap_or("load")
            .to_string();

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let session = self
            .manager
            .request("session.create", json!({}), None)
            .await?;
        let session_id = session
            .get("session_id")
            .or_else(|| session.get("sessionId"))
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::failed("session.create returned no session_id"))?
            .to_string();

        let result = self
            .crawl_inner(
                ctx,
                &session_id,
                &start_url,
                &start_host,
                max_depth,
                max_pages,
                same_domain,
                &wait_until,
            )
            .await;

        // Best-effort close. Don't propagate this error — the actual
        // crawl result is what the caller wants.
        let _ = self
            .manager
            .request("session.close", json!({ "session_id": session_id }), None)
            .await;

        result
    }
}

impl BrowserCrawlTool {
    #[allow(clippy::too_many_arguments)]
    async fn crawl_inner(
        &self,
        ctx: &ToolCtx,
        session_id: &str,
        start_url: &str,
        start_host: &str,
        max_depth: u32,
        max_pages: u32,
        same_domain: bool,
        wait_until: &str,
    ) -> Result<Value, ToolError> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        let mut pages_out: Vec<Value> = Vec::new();
        let mut page_id: Option<String> = None;

        queue.push_back((start_url.to_string(), 0));

        while let Some((url, depth)) = queue.pop_front() {
            if ctx.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            if pages_out.len() >= max_pages as usize {
                break;
            }
            let normalised = normalise_url(&url);
            if !visited.insert(normalised) {
                continue;
            }
            let Some(host) = super::url_host(&url) else {
                continue;
            };
            if same_domain && host != start_host {
                continue;
            }
            // Skip hosts that fail the policy gate — keep crawling
            // rather than fail the whole call, mirrors moxxy.
            if super::check_network_policy(ctx, &host, &url).is_err() {
                continue;
            }

            let mut goto_params = json!({
                "session_id": session_id,
                "url": &url,
                "wait_until": wait_until,
            });
            if let Some(pid) = &page_id {
                goto_params["page_id"] = json!(pid);
            }
            let Ok(goto) = self
                .manager
                .request("page.goto", goto_params, Some(PER_PAGE_TIMEOUT))
                .await
            else {
                tracing::warn!(url = %url, "crawl navigate failed, skipping");
                continue;
            };
            if page_id.is_none() {
                page_id = goto
                    .get("page_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            let status = goto.get("status").and_then(Value::as_u64);

            let read_params = match &page_id {
                Some(pid) => json!({ "page_id": pid }),
                None => continue,
            };
            let Ok(read) = self
                .manager
                .request("page.read", read_params, Some(PER_PAGE_TIMEOUT))
                .await
            else {
                tracing::warn!(url = %url, "crawl read failed, skipping");
                continue;
            };

            let html = read
                .get("html")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let title = read
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let final_url = read
                .get("final_url")
                .and_then(Value::as_str)
                .unwrap_or(&url)
                .to_string();

            let links = html_text::extract_links(&html, &final_url);

            pages_out.push(json!({
                "url": url,
                "final_url": final_url,
                "status": status,
                "depth": depth,
                "title": title,
                "links_found": links.len(),
            }));

            if depth < max_depth {
                for link in &links {
                    let n = normalise_url(&link.url);
                    if !visited.contains(&n) {
                        queue.push_back((link.url.clone(), depth + 1));
                    }
                }
            }
        }

        Ok(json!({
            "pages_crawled": pages_out.len(),
            "pages": pages_out,
        }))
    }
}

/// Normalise URLs for visited-set comparison. Strips fragments and
/// any trailing slash. Falls back to the raw string when the URL
/// can't be parsed — same behaviour as moxxy.
fn normalise_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut u) => {
            u.set_fragment(None);
            let mut s = u.to_string();
            if s.ends_with('/') && s.len() > 1 {
                s.pop();
            }
            s
        }
        Err(_) => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_strips_fragment_and_trailing_slash() {
        assert_eq!(normalise_url("https://e/a/#frag"), "https://e/a");
        assert_eq!(normalise_url("https://e/a"), "https://e/a");
        // The single-slash guard protects the literal "/" string, not
        // a URL whose path is "/" — matches moxxy's behaviour. Both
        // forms map to the same canonical bucket for visited-set
        // comparison; that's the load-bearing property.
        assert_eq!(normalise_url("https://e/"), "https://e");
    }

    #[test]
    fn normalise_falls_back_on_unparseable() {
        assert_eq!(normalise_url("not a url"), "not a url");
    }
}
