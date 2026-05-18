use serde::{Deserialize, Serialize};

use crate::id::{AssistantThreadId, JobId, RepoId, ReviewId, StageId, TaskId, TodoId};
use crate::job::StopReason;
use crate::money::CostCents;
use crate::review_gate::{PreCheckOutcome, ReviewVerdict};
use crate::scope_patch::{ScopePatchId, ScopePatchKind, ScopePatchTarget};
use crate::stage::{FailureClass, StageStatus};
use crate::task::TaskStatus;
use crate::time::UnixMillis;
use crate::todo::{TodoKind, TodoStatus};

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
        /// Free-text identifier for the surface that initiated the
        /// resume. The first consumer is the Slack control plane,
        /// which sets this to `"slack"` (vs. `"operator"` for direct
        /// UI/CLI/RPC, `"assistant"` for the assistant-tool path,
        /// etc.) so audit and dashboard surfaces can distinguish a
        /// phone-driven resume from a keyboard-driven one. `None`
        /// preserves the historical event shape; older replayed
        /// events deserialize unchanged.
        #[serde(default)]
        actor: Option<String>,
    },

    /// `reset_job` returned a stuck job (`Queued` whose driver kept
    /// failing, or a terminal `Failed` / `Stopped`) to an editable
    /// `Draft`. The captured worktree was reaped (best-effort) and
    /// `worktree_path` / `stop_reason` / `ended_at` were cleared.
    /// Distinct from `JobQueued` and `JobPromoted` so the dashboard
    /// can render a "reset to draft" divider rather than treating the
    /// row as a new submission. `previous_status` is the state the
    /// row held immediately before the reset so subscribers can
    /// distinguish a driver-give-up recovery from a manual rewind of
    /// a completed-but-failed run.
    #[serde(rename = "job-reset")]
    JobReset {
        job_id: JobId,
        previous_status: crate::job::JobStatus,
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
        /// Per-stage persona override resolved at job-submit
        /// (D1, D5). `None` means the stage inherits the job-level
        /// persona; the StageRecorder writes this verbatim onto
        /// `stages.persona_id` so a per-stage handover and a re-run
        /// reproduce the same binding. Defaults to `None` so older
        /// events from the persisted bus replay still decode.
        #[serde(default)]
        persona_id: Option<String>,
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
        /// Coarse classification when `status = Failed`. The
        /// StageRecorder writes this onto `stages.failure_class`
        /// in the same SQL update as `status`. `None` for
        /// `Passed` and for replayed events from before the field
        /// existed (which decode unchanged via `serde(default)`).
        #[serde(default)]
        failure_class: Option<FailureClass>,
        /// Short human-readable failure description paired with
        /// `failure_class`. Truncated to ~200 chars at the emit
        /// site. `None` for `Passed` and for legacy events.
        #[serde(default)]
        failure_detail: Option<String>,
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

    /// A new sub-step appeared on a running task — either the runner
    /// emitted a `TodoWrite`-equivalent tool call (`kind = Runner`) or
    /// the runtime injected one (`kind` is one of `Checks` / `Docs` /
    /// `Git` for the closing trio, or `Planner` for the future
    /// stage-level pre-declared path). See `DOCS/JOB-UI.md` "Todo rows
    /// (nested under a tick)" for the UI contract — the row's glyph
    /// flips `○ → ● → ✓` as `TodoUpdated` / `TodoCompleted` arrive.
    #[serde(rename = "todo-added")]
    TodoAdded {
        todo_id: TodoId,
        task_id: TaskId,
        ordinal: u32,
        title: String,
        kind: TodoKind,
    },
    /// Status transition on an existing todo. Carried as a separate
    /// event from `TodoCompleted` so an intermediate `Pending →
    /// InProgress` flip can light up the `●` glyph without faking a
    /// completion. `TodoCompleted` is the dedicated terminal event so
    /// the stage-completion gate (the part that holds back
    /// `StageCompleted` until the trio is resolved) can subscribe to
    /// one event type instead of inspecting every `TodoUpdated`.
    #[serde(rename = "todo-updated")]
    TodoUpdated { todo_id: TodoId, status: TodoStatus },
    /// Terminal transition: `Done`, `Skipped`, or `Failed`. Pairs with
    /// `TodoUpdated` — the runtime emits `TodoUpdated` for non-terminal
    /// flips and `TodoCompleted` for terminal ones, so subscribers can
    /// pick the granularity they care about.
    #[serde(rename = "todo-completed")]
    TodoCompleted { todo_id: TodoId, status: TodoStatus },

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
    /// Layer-1 diff-verify pre-check result for a REVIEW stage —
    /// the first half of Surface A's gate diagnostics. Emitted by
    /// `template_runner` alongside the existing `tracing::info!` /
    /// `tracing::warn!` lines that report the same outcome; the
    /// logs stay for operator debugging, the event carries the
    /// structured form the UI panel renders. `Pass` and `Fail`
    /// carry the resolved path lists rather than a boolean so the
    /// panel can name the exact set; `Skipped` and `NothingToVerify`
    /// are distinct on the wire so the panel can tell a setup gap
    /// apart from a clean-baseline handover (see
    /// `review_gate::PreCheckOutcome` for the variant contract).
    #[serde(rename = "review-pre-check")]
    ReviewPreCheck {
        stage_id: StageId,
        outcome: PreCheckOutcome,
    },
    /// Final REVIEW-gate verdict, including the runtime's own
    /// `AutoFail` cases (pre-check rejected, sentinel unparseable,
    /// scope-patch validation failed). Emitted exactly once per
    /// REVIEW stage that reaches a terminal verdict; the
    /// matching `tracing` call on `template_runner` keeps the
    /// human-readable log line. Pairs with `ReviewPreCheck` —
    /// when the pre-check auto-fails, both events fire (pre-check
    /// with `Fail`, verdict with `AutoFail`) so the panel sees
    /// the same two-phase shape it does for the model-driven path.
    #[serde(rename = "review-verdict")]
    ReviewVerdict {
        stage_id: StageId,
        verdict: ReviewVerdict,
    },

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

    /// Operator approved a proposed scope patch through the UI. Emitted
    /// after the approval commit lands so cross-window subscribers can
    /// invalidate their patch-inbox caches and link to the resulting
    /// commit without a follow-up RPC. Same shape considerations as
    /// `ScopePatchProposed` — mobile-safe, no enrichment that requires
    /// host-only types.
    #[serde(rename = "scope-patch-approved")]
    ScopePatchApproved {
        stage_id: StageId,
        review_id: ReviewId,
        patch_id: ScopePatchId,
        kind: ScopePatchKind,
        target: ScopePatchTarget,
        target_path: String,
        /// SHA of the human-authored approval commit produced by
        /// `approve_scope_patch`. The UI links the inbox row to a
        /// `commit/<sha>` route off this field.
        commit_sha: String,
    },

    /// Operator rejected a proposed scope patch through the UI. Pairs
    /// with `ScopePatchApproved`; same cross-window invalidation use
    /// case.
    #[serde(rename = "scope-patch-rejected")]
    ScopePatchRejected {
        stage_id: StageId,
        review_id: ReviewId,
        patch_id: ScopePatchId,
        kind: ScopePatchKind,
        target: ScopePatchTarget,
        target_path: String,
        commit_sha: String,
    },

    /// A stage failed under a job-level `AutoBypassPolicy` and the
    /// runtime auto-bypassed the failure to keep the job moving
    /// (Surface F, `DOCS/AUTO-BYPASS-DECISIONS.md` Q1/Q4). Emitted
    /// by the stage-failed handler in the same commit that writes
    /// `stages.bypassed_at` / `stages.bypassed_reason`; the
    /// corresponding `tracing` line stays for operator debugging
    /// while this event carries the structured form the UI gate
    /// panel reads. Deliberately a distinct variant from
    /// Surface E's operator-clicked `StageBypassed` so the panel
    /// can render `bypassed by policy: <name>` vs
    /// `bypassed by operator` without inspecting the reason
    /// string (Q6 "Event reuse").
    ///
    /// `policy_name` is the stable name returned by
    /// `AutoBypassPolicy::policy_name()` — one of the five preset
    /// labels or the literal `"Custom"`. `comment_used` is the
    /// canned (or operator-supplied) guidance string that was
    /// threaded into the next stage's prompt; it is carried on
    /// the wire so subscribers can render the comment without
    /// re-resolving the policy and to keep the audit trail intact
    /// if a future preset string is reworded (Q4
    /// wording-revision policy). `applied_at` mirrors
    /// `stages.bypassed_at` and is the timestamp the recorder
    /// stamped on the row.
    #[serde(rename = "stage-auto-bypassed")]
    StageAutoBypassed {
        stage_id: StageId,
        policy_name: String,
        comment_used: String,
        applied_at: UnixMillis,
    },

    /// `set_job_policy` replaced (or cleared) the job's
    /// `auto_bypass_policy`. Emitted only when the value actually
    /// changes — a same-policy call is a no-op success that publishes
    /// nothing, keeping cross-window invalidation traffic bounded
    /// (`DOCS/AUTO-BYPASS-DECISIONS.md` Q5 "Idempotency"). `policy_name`
    /// mirrors `AutoBypassPolicy::policy_name()` (one of the five preset
    /// labels or the literal `"Custom"`), or `None` when the policy was
    /// cleared. Subscribers refresh their per-job badge / submit-form
    /// state from this event without re-fetching the whole row.
    #[serde(rename = "job-policy-changed")]
    JobPolicyChanged {
        job_id: JobId,
        #[serde(default)]
        policy_name: Option<String>,
    },

    /// `assistant_threads.updated_at` advanced — a message was appended,
    /// an action card was confirmed/cancelled, or an attachment was
    /// uploaded. The `/assistant` thread-list rail subscribes to this
    /// envelope as its re-sort signal so the order stays live without
    /// the focusStore `refreshTick` polling counter the rail used to
    /// rely on (`DOCS/SCOPE-ASSISTANT-PARITY.md` §W1). The same envelope
    /// also lets the footer composer recount pending cards and the open
    /// thread view re-list messages when a touch arrives from another
    /// surface — every assistant pane keys off the same channel.
    ///
    /// Published with the synthetic `bus_job_id = JobId(thread_id.0)`
    /// the planner already uses (`assistant_planner.rs`), so a
    /// `{ scope: "job", job_id: thread_id }` subscriber sees the touch
    /// alongside the `AiToken` / `AiMessageComplete` envelopes for the
    /// same turn. Subscribers that need touches across every thread
    /// (the rail) subscribe with `scope: "all"`.
    #[serde(rename = "assistant-thread-touched")]
    AssistantThreadTouched { thread_id: AssistantThreadId },
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
