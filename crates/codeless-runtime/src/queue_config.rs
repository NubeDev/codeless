/// Concurrency caps applied by `SqliteStore::lease_next`. `None` on
/// any field means "no limit"; numbers mean "do not claim a fresh
/// task when this many are already in flight at this scope".
///
/// SCOPE.md "Per-runner concurrency cap": Claude Code is RAM-hungry,
/// so limiting how many `ClaudeRunner` sessions run at once is the
/// safety knob that keeps a multi-job runtime from OOMing the host.
/// `max_per_repo` follows the same logic for disk: each worktree
/// keeps its own `node_modules/` / `target/`; the cap bounds the
/// dominant disk cost. `max_global` is the umbrella over both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueConfig {
    pub max_global: Option<u32>,
    pub max_per_repo: Option<u32>,
    pub max_per_runner: Option<u32>,
}

impl QueueConfig {
    pub const fn unlimited() -> Self {
        Self {
            max_global: None,
            max_per_repo: None,
            max_per_runner: None,
        }
    }
}
