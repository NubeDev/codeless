//! Codeless tools — the LLM-callable surface.
//!
//! Every tool a runner subprocess can invoke lives here. Tools are
//! exposed through `codeless-mcp` as MCP tools; runners (Claude Code,
//! Codex, future) see them as ordinary MCP tools.
//!
//! Host-only by dependency edges, matching the pattern used by
//! `codeless-adapters-host`: this crate is never declared as a
//! dependency from any iOS- or Android-safe crate
//! (`codeless-types`, `codeless-rpc`, `codeless-client`,
//! `codeless-tauri-mobile`). The R1 grep in CI enforces this.
//!
//! Design intent lives in `DOCS/TOOLS-PORTING.md` in the outer
//! workspace.

pub mod attachment;
pub mod browser;
mod ctx;
pub mod email;
mod error;
pub mod html_text;
pub mod plan;
pub mod plugin;
pub mod policy;
mod registry;
pub mod runtime_adapter;
pub mod schedule;
pub mod testing;
mod tool;
pub mod tools;

pub use ctx::ToolCtx;
pub use error::ToolError;
pub use registry::ToolRegistry;
pub use tool::Tool;
