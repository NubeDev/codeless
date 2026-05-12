//! Host-only adapters: worktree manager, shell, PTY, CLI runners
//! (Claude / Codex / Copilot), REST runners (Anthropic / OpenAI /
//! OpenAI-compat), local filesystem access, secrets-file backend.
//! See the `codeless-adapters-host` row of the crate table in
//! DOCS/SCOPE.md. Process spawning is gated here so mobile builds
//! physically cannot pull it in via Cargo features.

pub mod secrets;
pub mod worktree;

pub use secrets::{SecretError, SecretStore};
pub use worktree::{WorktreeError, WorktreeHandle, WorktreeManager};
