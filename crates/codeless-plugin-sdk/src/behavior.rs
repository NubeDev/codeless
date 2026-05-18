use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Serialize};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tier::Tier;

/// Compile-time identity of a tool. Emitted by `#[derive(Tool)]`
/// from the `#[tool(id = "...", tier = "...")]` attribute and read
/// back by [`crate::Manifest::for_behavior`] without a runtime
/// allocation.
///
/// Split from [`ToolBehavior`] for one reason: the derive can fill
/// in the constants from the attribute, but cannot infer the
/// `Args` / `Output` associated types or the `call` body. Two traits
/// let the derive emit a complete impl block of its own without
/// touching the user's `impl ToolBehavior` block.
pub trait ToolMeta: 'static {
    const ID: &'static str;
    const TIER: Tier;
    const DESCRIPTION: &'static str = "";
}

/// The single authoring trait every plugin tool implements.
///
/// One source -> two flavours by Cargo feature (`builtin` /
/// `wasm`). The trait does not change between flavours; only the
/// glue the [`crate::register!`] macro emits does. PLUGIN-WASM.md
/// "The authoring SDK" is the load-bearing claim:
///
/// > The author writes the same source either way. This is the
/// > load-bearing claim of the substrate; if it stops being true,
/// > we have failed.
///
/// Async because the builtin flavour runs on tokio and the WASM
/// flavour's host bridge wraps the synchronous `call` export in an
/// `async` future for backpressure with the rest of the runtime.
/// Plugin code that has nothing to await is free to write a plain
/// `async fn` body.
#[async_trait]
pub trait ToolBehavior: ToolMeta + Send + Sync {
    /// Deserialised arg type. `JsonSchema` is required so the
    /// [`crate::Manifest`] can advertise the input schema to MCP
    /// clients and the runner can validate before `call` fires.
    type Args: JsonSchema + DeserializeOwned + Send;

    /// Serialised output type. The same `JsonSchema` requirement
    /// lets the Assistant agent loop walk the schema for
    /// `$ref: codeless://attachment` markers (PS7) without per-plugin
    /// UI code.
    type Output: JsonSchema + Serialize + Send;

    /// Invoke the tool. Implementations should poll for cooperative
    /// cancellation through `ctx` at every `await` point that could
    /// matter; today the type is opaque, but the future
    /// `ctx.is_cancelled()` query lands here once the per-flavour
    /// adapter wires it through.
    async fn call(&self, ctx: &ToolCtx, args: Self::Args) -> Result<Self::Output, ToolError>;
}
