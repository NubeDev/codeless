// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browse.rs
//
// Scope: just the `fetch` primitive. The HTML extractor, CSS selector
// path, and the JS-rendering browser are all later ports. What this
// file does: GET a URL with browser-like headers, return status +
// body. NetworkMode + AllowlistFile gating is enforced before egress.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::select;

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::policy::NetworkMode;
use crate::tool::Tool;

/// Maximum response body kept in memory. Chosen to roughly match
/// moxxy's default; oversized responses fail rather than truncate so
/// the runner sees a structured error.
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Per-request timeout. The runner can cancel earlier via `ToolCtx`.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct BrowseFetchTool {
    schema: Value,
    timeout: Duration,
    max_response_bytes: usize,
}

impl BrowseFetchTool {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_TIMEOUT, DEFAULT_MAX_RESPONSE_BYTES)
    }

    pub fn with_limits(timeout: Duration, max_response_bytes: usize) -> Self {
        Self {
            schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Absolute http(s) URL to fetch."
                    }
                },
                "required": ["url"]
            }),
            timeout,
            max_response_bytes,
        }
    }
}

impl Default for BrowseFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowseFetchTool {
    fn name(&self) -> &str {
        "codeless.browse.fetch"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'url'"))?;

        let host = url_host(url).ok_or_else(|| ToolError::invalid_args("URL has no host"))?;

        match ctx.network_mode() {
            NetworkMode::None => {
                return Err(ToolError::denied(format!(
                    "network disabled; cannot fetch '{}'",
                    url
                )));
            }
            NetworkMode::Allowlist => {
                if !ctx.allowlist().allows(&host) {
                    return Err(ToolError::denied(format!(
                        "host '{}' not in allowlist",
                        host
                    )));
                }
            }
            NetworkMode::Open => {}
        }

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let client = build_browser_client(self.timeout)
            .map_err(|e| ToolError::failed(format!("client build failed: {}", e)))?;

        let resp = select! {
            biased;
            _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
            r = client.get(url).send() => r.map_err(|e| {
                if e.is_timeout() {
                    ToolError::failed(format!("fetch timed out after {:?}", self.timeout))
                } else {
                    ToolError::failed(format!("fetch failed: {}", e))
                }
            })?,
        };

        let status = resp.status().as_u16();

        let bytes = select! {
            biased;
            _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
            b = resp.bytes() => b.map_err(|e| {
                ToolError::failed(format!("read body failed: {}", e))
            })?,
        };

        if bytes.len() > self.max_response_bytes {
            return Err(ToolError::failed(format!(
                "response exceeded {} bytes",
                self.max_response_bytes
            )));
        }

        let body = String::from_utf8_lossy(&bytes).into_owned();

        Ok(json!({
            "status": status,
            "url": url,
            "body_length": body.len(),
            "body": body,
        }))
    }
}

/// Extract the host portion of a URL.
///
/// Pulled out of the older moxxy `url_policy::extract_host`. Kept
/// stdlib-only to avoid a `url` crate dep for one function — the
/// matching rules (exact host, no scheme, no port) match
/// `AllowlistFile::allows` semantics.
fn url_host(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host_with_port = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = host_with_port
        .split('@')
        .next_back()
        .unwrap_or(host_with_port);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn build_browser_client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    use reqwest::header::{self, HeaderMap, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.5"),
    );

    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/123.0.0.0 Safari/537.36",
        )
        .default_headers(headers)
        .cookie_store(true)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_host_strips_scheme_path_port_and_userinfo() {
        assert_eq!(
            url_host("https://example.com/foo"),
            Some("example.com".into())
        );
        assert_eq!(
            url_host("http://example.com:8080"),
            Some("example.com".into())
        );
        assert_eq!(
            url_host("https://user:pw@example.com/foo"),
            Some("example.com".into())
        );
        assert_eq!(
            url_host("https://example.com?q=1#frag"),
            Some("example.com".into())
        );
        assert_eq!(url_host("example.com/foo"), Some("example.com".into()));
        assert_eq!(url_host(""), None);
    }
}
