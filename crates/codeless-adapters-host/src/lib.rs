//! Host-only adapters: worktree manager, shell, PTY, CLI runners
//! (Claude / Codex / Copilot), REST runners (Anthropic / OpenAI /
//! OpenAI-compat), local filesystem access, secrets-file backend.
//! See the `codeless-adapters-host` row of the crate table in
//! DOCS/SCOPE.md. Process spawning is gated here so mobile builds
//! physically cannot pull it in via Cargo features.

pub mod ai_chat;
pub mod ai_runner_bridge;
pub mod claude;
pub mod editor;
pub mod fs;
pub mod git_changed;
pub mod git_commit;
pub mod git_diff;
pub mod net;
pub mod secrets;
pub mod shell;
pub mod worktree;

pub use ai_chat::{
    parse_cli_runner_id, probe_available_cli_runners, run_chat, AgentChatError, ChatRunCfg,
};
pub use ai_runner_bridge::{
    forward_events, map_event, map_event_with_state, map_todo_write, TodoWriteTracker,
    CLAUDE_TODO_WRITE_TOOL,
};
pub use claude::probe as probe_claude;
pub use editor::{invoke_editor, pick_editor, EditorError};
pub use fs::{FsError, HostFs};
pub use git_changed::{changed_files, GitChangedError};
pub use git_commit::{
    commit_all_changes, commit_paths, find_patch_resolution, git_revert, head_sha, GitCommitError,
    PriorPatchResolution,
};
pub use git_diff::{diff_against, DiffFile, GitDiffError};
#[cfg(feature = "keyring")]
pub use secrets::KeyringSecretBackend;
pub use secrets::{SecretBackend, SecretError, SecretStore, TomlSecretBackend};
pub use shell::{run_shell, ShellOutcome};
pub use worktree::{OnDiskWorktree, WorktreeError, WorktreeHandle, WorktreeManager};
