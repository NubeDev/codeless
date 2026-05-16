use serde::{Deserialize, Serialize};

use crate::id::{JobId, StageId};
use crate::time::UnixMillis;

/// Stage lifecycle. Matches `stages.status` wire labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum StageStatus {
    Pending,
    Running,
    AwaitingReview,
    Passed,
    Failed,
}

/// A verify-gated chunk of a job — see SCOPE.md Appendix A `stages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Stage {
    pub id: StageId,
    pub job_id: JobId,
    pub ordinal: u32,
    pub name: String,
    pub status: StageStatus,
    /// `None` when this stage has no verify gate.
    pub verify_cmd: Option<String>,
    pub started_at: Option<UnixMillis>,
    pub ended_at: Option<UnixMillis>,
    /// Runner-supplied session identifier, captured the first time a
    /// task on this stage reports a non-empty `RunResult.session_id`.
    /// Free-form on the wire — Claude emits `sess-<ulid>`, other
    /// runners may use a different shape. `None` until a task
    /// reports one; never cleared once set, and never reused by a
    /// later stage (per SCOPE.md hard rule #1, the stage is the
    /// session boundary). Subsequent tasks within the **same** stage
    /// resume this session via `--continue <session_id>` — that is
    /// what makes pause / resume / cap-bump inside a stage feel like
    /// a continuous Claude Code conversation rather than a fresh
    /// codebase-exploration every time.
    pub session_id: Option<String>,
    /// One-sentence statement of what success for this stage means.
    /// Authored in the per-stage docs; surfaced in the UI overview so
    /// the reader doesn't have to open the stage doc to remember the
    /// intent. `None` for stages authored before the field existed.
    #[serde(default)]
    pub goal: Option<String>,
    /// Acceptance criteria bullets, in author order. The UI renders
    /// each as a tickable line so the human reviewer can match output
    /// against the contract the stage was written to. `None` (not
    /// `Some(vec![])`) for stages authored before the field existed
    /// so a missing list and a deliberately empty list stay
    /// distinguishable.
    #[serde(default)]
    pub acceptance: Option<Vec<String>>,
    /// Wall-clock millis of the last activity observed on this stage's
    /// warm session (turn start, user message, sweeper tick). `None`
    /// when the stage has never had a session captured. Used by the
    /// runtime's idle-timeout sweeper to decide when to archive the
    /// session so a paused/failed stage does not hold a runner
    /// subprocess open forever; see `session_idle` module.
    #[serde(default)]
    pub last_activity_at: Option<UnixMillis>,
    /// `true` once the warm session has been archived (idle timeout
    /// elapsed or explicit `archive_session` RPC). One-way flag: the
    /// next user message against this stage opens a fresh session with
    /// a handover document rather than resuming the recorded
    /// `session_id`. The captured id is preserved on the row for
    /// audit / UI labelling of the archived turn.
    #[serde(default)]
    pub archived: bool,
    /// Persona id this stage runs under. NULL means the stage
    /// inherits the job-level persona (`jobs.persona_id`); a populated
    /// value is the result of the per-stage `persona:` YAML override
    /// resolved at job-submit time (AGENT-DECISIONS.md D1). The column
    /// is recorded on the row so a re-run reproduces the same persona
    /// the stage originally ran under even if the user later edited
    /// the template or the persona row.
    #[serde(default)]
    pub persona_id: Option<String>,
    /// Wall-clock millis the operator (or a future auto-bypass
    /// policy) marked this stage as bypassed. `None` is the
    /// common case; `Some(_)` together with `status: Failed`
    /// means "advance past this stage on the next run." The
    /// status column stays `Failed` so the audit trail keeps
    /// the original outcome; the bypass timestamp is the
    /// forward-advance signal.
    #[serde(default)]
    pub bypassed_at: Option<UnixMillis>,
    /// Operator (or policy) reason for the bypass. Free-text;
    /// rendered in the run log + the UI gate panel so the audit
    /// trail names *why* the bypass happened. `None` when
    /// `bypassed_at` is also `None`.
    #[serde(default)]
    pub bypassed_reason: Option<String>,
}
