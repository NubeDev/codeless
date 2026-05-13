// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/browser/interact.rs
//
// Five interaction tools — click, type, fill, hover, scroll —
// sharing one dispatch shape: validate the required string args,
// hand the whole args object to the sidecar, propagate timeout_ms.
//
// Moxxy used a declarative macro to remove the boilerplate; codeless
// keeps the structs explicit so each one's required-args validation
// stays visible in its `call`. Five structs is not enough for a
// macro to earn its keep here.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::browser::BrowserManager;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;

fn timeout_from_args(args: &Value) -> Option<Duration> {
    args.get("timeout_ms")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
}

fn require_string(args: &Value, key: &str) -> Result<(), ToolError> {
    if args.get(key).and_then(Value::as_str).is_none() {
        return Err(ToolError::invalid_args(format!("missing '{key}'")));
    }
    Ok(())
}

pub struct BrowserClickTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserClickTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selector": { "type": "string" },
                    "button": { "type": "string", "enum": ["left", "right", "middle"] },
                    "click_count": { "type": "integer" },
                    "force": { "type": "boolean" },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id", "selector"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserClickTool {
    fn name(&self) -> &str {
        "codeless.browser.click"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        require_string(&args, "page_id")?;
        require_string(&args, "selector")?;
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager
            .request("page.click", args.clone(), timeout_from_args(&args))
            .await
    }
}

pub struct BrowserTypeTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserTypeTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selector": { "type": "string" },
                    "text": { "type": "string" },
                    "delay_ms": { "type": "integer" },
                    "clear_first": { "type": "boolean" },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id", "selector", "text"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserTypeTool {
    fn name(&self) -> &str {
        "codeless.browser.type"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        require_string(&args, "page_id")?;
        require_string(&args, "selector")?;
        require_string(&args, "text")?;
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager
            .request("page.type", args.clone(), timeout_from_args(&args))
            .await
    }
}

pub struct BrowserFillTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserFillTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selector": { "type": "string" },
                    "value": { "type": "string" },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id", "selector", "value"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserFillTool {
    fn name(&self) -> &str {
        "codeless.browser.fill"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        require_string(&args, "page_id")?;
        require_string(&args, "selector")?;
        require_string(&args, "value")?;
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager
            .request("page.fill", args.clone(), timeout_from_args(&args))
            .await
    }
}

pub struct BrowserHoverTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserHoverTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selector": { "type": "string" },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id", "selector"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserHoverTool {
    fn name(&self) -> &str {
        "codeless.browser.hover"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        require_string(&args, "page_id")?;
        require_string(&args, "selector")?;
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager
            .request("page.hover", args.clone(), timeout_from_args(&args))
            .await
    }
}

pub struct BrowserScrollTool {
    manager: Arc<BrowserManager>,
    schema: Value,
}

impl BrowserScrollTool {
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            schema: json!({
                "type": "object",
                "properties": {
                    "page_id": { "type": "string" },
                    "selector": {
                        "type": "string",
                        "description": "Scroll a specific element into view. Mutually exclusive with direction."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["top", "bottom", "to"],
                        "description": "Whole-page scroll target. With 'to', pass x and y."
                    },
                    "x": { "type": "integer" },
                    "y": { "type": "integer" },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["page_id"]
            }),
        }
    }
}

#[async_trait]
impl Tool for BrowserScrollTool {
    fn name(&self) -> &str {
        "codeless.browser.scroll"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        require_string(&args, "page_id")?;
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        self.manager
            .request("page.scroll", args.clone(), timeout_from_args(&args))
            .await
    }
}
