// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/core.rs
//
// browser.read: pull the rendered content of a tab. Three modes:
//   - "html"     (default): raw HTML the sidecar returned.
//   - "markdown": clean readable markdown via html_text; includes
//                 the resolved link list.
//   - "text"     : same extraction as markdown but the structural
//                 syntax (headings, lists) is included — moxxy
//                 treats text and markdown as the same surface
//                 (the markdown structure *is* the text format).
//
// In every mode the sidecar's response fields (title, byte_length,
// truncated, final_url) pass through.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::html_text;
use crate::tool::Tool;

pub struct BrowserReadTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserReadTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "mode": {
                        "type": "string",
                        "enum": ["html", "markdown", "text"],
                        "description": "Default 'html'. 'markdown' and 'text' run codeless's HTML-to-markdown extractor and also include a resolved link list."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Hard cap on the raw HTML the sidecar returns before extraction. Sidecar clamps to its own ceiling."
                    },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserReadTool {
    fn name(&self) -> &str {
        "codeless.browser.read"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if args.get("page_id").and_then(Value::as_str).is_none() {
            return Err(ToolError::invalid_args("missing 'page_id'"));
        }
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("html")
            .to_string();
        match mode.as_str() {
            "html" | "markdown" | "text" => {}
            other => {
                return Err(ToolError::invalid_args(format!(
                    "unknown mode '{other}' — expected html / markdown / text"
                )));
            }
        }
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let timeout = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(Duration::from_millis);

        let mut sidecar_params = json!({ "page_id": args["page_id"] });
        if let Some(b) = args.get("max_bytes").and_then(Value::as_u64) {
            sidecar_params["max_bytes"] = json!(b);
        }
        let raw = self
            .manager
            .request("page.read", sidecar_params, timeout)
            .await?;

        let title = raw.get("title").and_then(Value::as_str).unwrap_or("");
        let html = raw.get("html").and_then(Value::as_str).unwrap_or("");
        let final_url = raw.get("final_url").and_then(Value::as_str).unwrap_or("");
        let byte_length = raw.get("byte_length").cloned().unwrap_or(json!(0));
        let truncated = raw.get("truncated").cloned().unwrap_or(json!(false));

        match mode.as_str() {
            "html" => Ok(json!({
                "title": title,
                "html": html,
                "byte_length": byte_length,
                "truncated": truncated,
                "final_url": final_url,
            })),
            "markdown" | "text" => {
                let (text, links) = html_text::extract_text_and_links(html, final_url);
                let links_json: Vec<Value> = links
                    .into_iter()
                    .map(|l| json!({ "url": l.url, "text": l.text }))
                    .collect();
                Ok(json!({
                    "title": title,
                    "text": text,
                    "links": links_json,
                    "byte_length": byte_length,
                    "truncated": truncated,
                    "final_url": final_url,
                }))
            }
            _ => unreachable!("validated above"),
        }
    }
}
