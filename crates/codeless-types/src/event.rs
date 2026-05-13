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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(transparent)]
#[specta(transparent)]
pub struct EventCursor(pub i64);

/// One row from the `events` table. Variants are tagged by the
/// `events.type` wire label; payload fields are flattened into the
/// JSON object stored in `events.payload`.
///
/// Each variant carries an explicit `#[serde(rename = "...")]` rather
/// than a container-level `rename_all = "kebab-case"`: specta-serde
/// (0.0.10) propagates `rename_all` to variant fields too, which
/// silently kebab-cases `task_id` etc. in generated TypeScript and
/// drifts from serde's actual JSON output (where struct fields stay
/// snake_case unless `rename_all_fields` is also set, and the specta
/// macro does not forward that attribute). Explicit per-variant
/// renames keep the wire label visible at the variant site and dodge
/// the divergence.
///
/// `task-enqueued` carries `depends_on` from day one per SCOPE.md Rule 4:
/// the schema must describe DAG state even while Phase 2 executes
/// linearly, so the wire format does not need a breaking change when
/// topological scheduling lands later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "repo-added")]
    RepoAdded { repo_id: RepoId },
    #[serde(rename = "repo-removed")]
    RepoRemoved { repo_id: RepoId },
    #[serde(rename = "repo-updated")]
    RepoUpdated { repo_id: RepoId },

    #[serde(rename = "job-queued")]
    JobQueued { job_id: JobId, repo_id: RepoId },
    #[serde(rename = "job-promoted")]
    JobPromoted { job_id: JobId },
    #[serde(rename = "job-started")]
    JobStarted { job_id: JobId },
    #[serde(rename = "job-completed")]
    JobCompleted { job_id: JobId },
    #[serde(rename = "job-stopped")]
    JobStopped { job_id: JobId, reason: StopReason },
    #[serde(rename = "job-failed")]
    JobFailed { job_id: JobId },
    /// Job moved from `Running` (or `AwaitingReview`) to `Paused`.
    /// Distinct from `JobStopped`: a paused row is *expected* to
    /// be resumed via `resume_job`; the captured per-stage
    /// `Stage.session_id` is the resume handle. Emitted by the
    /// cap-watcher when a cost/wall-clock cap trips on a stage
    /// that has a captured session id (resumable) and by the
    /// `pause_job` RPC. The `reason` distinguishes user intent
    /// from cap-tripped pauses so dashboards and chat dividers
    /// can render the right copy.
    #[serde(rename = "job-paused")]
    JobPaused { job_id: JobId, reason: StopReason },
    /// User-initiated re-queue of a terminal-but-recoverable job
    /// (A0 — intra-stage session continuation). The job's branch,
    /// worktree, and captured per-stage `Stage.session_id` survive;
    /// the next claude task passes the session id as
    /// `CliCfg::resume_id` so the agent continues the same
    /// conversation rather than re-deriving. Distinct from
    /// `JobPromoted` (which is for `Draft -> Queued`) and `JobQueued`
    /// (which is for fresh submit) so the dashboard and chat can
    /// render a "resumed" divider rather than treating it as a new
    /// run.
    #[serde(rename = "job-resumed")]
    JobResumed {
        job_id: JobId,
        /// The reason the job had stopped before the resume. `None`
        /// when the row had no `stop_reason` recorded (a fresh
        /// resume of a never-stopped row is a programming error;
        /// the RPC enforces it).
        #[serde(default)]
        previous_reason: Option<StopReason>,
    },

    #[serde(rename = "stage-started")]
    StageStarted {
        stage_id: StageId,
        job_id: JobId,
        /// 0-based position of this stage in the template's `stages:`
        /// list. Carried on the wire so subscribers can persist
        /// `Stage` rows without re-parsing the YAML.
        #[serde(default)]
        ordinal: u32,
        /// Title of the stage (the YAML's `stages:` entry, with any
        /// `REVIEW ` prefix preserved). Same source of truth the UI
        /// already uses; persisted on the `Stage` row.
        #[serde(default)]
        name: String,
    },
    #[serde(rename = "verify-started")]
    VerifyStarted { stage_id: StageId },
    #[serde(rename = "verify-passed")]
    VerifyPassed { stage_id: StageId },
    #[serde(rename = "verify-failed")]
    VerifyFailed { stage_id: StageId, exit_code: i32 },
    #[serde(rename = "stage-completed")]
    StageCompleted {
        stage_id: StageId,
        status: StageStatus,
    },
    /// First-and-only-time capture of the runner-supplied session id
    /// for this stage. Emitted by `StageRecorder` the first time a
    /// task on the stage reports a non-empty `RunResult.session_id`;
    /// subsequent tasks with a session id on the same stage do not
    /// re-emit. The recorder pins the same value onto
    /// `stages.session_id` in SQLite so the observation survives
    /// session boundaries.
    #[serde(rename = "stage-session-captured")]
    StageSessionCaptured {
        stage_id: StageId,
        session_id: String,
    },

    #[serde(rename = "task-enqueued")]
    TaskEnqueued {
        task_id: TaskId,
        stage_id: StageId,
        depends_on: Vec<TaskId>,
    },
    #[serde(rename = "task-started")]
    TaskStarted { task_id: TaskId },
    #[serde(rename = "tool-call")]
    ToolCall {
        task_id: TaskId,
        tool: String,
        args_json: String,
    },
    #[serde(rename = "tool-approval-requested")]
    ToolApprovalRequested {
        task_id: TaskId,
        tool: String,
        args_json: String,
    },
    #[serde(rename = "ai-token")]
    AiToken { task_id: TaskId, delta: String },
    #[serde(rename = "ai-message-complete")]
    AiMessageComplete {
        task_id: TaskId,
        input_tokens: i64,
        output_tokens: i64,
        cost_cents: CostCents,
    },
    #[serde(rename = "task-completed")]
    TaskCompleted { task_id: TaskId, status: TaskStatus },

    #[serde(rename = "review-requested")]
    ReviewRequested {
        review_id: ReviewId,
        stage_id: StageId,
    },
    #[serde(rename = "review-approved")]
    ReviewApproved { review_id: ReviewId },
    #[serde(rename = "review-commented")]
    ReviewCommented {
        review_id: ReviewId,
        comment: String,
    },
    #[serde(rename = "review-stopped")]
    ReviewStopped { review_id: ReviewId },
}

/// Envelope written to the `events` table. The `cursor`, `created_at`,
/// and foreign-key columns are recorded by the runtime; the inner
/// `Event` is the JSON payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EventEnvelope {
    pub cursor: EventCursor,
    pub job_id: Option<JobId>,
    pub stage_id: Option<StageId>,
    pub task_id: Option<TaskId>,
    pub created_at: UnixMillis,
    pub event: Event,
}
