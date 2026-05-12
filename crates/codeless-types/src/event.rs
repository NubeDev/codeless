use serde::{Deserialize, Serialize};

use crate::id::{JobId, RepoId, ReviewId, StageId, TaskId};
use crate::job::StopReason;
use crate::money::CostCents;
use crate::stage::StageStatus;
use crate::task::TaskStatus;
use crate::time::UnixMillis;

/// Monotonic event index, allocated by `events.cursor INTEGER
/// AUTOINCREMENT`. Doubles as `Last-Event-ID` over SSE (SCOPE.md
/// "Catch-up cursor.") — clients send the last seen cursor on reconnect
/// and the server replays from that point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventCursor(pub i64);

/// One row from the `events` table. Variants are tagged by the
/// `events.type` wire label; payload fields are flattened into the
/// JSON object stored in `events.payload`.
///
/// `task-enqueued` carries `depends_on` from day one per SCOPE.md Rule 4:
/// the schema must describe DAG state even while Phase 2 executes
/// linearly, so the wire format does not need a breaking change when
/// topological scheduling lands later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    RepoAdded {
        repo_id: RepoId,
    },
    RepoRemoved {
        repo_id: RepoId,
    },
    RepoUpdated {
        repo_id: RepoId,
    },

    JobQueued {
        job_id: JobId,
        repo_id: RepoId,
    },
    JobPromoted {
        job_id: JobId,
    },
    JobStarted {
        job_id: JobId,
    },
    JobCompleted {
        job_id: JobId,
    },
    JobStopped {
        job_id: JobId,
        reason: StopReason,
    },
    JobFailed {
        job_id: JobId,
    },

    StageStarted {
        stage_id: StageId,
        job_id: JobId,
    },
    VerifyStarted {
        stage_id: StageId,
    },
    VerifyPassed {
        stage_id: StageId,
    },
    VerifyFailed {
        stage_id: StageId,
        exit_code: i32,
    },
    StageCompleted {
        stage_id: StageId,
        status: StageStatus,
    },

    TaskEnqueued {
        task_id: TaskId,
        stage_id: StageId,
        depends_on: Vec<TaskId>,
    },
    TaskStarted {
        task_id: TaskId,
    },
    ToolCall {
        task_id: TaskId,
        tool: String,
        args_json: String,
    },
    ToolApprovalRequested {
        task_id: TaskId,
        tool: String,
        args_json: String,
    },
    AiToken {
        task_id: TaskId,
        delta: String,
    },
    AiMessageComplete {
        task_id: TaskId,
        input_tokens: i64,
        output_tokens: i64,
        cost_cents: CostCents,
    },
    TaskCompleted {
        task_id: TaskId,
        status: TaskStatus,
    },

    ReviewRequested {
        review_id: ReviewId,
        stage_id: StageId,
    },
    ReviewApproved {
        review_id: ReviewId,
    },
    ReviewCommented {
        review_id: ReviewId,
        comment: String,
    },
    ReviewStopped {
        review_id: ReviewId,
    },
}

/// Envelope written to the `events` table. The `cursor`, `created_at`,
/// and foreign-key columns are recorded by the runtime; the inner
/// `Event` is the JSON payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub cursor: EventCursor,
    pub job_id: Option<JobId>,
    pub stage_id: Option<StageId>,
    pub task_id: Option<TaskId>,
    pub created_at: UnixMillis,
    pub event: Event,
}
