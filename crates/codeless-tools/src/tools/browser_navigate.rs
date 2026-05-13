// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/core.rs
//
// browser.navigate: drive an existing session to a URL. NetworkMode +
// allowlist gating reuses the codeless-tools policy helper, same as
// browse.fetch and http.request. wait_until + timeout_ms are passed
// through to the sidecar unchanged.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

pub struct BrowserNavigateTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserNavigateTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "url": { "type": "string" },
                    "page_id": {
                        "type": "string",
                        "description": "Optional existing tab to reuse. New tab if omitted."
                    },
                    "wait_until": {
                        "type": "string",
                        "enum": ["load", "domcontentloaded", "networkidle", "commit"],
                        "description": "Default 'load'."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Per-call ceiling. Clamped by BrowserManagerConfig::max_timeout."
                    }
                },
                "required": ["session_id", "url"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserNavigateTool {
    fn name(&self) -> &str {
        "codeless.browser.navigate"
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

        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let timeout = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(Duration::from_millis);

        self.manager.request("page.goto", args, timeout).await
    }
}
