//! Codeless MCP server.
//!
//! Bridges `codeless-tools`'s `ToolRegistry` to the Model Context
//! Protocol. A runner subprocess (Claude Code, Codex, ...) spawns
//! this server over stdio, sees every registered tool as a native
//! MCP tool, and invokes them through the standard MCP `tools/call`
//! flow.
//!
//! Single trust boundary: stdio means the child process inherits
//! the parent's permissions ([SCOPE](`../DOCS/SCOPE.md`) R5). No
//! bearer token, no auth middleware — the security boundary is the
//! parent process choosing to spawn this server with a specific
//! `ToolRegistry`.

pub mod audit;
pub mod contrib;
pub mod handler;
pub mod personas;
pub mod server;

pub use audit::{AuditSink, InMemoryAuditSink, McpAuditEvent, McpCallOutcome, NullAuditSink};
pub use contrib::{
    rows_for_loaded_plugin, McpContribution, McpContributionTable, ResolvedMcpDispatch,
};
pub use handler::CodelessMcpHandler;
pub use personas::{EmptyPersonaSource, PersonaSource, SqlitePersonaSource};
pub use server::{serve_stdio, ServerContext};
