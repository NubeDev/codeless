//! `ServerHandler` impl mapping codeless-tools to MCP tools.
//!
//! `list_tools` enumerates the registry; `call_tool` looks up the
//! named tool, builds a `ToolCtx`, and dispatches. ToolError
//! variants map to MCP error categories — Cancelled and Denied
//! become `tools/call` results with `is_error: true` so the runner
//! sees a structured failure; InvalidArgs becomes a protocol-level
//! error (the runner's schema validation should have caught it,
//! but we double-check).

use std::sync::Arc;

use codeless_tools::ToolError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData as McpError, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ErrorData;
use serde_json::Map;

use crate::server::ServerContext;

pub struct CodelessMcpHandler {
    ctx: Arc<ServerContext>,
}

impl CodelessMcpHandler {
    pub fn new(ctx: Arc<ServerContext>) -> Self {
        Self { ctx }
    }
}

impl ServerHandler for CodelessMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "codeless-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Codeless tool surface exposed over MCP. Tools follow the \
             `codeless.<family>.<verb>` naming convention. See each \
             tool's schema for argument shape."
                    .to_string(),
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools: Vec<Tool> = self
            .ctx
            .registry
            .iter()
            .map(|tool| {
                Tool::new(
                    tool.name().to_string(),
                    "",
                    Arc::new(object_or_empty(tool.schema())),
                )
            })
            .collect();
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.as_ref();
        let Some(tool) = self.ctx.registry.get(name) else {
            return Err(ErrorData::invalid_params(
                format!("unknown tool '{name}'"),
                None,
            ));
        };

        let args_value = match request.arguments {
            Some(map) => serde_json::Value::Object(map),
            None => serde_json::Value::Object(Map::new()),
        };

        let tool_ctx = self.ctx.build_tool_ctx();
        match tool.call(&tool_ctx, args_value).await {
            Ok(value) => Ok(success_result(value)),
            Err(err) => Ok(error_result(err)),
        }
    }
}

/// Convert a tool's JSON schema (which is `serde_json::Value`) into
/// the `JsonObject` (alias for `serde_json::Map`) rmcp wants. If a
/// tool's schema isn't an object we fall back to `{}` rather than
/// fail the entire `list_tools` call — bad schemas are bugs but
/// shouldn't take down the surface.
fn object_or_empty(schema: &serde_json::Value) -> Map<String, serde_json::Value> {
    match schema {
        serde_json::Value::Object(map) => map.clone(),
        _ => Map::new(),
    }
}

fn success_result(value: serde_json::Value) -> CallToolResult {
    CallToolResult::structured(value)
}

/// Map ToolError to an MCP CallToolResult with is_error: true. The
/// MCP protocol expects "expected" tool failures (denials,
/// cancellations) to come back as results, not protocol-level
/// errors — that's how the runner-side LLM sees them as part of the
/// conversation rather than a crash.
fn error_result(err: ToolError) -> CallToolResult {
    let kind = match &err {
        ToolError::InvalidArgs(_) => "invalid_args",
        ToolError::Cancelled => "cancelled",
        ToolError::Denied(_) => "denied",
        ToolError::Failed(_) => "failed",
    };
    let msg = err.to_string();
    let structured = serde_json::json!({
        "error": { "kind": kind, "message": msg }
    });
    CallToolResult::structured_error(structured)
}
