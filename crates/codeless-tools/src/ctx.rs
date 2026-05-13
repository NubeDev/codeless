use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;
use tracing::Span;

use crate::policy::{AllowlistFile, NetworkMode};

/// Per-call context a tool receives.
///
/// Borrowed for the call's duration — tools do not own it.
/// `codeless-mcp` constructs one per tool invocation in its dispatch
/// path and drops it when the call returns.
///
/// `mcp_session` is intentionally absent from this first cut: no
/// ported tool needs it yet, and a placeholder field would freeze a
/// type we don't yet know the shape of. Added when the first tool
/// that calls back into the runner lands.
pub struct ToolCtx {
    worktree_root: PathBuf,
    network_mode: NetworkMode,
    allowlist: AllowlistFile,
    cancel: CancellationToken,
    span: Span,
}

impl ToolCtx {
    pub fn new(
        worktree_root: impl Into<PathBuf>,
        network_mode: NetworkMode,
        allowlist: AllowlistFile,
        cancel: CancellationToken,
        span: Span,
    ) -> Self {
        Self {
            worktree_root: worktree_root.into(),
            network_mode,
            allowlist,
            cancel,
            span,
        }
    }

    pub fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    pub fn network_mode(&self) -> &NetworkMode {
        &self.network_mode
    }

    pub fn allowlist(&self) -> &AllowlistFile {
        &self.allowlist
    }

    /// Returns true once the runner has signalled cancellation.
    /// Tools poll this at await points and return
    /// `ToolError::Cancelled` to give the dispatcher a structured
    /// signal rather than a dropped future.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    pub fn span(&self) -> &Span {
        &self.span
    }
}
