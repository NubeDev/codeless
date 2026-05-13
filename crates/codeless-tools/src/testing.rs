//! Test harness for ported and native tools.
//!
//! `fake_ctx` builds a `ToolCtx` from a tempdir worktree, an
//! in-memory allowlist, and a controllable cancellation token —
//! enough surface to drive a `Tool::call` from a unit or
//! integration test without booting MCP, a runner subprocess, or
//! the codeless server.

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tracing::Span;

use crate::ctx::ToolCtx;
use crate::policy::{AllowlistFile, NetworkMode};

/// A `ToolCtx` plus the tempdir + cancel token that back it.
///
/// The tempdir is held here so the worktree path stays valid for
/// the call's duration; dropping `FakeCtx` cleans up the tempdir.
pub struct FakeCtx {
    pub ctx: ToolCtx,
    pub cancel: CancellationToken,
    _worktree: TempDir,
}

/// Build a default fake context: empty worktree, `NetworkMode::None`,
/// empty allowlist, fresh cancellation token. Tests that need
/// something different reach for `fake_ctx_builder`.
pub fn fake_ctx() -> FakeCtx {
    fake_ctx_builder().build()
}

pub fn fake_ctx_builder() -> FakeCtxBuilder {
    FakeCtxBuilder::default()
}

#[derive(Default)]
pub struct FakeCtxBuilder {
    network_mode: Option<NetworkMode>,
    allowlist: Option<AllowlistFile>,
}

impl FakeCtxBuilder {
    pub fn network_mode(mut self, mode: NetworkMode) -> Self {
        self.network_mode = Some(mode);
        self
    }

    pub fn allowlist(mut self, list: AllowlistFile) -> Self {
        self.allowlist = Some(list);
        self
    }

    pub fn build(self) -> FakeCtx {
        let worktree = TempDir::new().expect("tempdir creation");
        let cancel = CancellationToken::new();
        let ctx = ToolCtx::new(
            worktree.path(),
            self.network_mode.unwrap_or_default(),
            self.allowlist.unwrap_or_default(),
            cancel.clone(),
            Span::current(),
        );
        FakeCtx {
            ctx,
            cancel,
            _worktree: worktree,
        }
    }
}
