//! `codeless-mcp` binary entry point. Stdio-only MVP.
//!
//! The binary builds an empty `ToolRegistry` plus the standard
//! codeless tools and serves them over stdio. Production callers
//! that need to inject a `BrowserManager` (for the browser.* tools)
//! will use the library crate directly — this `main` is the
//! convenience surface for "just run a Codeless MCP server" and is
//! what Claude Code's `claude --mcp-config <file>` spawns when the
//! config points at our binary.
//!
//! Args today: none. The first envar that matters is
//! `CODELESS_WORKTREE_ROOT` (defaults to the current dir).

use std::path::PathBuf;
use std::sync::Arc;

use codeless_mcp::{serve_stdio, ServerContext};
use codeless_tools::tools::{BrowseFetchTool, HttpRequestTool};
use codeless_tools::ToolRegistry;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Tracing goes to stderr because stdout is the MCP transport.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,codeless_mcp=debug,codeless_tools=debug")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let worktree_root = std::env::var_os("CODELESS_WORKTREE_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    // Default-tools registry: the always-on subset that doesn't need
    // a BrowserManager. Browser tools register from a different
    // entry point (library API) when a manager is available.
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(BrowseFetchTool::new()));
    registry.register(Arc::new(HttpRequestTool::new()));

    tracing::info!(
        worktree = %worktree_root.display(),
        tool_count = registry.len(),
        "codeless-mcp serving stdio"
    );

    let ctx = ServerContext::new(Arc::new(registry), worktree_root);
    serve_stdio(ctx).await
}
