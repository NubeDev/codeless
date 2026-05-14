//! Host-only adapters: worktree manager, shell, PTY, CLI runners
//! (Claude / Codex / Copilot), REST runners (Anthropic / OpenAI /
//! OpenAI-compat), local filesystem access, secrets-file backend.
//! See the `codeless-adapters-host` row of the crate table in
//! DOCS/SCOPE.md. Process spawning is gated here so mobile builds
//! physically cannot pull it in via Cargo features.

pub mod ai_chat;
pub mod ai_runner_bridge;
pub mod claude;
pub mod fs;
pub mod git_commit;
pub mod git_diff;
pub mod secrets;
pub mod worktree;

pub use ai_chat::{
    AgentChatError, ChatRunCfg, parse_cli_runner_id, probe_available_cli_runners, run_chat,
};
pub use ai_runner_bridge::{forward_events, map_event};
pub use claude::probe as probe_claude;
pub use fs::{FsError, HostFs};
pub use git_commit::{GitCommitError, commit_paths};
pub use git_diff::{DiffFile, GitDiffError, diff_against};
pub use secrets::{SecretError, SecretStore};
pub use worktree::{OnDiskWorktree, WorktreeError, WorktreeHandle, WorktreeManager};
