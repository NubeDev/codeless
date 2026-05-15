use serde::{Deserialize, Serialize};

use crate::id::{JobId, RepoId, ReviewId, StageId, TaskId};
use crate::job::StopReason;
use crate::money::CostCents;
use crate::scope_patch::{ScopePatchId, ScopePatchKind, ScopePatchTarget};
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
    /// One layered verify gate started running. Paired with
    /// `verify-step-passed`, `verify-step-failed`, or
    /// `verify-step-skipped`. The outer `verify-started` /
    /// `verify-passed` / `verify-failed` envelopes still bracket the
    /// stage's whole verify run; the per-step events let the UI render
    /// a glyph per gate (`○ → ● → ✓` or `!`) instead of a single bit.
    #[serde(rename = "verify-step-started")]
    VerifyStepStarted {
        stage_id: StageId,
        /// 0-based index into the stage's `verify:` list. The
        /// list ordering is the contract; the recorder pins
        /// per-step rows by `(stage_id, step_index)`.
        step_index: u32,
        name: String,
    },
    #[serde(rename = "verify-step-passed")]
    VerifyStepPassed {
        stage_id: StageId,
        step_index: u32,
        name: String,
        /// Wall-clock duration of the step's shell invocation, in
        /// milliseconds. Surfaced in the UI's per-gate row so a slow
        /// gate is visible without opening the log.
        duration_ms: u64,
    },
    #[serde(rename = "verify-step-failed")]
    VerifyStepFailed {
        stage_id: StageId,
        step_index: u32,
        name: String,
        exit_code: i32,
        /// Last ~16 lines of merged stdout+stderr from the step. Kept
        /// short on the wire so the UI doesn't have to fetch a separate
        /// log blob to render the failure preview.
        tail: String,
    },
    /// A verify step that did not run because a prior step in the same
    /// stage already failed. Emitted (rather than silently omitted) so
    /// the UI can render a `-` or grey-out glyph instead of leaving the
    /// row blank, per SCOPE.md operator-visibility hard rule.
    #[serde(rename = "verify-step-skipped")]
    VerifyStepSkipped {
        stage_id: StageId,
        step_index: u32,
        name: String,
        /// Today the only reason is `"prior-gate-red"`; carried as a
        /// string so future reasons (e.g. timeout, cancelled) can land
        /// without a wire-format change.
        reason: String,
    },
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
    /// The stage's warm session was archived (idle timeout elapsed) and
    /// the next user message against the stage transparently opened a
    /// fresh session preceded by a handover document. Emitted exactly
    /// once per session boundary so the UI can render an inline divider
    /// (`prior session archived; resumed with handover`) without
    /// polling for state changes. `prior_session_id` is the value that
    /// was on `stages.session_id` at archive time; the new session's
    /// id arrives later via `StageSessionCaptured`.
    #[serde(rename = "session-archived-then-resumed")]
    SessionArchivedThenResumed {
        stage_id: StageId,
        prior_session_id: String,
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

    /// `template.yaml` for the job changed in SQLite. Emitted by
    /// `update_job_template` (user edits from the Spec pane) and by
    /// `resync_template_from_disk` at the head of `start_job` /
    /// `resume_job` when a chat-driven filesystem edit lands in the
    /// DB. Subscribers — the Spec pane, the chat's footer banner —
    /// use this as a refetch signal; the new YAML is not on the
    /// wire because the Spec pane already fetches it through
    /// `read_job_file` / `get_job`.
    #[serde(rename = "job-template-updated")]
    JobTemplateUpdated { job_id: JobId },
    /// An attached workspace's canonical root is no longer reachable
    /// (the directory was deleted, the volume unmounted, the symlink
    /// target vanished, etc). Emitted exactly once per transition by the
    /// 30s liveness sweep in `codeless-runtime::workspace_liveness`; the
    /// follow-up `WorkspaceRecovered` event fires when the path comes
    /// back. The UI uses the pair to flip a per-workspace badge in the
    /// sidebar without polling.
    ///
    /// `fs_root` is the canonical path as registered in
    /// `attached_workspaces.fs_root_canonical`. `reason` is a short
    /// machine-readable tag (`"missing"`, `"not-a-directory"`,
    /// `"io-error"`) — string-typed rather than a wire enum so a new
    /// failure mode can land without an enum-bump on every shell.
    #[serde(rename = "workspace-unhealthy")]
    WorkspaceUnhealthy {
        repo_id: RepoId,
        fs_root: String,
        reason: String,
    },
    /// Paired with `WorkspaceUnhealthy`. Fires the next sweep after the
    /// path resolves to a directory again.
    #[serde(rename = "workspace-recovered")]
    WorkspaceRecovered { repo_id: RepoId, fs_root: String },

    /// A supporting file under `.codeless/jobs/<name>/` was written
    /// or deleted. `filename` is the basename (e.g. `SCOPE.md`),
    /// not a full path, so subscribers don't have to know about
    /// repo layout. Emitted by `write_job_file` and `delete_job_file`.
    /// Chat-driven raw-`Edit`/`Write` edits do not emit this — the
    /// agent is told disk is the source of truth and the Spec pane
    /// refreshes on the next `JobTemplateUpdated` (run-time resync)
    /// or on user navigation.
    #[serde(rename = "job-file-updated")]
    JobFileUpdated { job_id: JobId, filename: String },

    /// A REVIEW stage proposed a rulebook patch (Step 4 of the
    /// SESSION-MUTABLE-SCOPE ramp, shadow mode). The full proposal
    /// body lives in `DOCS/SCOPE-PROPOSED.md`; this event carries
    /// identifiers and discriminants only so consumers can answer
    /// the kill-criterion query ("how many proposals in the last K
    /// REVIEW stages, how many had a predicate, how many landed")
    /// without re-reading the file. Decisions Q7 records why this
    /// is an event rather than a row in a new SQLite table (R4 —
    /// SQLite is source of truth, no new persistence store).
    ///
    /// `evidence_stage_id` is `Some` only for `Loosen` and `None`
    /// for `Tighten`; `has_predicate` is the dual signal for
    /// `Tighten`. Carrying both as separate fields (rather than
    /// merging into a single variant per kind) keeps the SSE wire
    /// label single — one envelope shape per event type — which
    /// the existing UI subscriber assumes.
    #[serde(rename = "scope-patch-proposed")]
    ScopePatchProposed {
        stage_id: StageId,
        review_id: ReviewId,
        patch_id: ScopePatchId,
        kind: ScopePatchKind,
        target: ScopePatchTarget,
        /// Repo-relative path of the file the proposal targets.
        /// Carried on the wire so subscribers can render the path
        /// without fetching the proposal body.
        target_path: String,
        evidence_stage_id: Option<StageId>,
        has_predicate: bool,
    },
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
