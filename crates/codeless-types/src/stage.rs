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
}
