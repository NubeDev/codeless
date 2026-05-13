// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/session.rs
//
// Session lifecycle: open, close, list. Sessions are the cookie /
// storage jars; pages live inside a session. Per-call cancellation
// is delivered to the manager via the request future being dropped
// (the sidecar's per-request timeout is the hard backstop) — these
// calls are short-lived so we don't wire a cancel-select.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

pub struct BrowserSessionOpenTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserSessionOpenTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "user_agent": { "type": "string" },
                    "viewport": {
                        "type": "object",
                        "properties": {
                            "width": { "type": "integer" },
                            "height": { "type": "integer" }
                        }
                    },
                    "locale": { "type": "string" },
                    "ignore_https_errors": { "type": "boolean" }
                }
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserSessionOpenTool {
    fn name(&self) -> &str {
        "codeless.browser.session.open"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager.request("session.create", args, None).await
    }
}

pub struct BrowserSessionCloseTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserSessionCloseTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" }
                },
                "required": ["session_id"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserSessionCloseTool {
    fn name(&self) -> &str {
        "codeless.browser.session.close"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager.request("session.close", args, None).await
    }
}

pub struct BrowserSessionListTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserSessionListTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({ "type": "object", "properties": {} }),
        }
    }
}

#[async_trait]
impl Tool for BrowserSessionListTool {
    fn name(&self) -> &str {
        "codeless.browser.session.list"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, _args: Value) -> Result<Value, ToolError> {
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager.request("session.list", json!({}), None).await
    }
}
