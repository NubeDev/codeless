// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/core.rs
//
// browser.eval — run an arbitrary JS expression in the page's
// context. POWERFUL: do not register this tool in jobs that don't
// need it. Expression is hard-capped at 8 KiB to match the moxxy
// ceiling; the sidecar wraps the string in an async IIFE.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

const MAX_EXPRESSION_BYTES: usize = 8 * 1024;

pub struct BrowserEvalTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserEvalTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "expression": {
                        "type": "string",
                        "description": "JavaScript expression. Wrapped in an async IIFE by the sidecar. Capped at 8 KiB."
                    },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id", "expression"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserEvalTool {
    fn name(&self) -> &str {
        "codeless.browser.eval"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if args.get("page_id").and_then(Value::as_str).is_none() {
            return Err(ToolError::invalid_args("missing 'page_id'"));
        }
        let expr = args
            .get("expression")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'expression'"))?;
        if expr.len() > MAX_EXPRESSION_BYTES {
            return Err(ToolError::invalid_args(format!(
                "expression exceeds {MAX_EXPRESSION_BYTES} bytes"
            )));
        }
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let timeout = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(Duration::from_millis);
        self.manager.request("page.eval", args, timeout).await
    }
}
