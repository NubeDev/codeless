//! Server lifecycle: build a `ToolCtx` per call, wire the registry
//! into MCP, run stdio.

use std::path::PathBuf;
use std::sync::Arc;

use codeless_tools::policy::{AllowlistFile, NetworkMode};
use codeless_tools::{ToolCtx, ToolRegistry};
use rmcp::transport::stdio;
use rmcp::ServiceExt;
use tokio_util::sync::CancellationToken;
use tracing::Span;

use crate::audit::{AuditSink, NullAuditSink};
use crate::contrib::McpContributionTable;
use crate::handler::CodelessMcpHandler;
use crate::personas::{EmptyPersonaSource, PersonaSource};

/// Per-invocation context for the server. Owns the `ToolRegistry`
/// and the policy / worktree state every `Tool::call` receives via
/// `ToolCtx`. Built by the host binary at startup.
///
/// Why this is its own type rather than just `Arc<ToolRegistry>`:
/// tools take a `ToolCtx` whose `worktree_root`, `network_mode`,
/// and `allowlist` live with the *job*, not with the registry. The
/// long-term shape is one `ServerContext` per running coding job,
/// constructed by the codeless runtime when the job spawns. For
/// today's stdio MVP one global context covers it — a single agent
/// with one worktree.
pub struct ServerContext {
    pub registry: Arc<ToolRegistry>,
    pub worktree_root: PathBuf,
    pub network_mode: NetworkMode,
    pub allowlist: AllowlistFile,
    pub cancel: CancellationToken,
    /// Catalogue the MCP prompts surface reads from. Defaults to
    /// `EmptyPersonaSource` so a server constructed without a DB
    /// simply advertises an empty prompt list (stage 10 / D3 —
    /// `use_for_jobs = 1` is the sole gate, but a missing catalogue
    /// behaves the same as "no personas qualify").
    pub personas: Arc<dyn PersonaSource>,
    /// Plugin MCP contributions (PLUGIN-MCP.md § Manifest). The host
    /// builds this once at boot from `PluginRegistry` after the
    /// parity check (`codeless_tools::plugin::check_mcp_parity`)
    /// has cleared each plugin. Defaults to empty so a server
    /// without any plugins behaves identically to the pre-stage-14
    /// surface.
    pub contributions: McpContributionTable,
    /// Audit sink invoked once per `tools/call` (PLUGIN-MCP.md
    /// § Audit). Defaults to `NullAuditSink` so the production
    /// stdio binary doesn't grow a buffer until an aggregator is
    /// wired; tests substitute `InMemoryAuditSink`.
    pub audit: Arc<dyn AuditSink>,
}

impl ServerContext {
    pub fn new(registry: Arc<ToolRegistry>, worktree_root: PathBuf) -> Self {
        Self {
            registry,
            worktree_root,
            network_mode: NetworkMode::default(),
            allowlist: AllowlistFile::new(),
            cancel: CancellationToken::new(),
            personas: Arc::new(EmptyPersonaSource),
            contributions: McpContributionTable::default(),
            audit: Arc::new(NullAuditSink),
        }
    }

    pub fn with_contributions(mut self, contributions: McpContributionTable) -> Self {
        self.contributions = contributions;
        self
    }

    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = audit;
        self
    }

    pub fn with_network(mut self, mode: NetworkMode, allowlist: AllowlistFile) -> Self {
        self.network_mode = mode;
        self.allowlist = allowlist;
        self
    }

    pub fn with_personas(mut self, personas: Arc<dyn PersonaSource>) -> Self {
        self.personas = personas;
        self
    }

    /// Build the `ToolCtx` a single tool call receives. Constructed
    /// fresh per call so the cancellation token is per-call: a
    /// single `tools/call` cancellation does not abort other
    /// concurrent calls sharing the same server.
    pub(crate) fn build_tool_ctx(&self) -> ToolCtx {
        let cancel = self.cancel.child_token();
        ToolCtx::new(
            self.worktree_root.clone(),
            self.network_mode.clone(),
            self.allowlist.clone(),
            cancel,
            Span::current(),
        )
    }
}

/// Run the MCP server over stdio until the client disconnects or a
/// fatal transport error fires.
pub async fn serve_stdio(
    ctx: ServerContext,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let handler = CodelessMcpHandler::new(Arc::new(ctx));
    let service = handler.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
