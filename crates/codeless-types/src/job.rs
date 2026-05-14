use serde::{Deserialize, Serialize};

use crate::id::{JobId, RepoId};
use crate::money::CostCents;
use crate::time::UnixMillis;

/// Lifecycle states for a job row. String form matches the
/// `jobs.status` column wire labels in SCOPE.md Appendix A.
///
/// `Draft` is the landing state when a job is submitted with
/// `start_immediately = false` — the row exists, the user can edit
/// the spec / docs / handover, but the driver does **not** pick it
/// up. The user calls `start_job` (or submits with
/// `start_immediately = true`) to promote `Draft → Queued`. From
/// there the lifecycle is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum JobStatus {
    Draft,
    Queued,
    Running,
    AwaitingReview,
    Completed,
    Failed,
    Stopped,
    /// User has paused, or a cap tripped on a resumable stage —
    /// the agent's captured `Stage.session_id` is the resume
    /// handle. Distinct from `Stopped`: a paused row is *expected*
    /// to be resumed; a stopped row is the user saying "I'm done."
    /// `resume_job` accepts both. See SCOPE.md hard rule #1
    /// (the stage is the session boundary; within a stage the
    /// runner session is continuous).
    Paused,
}

/// Where the agent's edits land. See SCOPE.md "Workspace mode".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMode {
    /// Edits land in the user's existing local clone on a fresh branch.
    #[default]
    InRepo,
    /// Edits land in a separate `git worktree add` checkout.
    Worktree,
}

/// Why a job left the running set early. `None` while running or after a
/// clean completion; populated when status is `Stopped` or `Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum StopReason {
    User,
    CostCap,
    WallClock,
    RunnerCrash,
}

/// One unit of work the user kicked off — see SCOPE.md Appendix A `jobs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Job {
    pub id: JobId,
    pub repo_id: RepoId,
    pub status: JobStatus,
    pub stop_reason: Option<StopReason>,
    pub template_yaml: Option<String>,
    pub prompt: Option<String>,
    /// Runner kind chosen at submit time (e.g. `"claude"`, `"anthropic"`).
    pub runner: String,
    pub branch: String,
    /// `in_repo` (default): agent edits the user's local clone.
    /// `worktree`: agent edits a separate `git worktree add` checkout.
    pub workspace_mode: WorkspaceMode,
    /// `None` until the worktree has been provisioned. Preserved across
    /// crashes so a reaper can clean up after a dead leaseholder.
    /// Always `None` in `in_repo` mode (edits land in the repo itself).
    pub worktree_path: Option<String>,
    pub cost_cap_cents: CostCents,
    pub wall_clock_cap_ms: i64,
    pub cost_cents: CostCents,
    /// Optional per-job model override forwarded to the runner. `None`
    /// uses the runner adapter's default. Free-form because each runner
    /// has its own model catalogue (Claude opus/sonnet/haiku, Copilot
    /// gpt-5.x, etc.) — validation is the adapter's job, not this layer.
    pub model: Option<String>,
    /// Optional per-job permission mode. Only meaningful for runners
    /// that expose a per-call permission gate (Claude). Wire labels
    /// match the snake_case form on `claude-wrapper`'s `PermissionMode`:
    /// `default | accept_edits | plan | bypass`. `None` leaves the
    /// adapter's headless default (`Bypass` for Claude) in place.
    pub permission_mode: Option<String>,
    /// Optional thinking-budget hint. Only meaningful for runners that
    /// honour it (Claude). Wire labels: `low | medium | high`. `None`
    /// disables the prompt-trigger prefix that maps to claude's
    /// "think" / "think hard" / "ultrathink" cues.
    pub effort: Option<String>,
    pub started_at: Option<UnixMillis>,
    pub ended_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
}
