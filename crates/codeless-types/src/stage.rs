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
    /// reports one; never cleared once set. Persisted for
    /// observability only — see SCOPE.md hard rule #1: codeless
    /// never reuses this to resume a runner.
    pub session_id: Option<String>,
}
