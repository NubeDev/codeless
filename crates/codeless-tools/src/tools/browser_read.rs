// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/core.rs
//
// browser.read: pull the rendered content of a tab.
//
// Scope deliberately narrow: HTML mode only. Moxxy's markdown / text
// modes depend on its html_text module — that's a separate port and
// belongs in a later sub-tick. HTML is what the sidecar returns
// directly; callers that want markdown can run their own extractor
// over it, and a future codeless.html.extract tool can take that
// over.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
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
                    "max_bytes": {
                        "type": "integer",
                        "description": "Hard cap on the HTML returned. Sidecar clamps to its own ceiling."
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

        Ok(json!({
            "title": raw.get("title").cloned().unwrap_or(Value::Null),
            "html": raw.get("html").cloned().unwrap_or(Value::Null),
            "byte_length": raw.get("byte_length").cloned().unwrap_or(json!(0)),
            "truncated": raw.get("truncated").cloned().unwrap_or(json!(false)),
            "final_url": raw.get("final_url").cloned().unwrap_or(Value::Null),
        }))
    }
}
