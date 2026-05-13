//! End-to-end smoke: register a no-op tool, dispatch through the
//! registry, drive it from the fake context. This is the proof that
//! the T2 surface (Tool, ToolCtx, ToolRegistry, fake_ctx) composes —
//! before any real tool lands in T5.

use std::sync::Arc;

use async_trait::async_trait;
use codeless_tools::testing::fake_ctx;
use codeless_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use serde_json::{json, Value};

struct Echo {
    schema: Value,
}

impl Echo {
    fn new() -> Self {
        Self {
            schema: json!({
                "type": "object",
                "properties": { "msg": { "type": "string" } },
                "required": ["msg"]
            }),
        }
    }
}

#[async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        "test.echo"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        if ctx.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let msg = args
            .get("msg")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing `msg`"))?;
        Ok(json!({ "echoed": msg }))
    }
}

#[tokio::test]
async fn registry_dispatches_to_registered_tool() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(Echo::new()));

    let harness = fake_ctx();
    let tool = registry.get("test.echo").expect("tool registered");
    let result = tool
        .call(&harness.ctx, json!({ "msg": "hello" }))
        .await
        .expect("echo succeeds");

    assert_eq!(result, json!({ "echoed": "hello" }));
}

#[tokio::test]
async fn cancellation_short_circuits_call() {
    let harness = fake_ctx();
    harness.cancel.cancel();

    let tool = Echo::new();
    let err = tool
        .call(&harness.ctx, json!({ "msg": "ignored" }))
        .await
        .expect_err("cancelled before work");

    assert!(matches!(err, ToolError::Cancelled));
}

#[tokio::test]
async fn invalid_args_surface_as_structured_error() {
    let harness = fake_ctx();
    let tool = Echo::new();
    let err = tool
        .call(&harness.ctx, json!({}))
        .await
        .expect_err("missing msg");

    assert!(matches!(err, ToolError::InvalidArgs(_)));
}
