use async_trait::async_trait;
use serde_json::Value;

use crate::ctx::ToolCtx;
use crate::error::ToolError;

/// Contract every tool implements.
///
/// JSON in, JSON out, async, with cancellation delivered through
/// `ToolCtx`. Schema is on the trait (not optional) because MCP
/// requires it — runners advertise tools to the LLM with their
/// schemas and validate args against them.
///
/// Streaming output (shell stdout chunks, browser progress events)
/// is deferred to a future `call_streaming` method; non-streaming
/// tools keep this cheap call path.
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    /// Stable identifier exposed to MCP. Dotted, codeless-prefixed
    /// convention: `codeless.browse.fetch`. Must be unique in a
    /// `ToolRegistry`.
    fn name(&self) -> &str;

    /// JSON Schema for the args object. `codeless-mcp` advertises
    /// this to runners; runners validate before calling.
    fn schema(&self) -> &Value;

    /// Invoke the tool. Implementations should poll
    /// `ctx.is_cancelled()` at every await point that could be
    /// load-bearing and return `ToolError::Cancelled` when set —
    /// this gives the dispatcher a structured signal instead of a
    /// silently-dropped future.
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<Value, ToolError>;
}
