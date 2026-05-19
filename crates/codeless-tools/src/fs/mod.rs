//! Filesystem tool surface for the Assistant planner.
//!
//! Three read-only tools — `fs.list`, `fs.read`, `fs.search` — share
//! a single [`Sandbox`] that pins every path under the assistant
//! thread's workspace root. Absolute inputs and `..` traversal are
//! rejected before any I/O; the canonical form of the resolved path
//! is re-checked against the canonical root so a checked-in symlink
//! pointing outside the workspace fails the same way as a literal
//! `..` (SCOPE-ASSISTANT-FS D4).
//!
//! The tools are intended to be registered on a per-thread tool
//! registry the assistant planner advertises to its CLI runner;
//! job runners reach the filesystem through their own runner-side
//! capabilities and do not consume this surface.

mod fs_list;
mod fs_read;
mod fs_search;
mod sandbox;

use std::sync::Arc;

pub use fs_list::FsListTool;
pub use fs_read::{FsReadTool, READ_BYTE_CAP};
pub use fs_search::{FsSearchTool, SEARCH_MATCH_CAP};
pub use sandbox::Sandbox;

use crate::registry::ToolRegistry;

/// Register the read-only filesystem tools (`fs.list`, `fs.read`,
/// `fs.search`) on a tool registry. Caller owns the registry and is
/// responsible for scoping it to an assistant thread; this helper is
/// the seam the planner uses so the registration list is in one place
/// and the per-thread caller does not have to know each tool's
/// constructor.
///
/// `sandbox` carries the thread's workspace root; the three tools
/// share an `Arc` so a single canonicalisation per call site is
/// amortised across them.
pub fn register_assistant_thread_read_tools(registry: &mut ToolRegistry, sandbox: Arc<Sandbox>) {
    registry.register(Arc::new(FsListTool::new(Arc::clone(&sandbox))));
    registry.register(Arc::new(FsReadTool::new(Arc::clone(&sandbox))));
    registry.register(Arc::new(FsSearchTool::new(sandbox)));
}
