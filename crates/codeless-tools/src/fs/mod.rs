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
//! Two write tools — `fs.write`, `fs.edit` — share the same sandbox
//! and delegate the actual mutation to a [`WriteDispatcher`] so the
//! Tool impls are identical across the `approve-edits` (card) and
//! `bypass` (write through) modes (D1). Job-scope writes
//! (`.codeless/jobs/<name>/...`) always route through the
//! dispatcher's `job_scope_write` method regardless of mode (D3) so
//! the paused-job rule cannot be bypassed.
//!
//! The tools are intended to be registered on a per-thread tool
//! registry the assistant planner advertises to its CLI runner;
//! job runners reach the filesystem through their own runner-side
//! capabilities and do not consume this surface.

pub mod dispatch;
mod fs_edit;
mod fs_list;
mod fs_read;
mod fs_search;
mod fs_write;
mod sandbox;

use std::sync::Arc;

pub use dispatch::{
    classify_target, JobScopeWrite, SharedWriteDispatcher, WorkspaceWrite, WriteDispatcher,
    WriteTarget, JOB_SCOPE_ROOT,
};
pub use fs_edit::FsEditTool;
pub use fs_list::FsListTool;
pub use fs_read::{FsReadTool, READ_BYTE_CAP};
pub use fs_search::{FsSearchTool, SEARCH_MATCH_CAP};
pub use fs_write::{FsWriteTool, WRITE_BYTE_CAP};
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

/// Register the write filesystem tools (`fs.write`, `fs.edit`) on a
/// tool registry. The caller resolves the thread's mode and only
/// invokes this helper for `approve-edits` and `bypass` — D8 omits
/// the tools entirely on `read-only`, so a planner running on that
/// mode never sees a fs.write to propose. The two tools share the
/// dispatcher so a single mode-aware seam backs them both.
pub fn register_assistant_thread_write_tools(
    registry: &mut ToolRegistry,
    sandbox: Arc<Sandbox>,
    dispatcher: SharedWriteDispatcher,
) {
    registry.register(Arc::new(FsWriteTool::new(
        Arc::clone(&sandbox),
        Arc::clone(&dispatcher),
    )));
    registry.register(Arc::new(FsEditTool::new(sandbox, dispatcher)));
}
