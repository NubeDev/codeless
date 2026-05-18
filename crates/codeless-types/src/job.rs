use serde::{Deserialize, Serialize};

use crate::auto_bypass::AutoBypassPolicy;
use crate::id::{JobId, RepoId};
use crate::money::CostCents;
use crate::pause_point::PausePointId;
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
    /// Surface F thrashing guard: the per-job `AutoBypassPolicy` fired
    /// twice in a row with no `Passed` stage between, so the runtime
    /// halts the job instead of auto-bypassing a third time. The
    /// guidance comment thread converged on nothing in two attempts;
    /// a third would burn more tokens for the same outcome. See
    /// `DOCS/AUTO-BYPASS-DECISIONS.md` Q1 — two-strikes is the
    /// canonical window size, written here as the wire-level stop
    /// reason so the UI can render `policy thrashing` distinctly from
    /// the other terminal causes.
    AutoBypassThrashing,
    /// A REVIEW stage's structural diff-verify pre-check rejected the
    /// prior stage's handover before the model ran. Distinct from
    /// `RunnerCrash` because the runner never executed: a plain
    /// resume re-runs the same deterministic pre-check against the
    /// same handover and fails identically. The recovery path is the
    /// explicit `override_pre_check_and_resume` RPC, which requires
    /// the operator to acknowledge the gap and supply a comment that
    /// threads into the stage prompt; the override is a one-shot
    /// audit-logged opt-in, not a sticky flag.
    ReviewPreCheck,
    /// A host-side infrastructure failure (SQLite `SQLITE_FULL` /
    /// `SQLITE_IOERR` / `SQLITE_CORRUPT` / `SQLITE_CANTOPEN` /
    /// `SQLITE_READONLY`) terminated the stage. Distinct from
    /// `RunnerCrash` because retrying the same SQL on the same disk
    /// is guaranteed not to help — the operator has to fix the host
    /// (free disk, repair the file, restore writability) before the
    /// job can advance. The runtime's auto-bypass policy never
    /// silently retries an infrastructure error; the UI labels this
    /// halt as `Infrastructure failure` so the operator sees the
    /// host condition rather than a generic crash chip. See
    /// `DOCS/AUTO-BYPASS-DECISIONS.md` Q1.
    Infrastructure,
    /// A pre-declared scope-level pause point (operator-authored in
    /// `template.yaml`) fired. Distinct from `User` so the chat and
    /// dashboard can render a "planned pause" divider instead of a
    /// runtime-pause chip, and the divider lookup can carry the
    /// `point_id` back to the `scheduled_pause_points` row for
    /// `reason` text. Resumes through the existing `resume_job` RPC;
    /// cost-cap behaviour is identical to `User` per SCOPED-PAUSE-
    /// POINTS §5 Q4.
    ScopedPausePoint {
        point_id: PausePointId,
    },
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
    /// Optional persona-derived system prompt composed at submit time.
    /// When set, the runner factory uses this as the agent's system
    /// prompt for every stage of the job, overriding the server's
    /// default. `None` keeps the server-configured default in place so
    /// jobs submitted without a persona run unchanged. Stored on the
    /// row so a reboot, resume, or rerun reproduces the same prompt;
    /// `rerun_job` carries the value forward verbatim.
    pub system_prompt: Option<String>,
    /// Persona the user picked at submit time. The composed prompt
    /// rides on `system_prompt`; this column preserves the lookup key
    /// so a rerun can reproduce the same agent posture even if the
    /// persona's body is edited later. `None` means the user submitted
    /// without picking a persona — the server default applies and a
    /// rerun keeps that posture. Free TEXT until personas move to a
    /// server-side table; the FK lands in a later stage.
    pub persona_id: Option<String>,
    /// Per-job auto-bypass policy. `None` (the default) preserves the
    /// existing behaviour: a stage failure under any non-cap reason
    /// halts the job and waits for operator triage. `Some(policy)`
    /// pre-authorises the runtime to mark the failed stage
    /// `Failed`-with-bypass and advance, threading the policy's
    /// canned (or operator-supplied) guidance into the next stage's
    /// prompt. Cap breaches (`CostCap`, `WallClock`) ignore the
    /// policy and halt regardless — see
    /// `DOCS/AUTO-BYPASS-DECISIONS.md` Q2.
    pub auto_bypass_policy: Option<AutoBypassPolicy>,
    pub pending_operator_comment: Option<String>,
    /// One-shot flag set by `override_pre_check_and_resume`: the
    /// operator has acknowledged that the REVIEW stage's diff-verify
    /// pre-check will fail against the existing handover and wants
    /// the stage to run anyway. Consumed atomically by the runner on
    /// the first re-entry into the REVIEW stage so a subsequent run
    /// (or a later REVIEW stage) does not silently inherit the
    /// override. `false` is the default and the value on every fresh
    /// row.
    #[serde(default)]
    pub precheck_override_once: bool,
    pub started_at: Option<UnixMillis>,
    pub ended_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
}
