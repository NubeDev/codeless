// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/core.rs
//
// browser.extract, browser.wait, browser.cookies. Thin pass-through
// tools — every sidecar method handles its own argument validation,
// and there's no codeless-side gating to apply beyond cancellation +
// page_id presence. Grouped into one file because each is < 40 LOC.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

/// Helper: require page_id, short-circuit on cancellation, dispatch
/// to a sidecar method.
async fn dispatch_with_page_id(
    manager: &BrowserManager,
    ctx: &ToolCtx,
    method: &str,
    args: Value,
    timeout: Option<Duration>,
) -> Result<Value, ToolError> {
    if args.get("page_id").and_then(Value::as_str).is_none() {
        return Err(ToolError::invalid_args("missing 'page_id'"));
    }
    if ctx.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    manager.request(method, args, timeout).await
}

pub struct BrowserExtractTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserExtractTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selectors": {
                        "type": "object",
                        "description": "Map of field name to CSS selector. Runs against the live DOM after JS execution."
                    }
                },
                "required": ["page_id", "selectors"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserExtractTool {
    fn name(&self) -> &str {
        "codeless.browser.extract"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if args.get("selectors").and_then(Value::as_object).is_none() {
            return Err(ToolError::invalid_args(
                "missing 'selectors' (object of field -> CSS selector)",
            ));
        }
        dispatch_with_page_id(&self.manager, ctx, "page.extract", args, None).await
    }
}

pub struct BrowserWaitTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserWaitTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selector": { "type": "string" },
                    "state": { "type": "string", "enum": ["attached", "detached", "visible", "hidden"] },
                    "load_state": { "type": "string", "enum": ["load", "domcontentloaded", "networkidle"] },
                    "delay_ms": { "type": "integer" },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserWaitTool {
    fn name(&self) -> &str {
        "codeless.browser.wait"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let timeout = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map(Duration::from_millis);
        dispatch_with_page_id(&self.manager, ctx, "page.wait_for", args, timeout).await
    }
}

pub struct BrowserCookiesTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserCookiesTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "action": { "type": "string", "enum": ["get", "set", "clear"] },
                    "cookies": {
                        "type": "array",
                        "description": "Cookies to set when action='set'. Each follows Playwright's cookie shape."
                    }
                },
                "required": ["page_id", "action"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserCookiesTool {
    fn name(&self) -> &str {
        "codeless.browser.cookies"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if args.get("action").and_then(Value::as_str).is_none() {
            return Err(ToolError::invalid_args(
                "missing 'action' (get / set / clear)",
            ));
        }
        dispatch_with_page_id(&self.manager, ctx, "page.cookies", args, None).await
    }
}
