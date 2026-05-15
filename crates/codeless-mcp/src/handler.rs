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
use codeless_types::Persona;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData as McpError, GetPromptRequestParams,
    GetPromptResult, Implementation, ListPromptsResult, ListToolsResult, PaginatedRequestParams,
    Prompt, PromptMessage, PromptMessageContent, PromptMessageRole, ProtocolVersion,
    ServerCapabilities, ServerInfo, Tool,
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
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new(
            "codeless-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "Codeless tool surface exposed over MCP. Tools follow the \
             `codeless.<family>.<verb>` naming convention; prompts are \
             personas whose `use_for_jobs` flag is set. See each \
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

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(self.list_prompts_inner().await)
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        self.get_prompt_inner(&request.name).await
    }
}

impl CodelessMcpHandler {
    /// Pure-function form of `list_prompts` so unit tests can call
    /// it without faking a `RequestContext`. Every persona with
    /// `use_for_jobs = 1` becomes one prompt; the id is the prompt
    /// name. No arguments — the persona's `instructions` field is the
    /// whole prompt body (D4 keeps snippet expansion chat-only).
    pub async fn list_prompts_inner(&self) -> ListPromptsResult {
        let prompts = self
            .ctx
            .personas
            .list_for_jobs()
            .await
            .into_iter()
            .map(persona_to_prompt)
            .collect();
        ListPromptsResult {
            prompts,
            ..Default::default()
        }
    }

    /// Pure-function form of `get_prompt`. The `use_for_jobs = 1`
    /// gate applies here too — a chat-only persona that the caller
    /// somehow asked for by id is reported as not-found, never
    /// rendered. Same rule for an unknown id and a hard-deleted row.
    pub async fn get_prompt_inner(&self, name: &str) -> Result<GetPromptResult, McpError> {
        let Some(persona) = self.ctx.personas.get(name).await else {
            return Err(ErrorData::invalid_params(
                format!("unknown prompt '{name}'"),
                None,
            ));
        };
        if !persona.use_for_jobs {
            return Err(ErrorData::invalid_params(
                format!("prompt '{name}' is not exposed (use_for_jobs is false)"),
                None,
            ));
        }
        Ok(persona_to_prompt_result(persona))
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

/// A persona becomes an MCP `Prompt` advert: id is the wire name
/// (callers quote it verbatim in `prompts/get`), the persona's
/// human name is the title, the persona's description is the
/// `description` field shown to a model selecting a prompt. No
/// `arguments` are advertised — the prompt body is fully resolved
/// from the persona row itself (D4 defers snippet expansion).
fn persona_to_prompt(persona: Persona) -> Prompt {
    let description = if persona.description.is_empty() {
        None
    } else {
        Some(persona.description)
    };
    let mut prompt = Prompt::new(persona.id, description, None);
    if !persona.name.is_empty() {
        prompt = prompt.with_title(persona.name);
    }
    prompt
}

/// `prompts/get` renders the persona as a single user-role message
/// containing the persona's `instructions` (D4: chat-only snippet
/// resolution stays out of the job-time composition path; MCP
/// inherits the same rule because an MCP prompt *is* a way to
/// start a job using that persona).
fn persona_to_prompt_result(persona: Persona) -> GetPromptResult {
    let messages = vec![PromptMessage::new(
        PromptMessageRole::User,
        PromptMessageContent::text(persona.instructions),
    )];
    let mut result = GetPromptResult::new(messages);
    if !persona.description.is_empty() {
        result = result.with_description(persona.description);
    }
    result
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use async_trait::async_trait;
    use codeless_tools::ToolRegistry;
    use codeless_types::{Persona, UnixMillis};

    use super::*;
    use crate::personas::PersonaSource;
    use crate::server::ServerContext;

    struct FixturePersonas(Vec<Persona>);

    #[async_trait]
    impl PersonaSource for FixturePersonas {
        async fn list_for_jobs(&self) -> Vec<Persona> {
            self.0.iter().filter(|p| p.use_for_jobs).cloned().collect()
        }

        async fn get(&self, id: &str) -> Option<Persona> {
            self.0.iter().find(|p| p.id == id).cloned()
        }
    }

    fn fixture(id: &str, use_for_jobs: bool) -> Persona {
        Persona {
            id: id.to_string(),
            name: format!("name:{id}"),
            description: format!("desc:{id}"),
            icon: "coder".to_string(),
            instructions: format!("INSTRUCTIONS:{id}"),
            use_for_jobs,
            default_model: None,
            allowed_subagents: Vec::new(),
            default_snippets: Vec::new(),
            built_in: false,
            created_at: UnixMillis::from(0),
            updated_at: UnixMillis::from(0),
        }
    }

    fn handler_with(personas: Vec<Persona>) -> CodelessMcpHandler {
        let ctx = ServerContext::new(Arc::new(ToolRegistry::new()), PathBuf::from("."))
            .with_personas(Arc::new(FixturePersonas(personas)));
        CodelessMcpHandler::new(Arc::new(ctx))
    }

    #[tokio::test]
    async fn list_prompts_filters_on_use_for_jobs() {
        let handler = handler_with(vec![
            fixture("builtin:coder", true),
            fixture("builtin:designer", false),
            fixture("custom:ops", true),
        ]);
        let result = handler.list_prompts_inner().await;
        let names: Vec<&str> = result.prompts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["builtin:coder", "custom:ops"]);
    }

    #[tokio::test]
    async fn list_prompts_surfaces_name_and_description() {
        let handler = handler_with(vec![fixture("builtin:coder", true)]);
        let prompt = handler.list_prompts_inner().await.prompts.remove(0);
        assert_eq!(prompt.name, "builtin:coder");
        assert_eq!(prompt.title.as_deref(), Some("name:builtin:coder"));
        assert_eq!(prompt.description.as_deref(), Some("desc:builtin:coder"));
        assert!(prompt.arguments.is_none(), "D4: no advertised arguments");
    }

    #[tokio::test]
    async fn get_prompt_renders_instructions_as_user_message() {
        let handler = handler_with(vec![fixture("builtin:coder", true)]);
        let result = handler.get_prompt_inner("builtin:coder").await.unwrap();
        assert_eq!(result.messages.len(), 1);
        assert!(matches!(result.messages[0].role, PromptMessageRole::User));
        match &result.messages[0].content {
            PromptMessageContent::Text { text } => {
                assert_eq!(text, "INSTRUCTIONS:builtin:coder");
            }
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_prompt_rejects_chat_only_persona() {
        let handler = handler_with(vec![fixture("builtin:designer", false)]);
        let err = handler
            .get_prompt_inner("builtin:designer")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not exposed") || msg.contains("use_for_jobs"),
            "got {msg}"
        );
    }

    #[tokio::test]
    async fn get_prompt_unknown_id_is_not_found() {
        let handler = handler_with(vec![]);
        let err = handler.get_prompt_inner("builtin:nope").await.unwrap_err();
        assert!(err.to_string().contains("unknown"));
    }
}
