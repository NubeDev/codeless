// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/http.rs
//
// Scope: GET/POST/PUT/PATCH/DELETE/HEAD with optional body and
// custom headers. Same NetworkMode + AllowlistFile gate as
// codeless.browse.fetch. Response headers are returned to the
// caller; redaction is the caller's job (the tool surface stays
// transport-faithful so callers can build their own redaction
// layers without fighting hidden filtering).

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio::select;

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct HttpRequestTool {
    schema: Value,
    timeout: Duration,
    max_response_bytes: usize,
}

impl HttpRequestTool {
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
                        "description": "Absolute http(s) URL to request."
                    },
                    "method": {
                        "type": "string",
                        "description": "HTTP method. Defaults to GET.",
                        "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]
                    },
                    "body": {
                        "type": "string",
                        "description": "Request body. Sent verbatim. Default content-type is application/json when a body is present and headers do not override."
                    },
                    "headers": {
                        "type": "object",
                        "description": "Additional headers as a string-keyed map; values are strings.",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["url"]
            }),
            timeout,
            max_response_bytes,
        }
    }
}

impl Default for HttpRequestTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "codeless.http.request"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'url'"))?;

        let host =
            super::url_host(url).ok_or_else(|| ToolError::invalid_args("URL has no host"))?;

        super::check_network_policy(ctx, &host, url)?;

        let method = args
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_ascii_uppercase();

        let body = args.get("body").and_then(Value::as_str);
        let headers = args.get("headers").and_then(Value::as_object);

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| ToolError::failed(format!("client build failed: {}", e)))?;

        let mut req = match method.as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
            "HEAD" => client.head(url),
            other => {
                return Err(ToolError::invalid_args(format!(
                    "unsupported HTTP method '{}'",
                    other
                )));
            }
        };

        let mut content_type_set = false;
        if let Some(map) = headers {
            for (k, v) in map {
                let Some(value) = v.as_str() else {
                    return Err(ToolError::invalid_args(format!(
                        "header '{}' is not a string",
                        k
                    )));
                };
                if k.eq_ignore_ascii_case("content-type") {
                    content_type_set = true;
                }
                req = req.header(k.as_str(), value);
            }
        }

        if let Some(b) = body {
            if !content_type_set {
                req = req.header("content-type", "application/json");
            }
            req = req.body(b.to_string());
        }

        let resp = select! {
            biased;
            _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
            r = req.send() => r.map_err(|e| {
                if e.is_timeout() {
                    ToolError::failed(format!("request timed out after {:?}", self.timeout))
                } else {
                    ToolError::failed(format!("request failed: {}", e))
                }
            })?,
        };

        let status = resp.status().as_u16();
        let resp_headers = collect_headers(resp.headers());

        let bytes = select! {
            biased;
            _ = ctx.cancel_token().cancelled() => return Err(ToolError::Cancelled),
            b = resp.bytes() => b.map_err(|e| ToolError::failed(format!("read body failed: {}", e)))?,
        };

        if bytes.len() > self.max_response_bytes {
            return Err(ToolError::failed(format!(
                "response exceeded {} bytes",
                self.max_response_bytes
            )));
        }

        let body_str = String::from_utf8_lossy(&bytes).into_owned();

        Ok(json!({
            "status": status,
            "headers": Value::Object(resp_headers),
            "body": body_str,
            "body_length": body_str.len(),
        }))
    }
}

/// Materialise response headers as a JSON map.
///
/// Non-UTF-8 header values are dropped rather than represented as
/// bytes — coding-job callers don't have a way to act on raw bytes
/// through JSON, and surfacing them as garbled strings is worse
/// than not surfacing them at all.
fn collect_headers(headers: &reqwest::header::HeaderMap) -> Map<String, Value> {
    let mut out = Map::with_capacity(headers.len());
    for (k, v) in headers {
        if let Ok(value) = v.to_str() {
            out.insert(k.as_str().to_string(), Value::String(value.to_string()));
        }
    }
    out
}
