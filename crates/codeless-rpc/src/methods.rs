use codeless_types::pause_point::PausePoint;
use codeless_types::{
    AssistantAction, AssistantActionCard, AssistantAttachment, AssistantMessage,
    AssistantMessageId, AssistantThread, AssistantThreadId, AssistantThreadMode, AutoBypassPolicy,
    ChatBinding, ChatMessage, ChatRole, ChatTransport, FsEntry, FsEntryKind, GitAuth, Job, JobId,
    MessageId, Persona, ProposedScopePatch, Repo, RepoId, Review, ReviewId, ReviewStatus,
    ScopePatchId, Stage, StageId, TaskId, UnixMillis, WorkspaceMode,
};
use serde::{Deserialize, Serialize};

/// Arguments and result types for the typed RPC methods. Kept in their
/// own module so transport adapters can pattern-match on a request enum
/// per method (Phase 3) without touching the trait surface.
///
/// Field names match the column names in SCOPE.md Appendix A wherever
/// the underlying row is being created or returned — the wire form
/// flows straight into `serde_json` payloads.
///
/// Every struct derives `specta::Type` so the wire snapshot in
/// `codeless-types::tests::specta_snapshot` covers RPC inputs and
/// outputs alongside the core domain types. That is what makes the
/// generated TypeScript a complete contract for the UI side.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AddRepoArgs {
    pub name: String,
    pub clone_url: String,
    pub default_branch: String,
    pub local_path: String,
    pub git_auth: GitAuth,
    pub concurrency_cap: Option<u32>,
    pub default_runner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RemoveRepoArgs {
    pub repo_id: RepoId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListReposResult {
    pub repos: Vec<Repo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SubmitJobArgs {
    pub repo_id: RepoId,
    pub prompt: Option<String>,
    pub template_yaml: Option<String>,
    pub runner: String,
    pub branch: String,
    /// `in_repo` (default) edits the user's local clone; `worktree`
    /// creates a separate `git worktree add` checkout. Omit or `null`
    /// to get the default (`in_repo`).
    #[serde(default)]
    pub workspace_mode: Option<WorkspaceMode>,
    pub cost_cap_cents: i64,
    pub wall_clock_cap_ms: i64,
    /// Per-job runner overrides. All three are optional and round-trip
    /// onto `Job` so re-runs inherit the original settings. Adapters
    /// silently ignore knobs they do not support (Copilot has no
    /// `permission_mode` or `effort`, for example).
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// Persona-derived system prompt composed by the caller (UI, CLI)
    /// and applied to every stage of the job. The UI fills this from
    /// the selected persona's `instructions` when the user picks one
    /// from the job-submit dropdown; `None` keeps the server's
    /// configured default. A future stage replaces this with a
    /// `persona_id` lookup against a server-side persona table; until
    /// then the composed text travels on the submit args and is
    /// persisted on the job row so reruns and resumes reproduce it.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Persona the user picked at submit time. The composed prompt
    /// still travels on `system_prompt`; this is the lookup key that
    /// produced it. The runtime persists it verbatim onto the job row
    /// so a rerun can reproduce the same agent posture. `None` means
    /// the user submitted without picking a persona. Personas live in
    /// the UI KV store today, so the field is a free string; the FK
    /// against a server-side persona table lands in a later stage.
    #[serde(default)]
    pub persona_id: Option<String>,
    /// Per-job auto-bypass policy (Surface F). `None` (default) keeps
    /// the existing halt-on-failure behaviour; `Some(...)` pre-
    /// authorises the runtime to advance past a failed stage under
    /// the chosen preset's canned guidance (or a custom comment).
    /// The value is persisted onto the `Job` row verbatim so a
    /// resume / rerun reproduces the same policy.
    #[serde(default)]
    pub auto_bypass_policy: Option<AutoBypassPolicy>,
    /// `false` (default) lands the job in `Draft` status — the row
    /// exists, the user can edit the spec / docs / handover, but the
    /// driver does not pick it up. The user calls `start_job` to
    /// promote `Draft → Queued`. `true` (legacy / power-user behaviour)
    /// skips the draft state and lands directly in `Queued` so the
    /// driver runs immediately.
    #[serde(default)]
    pub start_immediately: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StartJobArgs {
    pub job_id: JobId,
}

/// Re-queue a terminal-but-recoverable job so the driver picks it up
/// again, optionally bumping the caps that ended it. The job's branch,
/// worktree, and captured per-stage `Stage.session_id` are reused —
/// the next claude invocation passes the session id as
/// `CliCfg::resume_id`, which the claude-wrapper renders to
/// `--continue <id>`. The agent picks up the same conversation rather
/// than re-deriving the codebase from scratch. See SCOPE.md hard
/// rule #1: the stage is the session boundary; within a stage the
/// runner session is continuous, and a cost/wall-clock cap is a
/// pause, not a reset.
///
/// Both bumps are *additive* on the existing caps; `None` leaves the
/// cap as-is. A resume that does not raise the cap will simply trip
/// it again — the RPC accepts this, the user is expected to know
/// what they want.
///
/// Errors `Conflict` if the source job is not in a resumable state
/// (`Stopped` or `Failed`), `NotFound` for an unknown id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ResumeJobArgs {
    pub job_id: JobId,
    #[serde(default)]
    pub additional_cost_cap_cents: Option<i64>,
    #[serde(default)]
    pub additional_wall_clock_cap_ms: Option<i64>,
    /// Bypass the most recently failed stage on resume. When `true`,
    /// the runtime marks that stage's `bypassed_at` so the
    /// skip-passed-or-bypassed branch in `TemplateRunner` advances
    /// past it instead of re-running. The stage row stays
    /// `Failed` in the database; the bypass is a *forward* advance,
    /// not a rewrite of history. Defaults to `false` so callers
    /// that did not set it keep the existing retry semantics. The
    /// serde alias preserves wire compatibility with callers that
    /// were written against the earlier `bypass_failing_stage` name
    /// shipped under SCOPE-MUTABLE-UI Surface E.
    #[serde(default, alias = "bypass_failing_stage")]
    pub bypass: bool,
    /// Operator-supplied free-text comment threaded into the next
    /// stage's prompt under an `# Operator comment` heading (the
    /// same envelope auto-bypass uses, so the model parses one form,
    /// not two). Surfaces the Slack `resume <job-id> "<comment>"`
    /// shape and the equivalent UI affordance. `None` keeps the
    /// existing resume semantics; an empty string is treated as
    /// `None` by the runtime so a no-op payload from a chat client
    /// does not produce a stray empty heading in the prompt.
    #[serde(default)]
    pub next_stage_comment: Option<String>,
}

/// Operator-explicit escape hatch for a REVIEW stage's diff-verify
/// pre-check failure. A plain `resume_job` re-runs the same
/// deterministic pre-check against the same handover and fails
/// identically — the gate has no inputs left to change. This RPC
/// flips a one-shot `precheck_override_once` flag on the job row
/// that the runner consumes atomically just before the gate runs,
/// then enters the resume path so the rest of the recovery (caps,
/// comment threading, status transition) flows through the same
/// code as a normal resume. The required `comment` lands on the
/// pending-operator-comment slot so the model sees why the gate
/// was bypassed; the runtime refuses an empty comment because
/// "override silently" is never the right operator intent here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct OverridePreCheckAndResumeArgs {
    pub job_id: JobId,
    pub comment: String,
    #[serde(default)]
    pub additional_cost_cap_cents: Option<i64>,
    #[serde(default)]
    pub additional_wall_clock_cap_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GetJobArgs {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListJobsArgs {
    /// `None` returns jobs across every repo.
    pub repo_id: Option<RepoId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListJobsResult {
    pub jobs: Vec<Job>,
}

/// Per-stage rollup returned by `list_stages`. Carries the canonical
/// `Stage` row plus values rolled up from child tasks so the UI can
/// render duration and cost without a second query. `task_count`
/// surfaces "how many task rows backed this stage" — useful for
/// distinguishing "stage ran with 0 tasks recorded" (recorder ran
/// but no AI work happened) from "stage was never persisted at all"
/// (pre-recorder job).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StageRollup {
    pub stage: Stage,
    pub cost_cents: i64,
    pub task_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListStagesArgs {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListStagesResult {
    pub stages: Vec<StageRollup>,
}

/// Read-only enumeration of the operator-authored pause points for one
/// job. The UI seeds its planned-pause divider chips from this list on
/// `JobPage` mount and re-fetches when a `template-resynced` event
/// fires; the schedule itself is rewritten server-side inside
/// `resync_template_from_disk`, so a stale snapshot is only a one-tick
/// problem. Returns an empty list for jobs that predate scoped pause
/// points or whose template carries no `pause_points:` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListScheduledPausePointsArgs {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListScheduledPausePointsResult {
    pub points: Vec<PausePoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobReportArgs {
    pub job_id: JobId,
}

/// One stage entry in the report. `attempt` distinguishes a re-run of
/// the same `ordinal` (e.g. a cost-capped stage 0 that was resumed —
/// the resume row carries a fresh `session_id` and a separate row
/// here so callers can see both the failure and the recovery).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobReportStage {
    pub ordinal: u32,
    pub attempt: u32,
    pub title: String,
    pub status: String,
    pub session_id: Option<String>,
    pub cost_cents: i64,
    pub duration_ms: Option<i64>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

/// One ai-message-complete event, i.e. one Claude reply. The cost is
/// the marginal cost the runner reported for that reply; sum across
/// turns to recover the job total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobReportTurn {
    pub task_id: String,
    pub stage_ordinal: Option<u32>,
    pub cost_cents: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobReportToolCall {
    pub tool: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobReportEventTally {
    pub kind: String,
    pub count: u32,
}

/// One bucket in the "spec changes" rollup. `kind` is `"template"`
/// (a `JobTemplateUpdated` event, from `update_job_template` or the
/// `start_job` / `resume_job` resync of a chat-driven on-disk edit)
/// or `"file"` (a `JobFileUpdated` event, from `write_job_file` /
/// `delete_job_file`). `filename` is set only for `"file"` rows so
/// the UI can render "SCOPE.md ×3, WORKFLOW.md ×1" without joining
/// against the raw events table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobReportSpecChange {
    pub kind: String,
    pub filename: Option<String>,
    pub count: u32,
    pub last_at: i64,
}

/// Structured report for one job. The UI's Summary tab renders this;
/// scripts can also curl it for cron-style digests. Counts are exact
/// (full table scans over the events table for one job_id); costs are
/// summed from the persisted `ai-message-complete` payloads, which
/// is the same source the dashboard's `cost_cents` rollup uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobReportResult {
    pub job_id: JobId,
    pub status: String,
    pub stop_reason: Option<String>,
    pub cost_cents: i64,
    pub cost_cap_cents: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub wall_clock_ms: Option<i64>,
    pub stages: Vec<JobReportStage>,
    pub turns: Vec<JobReportTurn>,
    pub tool_calls: Vec<JobReportToolCall>,
    pub event_tally: Vec<JobReportEventTally>,
    pub spec_changes: Vec<JobReportSpecChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StopJobArgs {
    pub job_id: JobId,
}

/// Argument shape for the `reset_job` recovery hatch. A job stuck in
/// `Queued` (driver kept failing before reaching `Running`), `Failed`,
/// or `Stopped` is moved back to `Draft` so the operator can edit the
/// spec or simply re-`start_job` without the resume-cap dance. The
/// captured worktree (if any) is reaped and `worktree_path` is
/// cleared; `stop_reason` and `ended_at` are wiped so the row reads
/// like a fresh draft. Refused for `Running`, `Paused`, `AwaitingReview`,
/// and `Completed` — those go through `stop_job` / `pause_job` /
/// `resume_job` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ResetJobArgs {
    pub job_id: JobId,
}

/// Patch mutable fields on a job. Only editable while the job is
/// `Draft` or a terminal state (`Stopped`, `Failed`, `Completed`).
/// Every field is optional — `None` means "leave unchanged".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpdateJobArgs {
    pub job_id: JobId,
    #[serde(default)]
    pub runner: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub cost_cap_cents: Option<i64>,
    #[serde(default)]
    pub wall_clock_cap_ms: Option<i64>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeleteJobArgs {
    pub job_id: JobId,
}

/// Move a `Running` (or `AwaitingReview`) job to `Paused`. The
/// captured per-stage `Stage.session_id` becomes the resume handle
/// for the next `resume_job` call; the in-flight runner is cancelled
/// (cleanly, at the next `await` boundary — any tool call currently
/// running on disk will finish before the runner exits). Distinct
/// from `stop_job` because the *intent* differs: pause is "I'll come
/// back," stop is "I'm done."
///
/// Errors `Conflict` when the job is not in a pausable state
/// (anything but Running / AwaitingReview), `NotFound` for an
/// unknown id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PauseJobArgs {
    pub job_id: JobId,
}

/// Re-queue a job using the same prompt, runner, caps, and repo as a
/// previous run. A fresh `JobId` is minted; the branch is left empty
/// so `WorktreeManager` falls back to `codeless/job-<new_id>` and the
/// original run's branch stays untouched. `source_job_id` may be in
/// any status — re-running a still-running job is allowed; the caller
/// is asserting "give me a clean attempt, don't touch the original".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RerunJobArgs {
    pub source_job_id: JobId,
}

/// Reclaim disk used by job worktrees that the user is done with.
/// `older_than_ms` selects worktrees whose directory mtime is older
/// than `now - older_than_ms`; `None` means "no age filter, match all
/// candidates". `job_ids` further restricts the set to specific jobs
/// (independent of age). When both are `None` every worktree under
/// the configured root is a candidate — explicit and dangerous,
/// which is why the UI defaults the modal to a dry run.
///
/// `dry_run: true` returns the matching entries without removing
/// anything so the UI can preview size and paths before confirming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GcWorktreesArgs {
    pub older_than_ms: Option<i64>,
    pub job_ids: Option<Vec<JobId>>,
    pub dry_run: bool,
}

/// A single worktree the GC sweep considered. `removed` is true if
/// `gc_worktrees` actually deleted it; false on dry-run or when the
/// underlying `git worktree remove` failed (per-entry failures
/// surface here instead of failing the whole RPC so partial
/// reclamation is observable). `size_bytes` is the on-disk size as
/// of the sweep — best-effort, not transactional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GcWorktreeEntry {
    pub job_id: Option<JobId>,
    pub path: String,
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
    pub removed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GcWorktreesResult {
    pub entries: Vec<GcWorktreeEntry>,
    pub total_size_bytes: i64,
    pub removed_count: i64,
    pub root: Option<String>,
}

/// Filter for `list_reviews`. All fields compose with AND; `None`
/// means "do not narrow on this column". Returned rows are ordered by
/// `requested_at` ascending so a UI can render the oldest pending
/// review first without re-sorting. The per-job filter joins through
/// `stages` so the UI's per-job review panel does not need to map
/// stages to jobs client-side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, specta::Type)]
pub struct ListReviewsArgs {
    pub job_id: Option<JobId>,
    pub stage_id: Option<StageId>,
    pub status: Option<ReviewStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListReviewsResult {
    pub reviews: Vec<Review>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ApproveReviewArgs {
    pub review_id: ReviewId,
}

/// Adds a free-form comment to a review without changing its status.
/// `Pending` reviews stay pending so the operator can keep iterating;
/// the final approve / stop call lands a terminal status transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentReviewArgs {
    pub review_id: ReviewId,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StopReviewArgs {
    pub review_id: ReviewId,
}

/// Paths in every `fs_*` arg are interpreted relative to the attached
/// workspace identified by `repo_id`. The runtime resolves `repo_id`
/// to the workspace's `fs_root_canonical` via the
/// `attached_workspaces` table and hands that to the host adapter as
/// the jail root for the call. An unknown or detached `repo_id` is
/// refused with a typed error before the adapter is consulted, so a
/// stale browser tab cannot read or write into a workspace that was
/// detached out from under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsReadDirArgs {
    pub repo_id: RepoId,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsReadDirResult {
    pub entries: Vec<FsEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsReadFileArgs {
    pub repo_id: RepoId,
    pub path: String,
}

/// Result of `fs_read_file`. Binary and over-limit cases will gain
/// their own variants on this struct when the editor needs them;
/// the explorer/editor MVP only handles utf-8 text. Files that fail
/// to decode return `InvalidArgument` for now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsReadFileResult {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsWriteFileArgs {
    pub repo_id: RepoId,
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsStatArgs {
    pub repo_id: RepoId,
    pub path: String,
}

/// Single-entry stat. `kind` is `None` if the path does not exist —
/// the call still succeeds so callers can probe existence without
/// catching `NotFound`. Present-entry stats populate `kind`, `size`,
/// `mtime` from the same source as `fs_read_dir`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsStatResult {
    pub kind: Option<FsEntryKind>,
    pub size: Option<i64>,
    pub mtime: Option<UnixMillis>,
}

/// Arguments for `fs_cwd`. Carries the `repo_id` of the workspace the
/// UI wants the absolute root for; the runtime returns that
/// workspace's `fs_root_canonical` so two browser tabs viewing two
/// different workspaces each anchor their explorer at their own root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsCwdArgs {
    pub repo_id: RepoId,
}

/// Result of `fs_cwd`. The path is the absolute server root the
/// `fs_*` methods are scoped under for `repo_id`. The UI uses this to
/// populate the explorer when no terminal has yet set a working
/// directory, so the first browser visit against a real server shows
/// the workspace contents instead of an empty pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsCwdResult {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsCreateFileArgs {
    pub repo_id: RepoId,
    pub path: String,
    pub content: Option<String>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsCreateDirArgs {
    pub repo_id: RepoId,
    pub path: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsMoveArgs {
    pub repo_id: RepoId,
    pub from: String,
    pub to: String,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsDeleteArgs {
    pub repo_id: RepoId,
    pub path: String,
    pub recursive: bool,
}

/// One entry in `ServerInfo.runners`. The `id` matches the runner key
/// the driver dispatches on (`mock`, `claude`, `anthropic`); the UI
/// uses it as the value submitted in `SubmitJobArgs.runner`. `default`
/// flags the runner the UI should pre-select when opening the submit
/// dialog — the server picks at most one, with a stable preference for
/// real runners over the mock so a freshly-`--enable-claude` server
/// does not silently default new jobs to the demo path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RunnerInfo {
    pub id: String,
    pub default: bool,
}

/// Best-effort probe result for Claude Code on the host. The host
/// adapter populates this at server boot when `--enable-claude` is
/// passed; the UI consumes it on the settings → Models surface to
/// render an actionable hint ("Install Claude Code", "Run
/// `claude auth login`", "Ready"). `authenticated` is `Some(true)` /
/// `Some(false)` only when the probe could parse a definite answer;
/// `None` means the binary exists but the wrapper did not report
/// auth state within the probe budget, so the UI should fall back to
/// the neutral "binary detected" hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ClaudeStatus {
    pub binary_path: String,
    pub version: Option<String>,
    pub authenticated: Option<bool>,
}

/// `GET /server/info` payload. Sits outside the bearer gate alongside
/// `/healthz` and `/version` — the UI must reach it before the user
/// can supply a token, since the runner dropdown and "demo mode"
/// banner both depend on it. No mutable runtime state leaks here; it
/// is a snapshot of how `codeless serve` was configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ServerInfo {
    pub version: String,
    pub runners: Vec<RunnerInfo>,
    /// Frozen at boot to the `--fs-root` flag the operator launched
    /// `codeless serve` with (or `None` when the flag was omitted). Does
    /// **not** track the live `attached_workspaces` set: attaching or
    /// detaching a workspace at runtime never rewrites this field, so
    /// the UI must read `list_workspaces` for the active roster. The
    /// field stays in `ServerInfo` because some shells (the bootstrap
    /// banner, the demo-mode hint) need the boot-time path before any
    /// authenticated RPC has run.
    pub fs_root: Option<String>,
    pub worktree_root: Option<String>,
    /// `Some` when the `claude` runner is enabled and the host probe
    /// ran. The probe is cheap (one `--version` invocation plus an
    /// optional 2 s auth check) so it runs once at boot; the UI need
    /// not poll. `None` when the runner is disabled — the settings
    /// surface renders an "enable with --enable-claude" hint instead.
    pub claude: Option<ClaudeStatus>,
    /// CLI coder runners (`claude`, `codex`, `copilot`) the host has
    /// probed and found ready — binary discoverable, basic auth state
    /// surface-able. The footer agent panel reads this to filter its
    /// model dropdown so the user never picks a runner that will
    /// immediately fail. Probed once at boot via `Runner::ready`;
    /// re-launch the server to refresh after installing a new CLI.
    #[serde(default)]
    pub available_cli_runners: Vec<String>,
    /// Boot-time capability flags. The runtime sets each flag once
    /// the underlying capability is real; the UI gates surfaces on
    /// the corresponding flag rather than shipping a row that may
    /// silently lie when the runtime side has not yet landed. New
    /// flags must default to `false` so older runtimes parse a
    /// shorter payload as "capability absent" rather than missing
    /// the field and crashing the UI deserialisation.
    #[serde(default)]
    pub feature_flags: ServerFeatureFlags,
    /// Loopback REST endpoint exposed alongside this runtime. The
    /// Tauri desktop shell embeds `codeless-server` on an ephemeral
    /// `127.0.0.1` port so external tools (scripts, AI agents) can
    /// reach the same in-process runtime over HTTP without spawning a
    /// second `codeless serve`. `None` for hosts that do not expose a
    /// REST surface (the bare `codeless serve` path leaves this
    /// `None` because the server itself *is* the REST surface and the
    /// bind address is already public in its launch banner).
    #[serde(default)]
    pub rest_url: Option<String>,
}

/// Capability bits the runtime advertises to the UI at boot. Each
/// flag is `false` by default and flips to `true` only when the
/// runtime can guarantee the corresponding behaviour end-to-end.
/// Whoever lands the runtime support also flips the flag, in the
/// same change, so the UI never has to special-case "flag exists
/// but the underlying behaviour is half-built".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ServerFeatureFlags {
    /// `true` once the handover writer round-trips
    /// `<!-- SCOPE-PATCH-BEGIN ... -->` / `<!-- SCOPE-PATCH-END -->`
    /// markers without dropping or rewriting them — the precondition
    /// for the SESSION-MUTABLE-SCOPE patch flow to surface in the UI.
    /// Surface A's "Patches proposed: N" counter row is omitted while
    /// this is `false`, on the principle from
    /// `DOCS/SCOPE-MUTABLE-UI-DECISIONS.md` OQ#1 that a counter row
    /// gated on a half-built capability would lie. Step 2 of the
    /// scope-mutable-ui ramp lands the handover-schema fix and flips
    /// this to `true` from `build_server_info`.
    #[serde(default)]
    pub scope_patch_handover_round_trip: bool,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            runners: Vec::new(),
            fs_root: None,
            worktree_root: None,
            claude: None,
            available_cli_runners: Vec::new(),
            feature_flags: ServerFeatureFlags::default(),
            rest_url: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobDiffArgs {
    pub job_id: JobId,
}

/// Per-file entry in `JobDiffResult.files`. Mirrors the columns the
/// UI's files-changed tab needs: filename, what kind of change
/// (added / modified / deleted / renamed), and line counts so the
/// summary row can show `+N -M` without parsing the patch. `patch`
/// is the unified-diff body for the single file; clients render it
/// or skip rendering when the file is binary (in which case `patch`
/// is the empty string and `is_binary` is true).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobDiffFile {
    pub path: String,
    /// `"A" | "M" | "D" | "R"` matching `git diff --name-status`.
    /// Kept as a one-letter string so future status codes (`"C"` for
    /// copied, `"T"` for type-change) don't break the wire when they
    /// surface.
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub is_binary: bool,
    pub patch: String,
}

/// Diff of a job's branch against its repo's default branch. Computed
/// server-side via `git diff` so it works after the worktree has been
/// reaped (the branch survives in the source repo). Returns
/// `NotFound` if the job, repo, or branch is gone; `Internal` wraps
/// `git diff` failures with the stderr the operator can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobDiffResult {
    /// The base ref the diff was computed against (typically the
    /// repo's `default_branch`). Returned so the UI can label the
    /// comparison ("vs main").
    pub base: String,
    /// The job's branch. Same as `codeless/job-<job_id>`.
    pub head: String,
    pub files: Vec<JobDiffFile>,
}

/// Arguments for `list_job_files`. The runtime resolves `job_id` to
/// `<repo>/.codeless/jobs/<template.name>/` and returns the directory
/// contents. A job that has no parseable `template_yaml` (a raw
/// `prompt`-only job) returns `InvalidArgument` — the directory
/// surface is template-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListJobFilesArgs {
    pub job_id: JobId,
}

/// One file under `<repo>/.codeless/jobs/<name>/`. The boolean flags
/// give the UI everything it needs to render the file list without
/// re-parsing filenames: the spec gets a "(spec)" suffix and is
/// pinned to the top of the list, `SCOPE.md` / `WORKFLOW.md` render
/// with their own affordances (preset buttons stay hidden when the
/// file is already there), and any other markdown renders plain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobFileEntry {
    /// Basename only — no directory segments. The runtime guarantees
    /// no nested files exist (the surface is one level deep).
    pub name: String,
    /// True for `template.yaml`. There is at most one such entry per
    /// job; the UI uses it to label the file and to suppress the
    /// delete affordance.
    pub is_template: bool,
    /// True for `SCOPE.md` (case-insensitive). UI uses this to skip
    /// the "+ scope preset" button when the file is already on disk.
    pub is_scope: bool,
    /// True for `WORKFLOW.md` (case-insensitive). Same rationale.
    pub is_workflow: bool,
}

/// Result of `list_job_files`. `entries` is ordered: `template.yaml`
/// first (when present), then `*.md` in filename-ascending order
/// (other extensions interleave alphabetically). `layout` reports
/// `"directory"` / `"flat"` / `"none"` so the UI can render the
/// legacy-flat hint when migration hasn't happened yet — see
/// `DOCS/JOB-DIR.md` "Layout marker".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListJobFilesResult {
    pub entries: Vec<JobFileEntry>,
    pub layout: String,
    /// Absolute path of the job directory on disk, when it exists.
    /// `None` for the `"none"` / `"flat"` layouts where the directory
    /// has not been created yet. The UI surfaces this via "Open in
    /// editor tab" for users who prefer a host-side editor for long
    /// docs.
    pub directory_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ReadJobFileArgs {
    pub job_id: JobId,
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ReadJobFileResult {
    pub content: String,
}

/// `write_job_file` creates or overwrites a single file under the
/// job directory. The runtime sanitises the filename, refuses
/// `template.yaml` (callers use `update_job_template` for the spec),
/// and migrates flat→directory transparently on the first write
/// against a legacy-flat job. Each migration step is its own commit
/// so `git log` records the move explicitly — see `DOCS/JOB-DIR.md`
/// "Migration".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WriteJobFileArgs {
    pub job_id: JobId,
    pub filename: String,
    pub content: String,
}

/// Result of a successful write. `name` is the *normalised* filename
/// the runtime stored (a bare `design` becomes `design.md`). The UI
/// uses this to re-select the file in the list after the round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WriteJobFileResult {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeleteJobFileArgs {
    pub job_id: JobId,
    pub filename: String,
}

/// Replace a job's spec. The new YAML is validated through the same
/// parser the runner uses (`name` / `goal` / `stages` non-empty);
/// invalid YAML returns `InvalidArgument`. Writes
/// `<repo>/.codeless/jobs/<name>/template.yaml`, promoting from the
/// legacy flat layout in two commits when needed. The job's
/// `template_yaml` DB row is refreshed so subsequent prompt builds
/// and StageTree renders see the new shape immediately.
///
/// Renaming the job is **refused** — `template.name` must equal the
/// current value. Job directories are addressed by name, and a rename
/// would orphan `SCOPE.md`/`WORKFLOW.md`/extras under the old name;
/// the right way to rename is to submit a fresh job and migrate by
/// hand. The runtime returns `Conflict` so the UI can render an
/// actionable error rather than a generic 400.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpdateJobTemplateArgs {
    pub job_id: JobId,
    pub template_yaml: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpdateJobTemplateResult {
    /// The job's resolved template name after the update — always
    /// equal to the pre-update name (renames are rejected). Returned
    /// so the UI can re-select the now-edited spec in its file list
    /// without a separate `list_job_files` round trip.
    pub name: String,
}

/// Seed (or overwrite) the per-stage handover. JOB-MODEL.md (H1)
/// keys handover by stage: the file lives at
/// `<worktree>/runs/<job_id>/<stage_id>/handover.md`. Callers may
/// supply `stage_id` explicitly; when omitted the runtime resolves to
/// the job's highest-ordinal stage so the existing UI seeding flow
/// keeps working without picking a stage by hand. The job must have
/// a worktree (`job.worktree_path` non-null) and at least one stage;
/// otherwise the call returns `Conflict`. The supplied `handover`
/// must have non-empty `done` and `next` sections (H7);
/// `InvalidArgument` is returned for a violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WriteHandoverArgs {
    pub job_id: JobId,
    pub handover: codeless_types::Handover,
    /// Optional stage id. When omitted, the runtime writes to the
    /// job's highest-ordinal stage (the current "active" stage).
    #[serde(default)]
    pub stage_id: Option<codeless_types::StageId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WriteHandoverResult {
    /// Absolute path the runtime wrote, so the UI can surface it
    /// (e.g. for "open in editor tab"). Always inside the job's
    /// worktree under `runs/<job_id>/<stage_id>/handover.md`.
    pub path: String,
}

/// Rewrite a job's `SCOPE.md` from chat. The assistant surface
/// dispatches a confirmed `EditScope` action card through this RPC so
/// the paused-job guard lives behind a single named entry point; the
/// raw `write_job_file` path stays unchanged for callers that opt into
/// editing a live job. `content` is taken verbatim and written through
/// the same commit pipeline as a Spec-pane save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpdateJobScopeArgs {
    pub job_id: JobId,
    pub content: String,
}

/// Result of `jobs.updateScope`. `filename` is the basename the runtime
/// wrote (`SCOPE.md`) so the UI can re-select the file without a
/// `list_job_files` round trip — same shape as `WriteJobFileResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpdateJobScopeResult {
    pub filename: String,
}

/// Mint a fresh `Draft` job from the latest pending `DraftJob` action
/// card in an assistant thread. The runtime walks the thread's
/// transcript newest-to-oldest, pulls the proposal out of the card's
/// `meta_json`, and dispatches `submit_job` with `start_immediately =
/// false` so the row lands in `Draft` for the user to edit before
/// queueing. Returns the new `Job`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DraftJobFromConversationArgs {
    pub thread_id: codeless_types::AssistantThreadId,
}

/// One-shot chat turn routed to a CLI coder runner (Claude Code,
/// Codex, Copilot). The runner is invoked in the server's working
/// directory with the user's prompt; streaming output flows back via
/// the regular event bus, scoped by `session_id` so the panel can
/// subscribe to just its own turn without seeing job-driven traffic.
///
/// `session_id` is opaque to the runtime — no `jobs` row backs it.
/// The `events` table has no FK on `job_id`, so synthetic IDs persist
/// alongside real ones and `subscribe(EventFilter::Job)` matches them
/// uniformly. Choosing a `JobId` as the wire type keeps the existing
/// envelope shape and avoids growing the `EventFilter` enum for v1.
///
/// Standalone per-turn: no multi-turn session continuation in v1. A
/// future revision can carry a `previous_session_id` so the wrapper
/// can pass `--continue` to the underlying CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentChatArgs {
    /// Wire id of the CLI runner: `claude`, `codex`, or `copilot`.
    /// Matches `ServerInfo.available_cli_runners` entries. REST
    /// runners (`anthropic`, `openai`) are rejected with
    /// `InvalidArgument` — those still run browser-direct via the
    /// existing DirectChatTransport path.
    pub runner: String,
    pub prompt: String,
    /// Caller-minted correlation id. The client subscribes with
    /// `EventFilter::Job { job_id: session_id }` before issuing the
    /// call so it sees every emitted event.
    pub session_id: JobId,
    /// Per-call working-directory override. When `Some`, the runner
    /// runs in this directory instead of the server's configured chat
    /// cwd; used by the per-job chat panel so questions like "how many
    /// rows in the csv" can read files that live on the job's branch
    /// (the file may not exist in the server's cwd if the worktree
    /// hasn't been merged). The runtime resolves and canonicalises the
    /// path; values outside the configured fs roots are rejected with
    /// `InvalidArgument` so a chat turn can't be used to read arbitrary
    /// host paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Optional structured context the UI knows about and the model
    /// should as well: attached files, where the user is in the app,
    /// any selection, and saved prompt snippets the user opted in.
    /// Additive — new optional fields land here without breaking
    /// existing clients. The runtime renders this into a deterministic
    /// preamble prepended to `prompt` before the runner is spawned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ChatContext>,
    /// What the user means this turn to do. `Work` (default) is the
    /// existing behaviour: full ambient tools, agent edits whatever
    /// it needs in the worktree. `Spec` narrows the agent to editing
    /// `.codeless/jobs/<name>/*` (template.yaml, SCOPE.md, etc.) —
    /// the runtime swaps in a stricter preamble and passes
    /// `allowed_tools` to the CLI runner so the agent literally
    /// cannot Bash / commit / touch repo source. Mirrors the
    /// claude-code plan-mode mental model: an explicit intent
    /// signal so the user isn't relying on the agent inferring it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ChatMode>,
}

/// Mode the chat composer is in for a given turn. Default `Work`
/// keeps the existing footer/job chat behaviour; `Spec` flips the
/// preamble to "you are authoring the job spec" and clamps tool
/// access so the agent cannot accidentally edit repo source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ChatMode {
    #[default]
    Work,
    Spec,
}

/// Forward-compatible bag of "what the user has on screen / wants
/// included" passed into a chat turn. Every field is optional and
/// the runtime tolerates an empty struct — adding a new field is a
/// non-breaking change for older UIs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ChatContext {
    /// Files previously written via `upload_chat_attachment`. The
    /// runtime renders these as a `Files attached:` list in the
    /// preamble; the runner reads them from the worktree cwd.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ChatAttachmentRef>,
    /// Where the user invoked chat from in the UI, e.g.
    /// `jobs/01H…`, `repos/myrepo`, `settings/models`. Free-form so
    /// the UI can evolve its routes without a wire change. Renders
    /// into the preamble as a "User is viewing:" line so the model
    /// can ground answers to the active surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_location: Option<String>,
    /// Currently-selected text in the UI (editor selection, log
    /// snippet, diff hunk). Optional and bounded by the UI to a
    /// sensible size before sending — the runtime does not truncate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// Named or ad-hoc prompt snippets the user opted in for this
    /// turn (saved from a prompt library, project conventions, etc.).
    /// Each entry is rendered as its own block in the preamble; the
    /// UI is responsible for ordering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_prompts: Vec<UserPromptSnippet>,
    /// Other jobs the user wants folded into this turn's preamble.
    /// Each entry opts into the referenced job's spec files and/or
    /// recent history snapshot. Restricted server-side to refs whose
    /// repo matches the active job's — cross-repo refs return
    /// `InvalidArgument`. Additive: an empty vec preserves the
    /// pre-`job_refs` preamble shape exactly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub job_refs: Vec<JobContextRef>,
}

/// One referenced job folded into the chat preamble. Toggles let the
/// caller pick a spec-only attach (cheap, useful for "what is job B
/// trying to do"), a history-only attach (cheap-ish, useful for "what
/// has job B actually done lately"), or both. `history_turn_limit`
/// bounds the walk before rendering so a long-running job doesn't
/// silently blow the per-section byte budget on the runtime side.
/// `None` means "use the runtime default".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct JobContextRef {
    pub job_id: JobId,
    pub include_spec: bool,
    pub include_history: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_turn_limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ChatAttachmentRef {
    /// Path relative to the job worktree root, as returned by
    /// `upload_chat_attachment`. Used verbatim in the prompt
    /// preamble.
    pub relative_path: String,
    /// Optional MIME type the UI sniffed at upload time (e.g.
    /// `image/png`). Surfaced to the model so it can pick the right
    /// reading strategy when the runner supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UserPromptSnippet {
    /// Short label the UI showed the user when they opted in (e.g.
    /// `repo conventions`). Rendered as the block heading in the
    /// preamble.
    pub label: String,
    /// Snippet body. Markdown is allowed; the runtime emits it as-is.
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentChatResult {
    /// Echoed back so a caller that lost track can re-subscribe.
    pub session_id: JobId,
    /// Task id the runner's events are tagged with. The UI uses it to
    /// distinguish AiToken / ToolCall envelopes belonging to this turn
    /// when multiple chat turns share a panel.
    pub task_id: TaskId,
}

/// Drop a binary blob (image, PDF, csv, …) into the job worktree
/// under `.codeless/chat-attachments/` so the next `agent_chat` turn
/// can reference it by path. Files are written with a unique prefix
/// (millis + counter) to avoid collisions and are NOT git-committed —
/// they are out-of-band scratch input for the CLI runner running in
/// the worktree cwd.
///
/// Returns `Conflict` if the job has no worktree yet (runner has not
/// run); the UI surfaces this as "submit the job first".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UploadChatAttachmentArgs {
    pub job_id: JobId,
    /// Basename only — directory components are stripped. Sanitised
    /// server-side; the original is kept as the suffix after the
    /// unique prefix so the filename is recognisable to the model.
    pub filename: String,
    /// Standard base64 (with or without padding). Decoded server-side;
    /// invalid input returns `InvalidArgument`.
    pub content_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UploadChatAttachmentResult {
    /// Path relative to the job worktree root, e.g.
    /// `.codeless/chat-attachments/1700000000000-0-screenshot.png`.
    /// The UI references this in the chat prompt so the CLI runner,
    /// invoked with the worktree as cwd, can read it directly.
    pub relative_path: String,
    /// Absolute path on the server host. Surfaced for parity with
    /// `write_handover`; UI does not need it for the chat flow but
    /// may show it in a tooltip.
    pub absolute_path: String,
}

/// Fire the cancellation token registered for a chat turn so the
/// in-flight CLI runner exits at its next `await` boundary. Idempotent
/// — a missing entry (the turn already completed) is `Ok(())`, not a
/// failure, so the UI can call this even when racing the natural end
/// of the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CancelChatTaskArgs {
    pub task_id: TaskId,
}

/// Stop *whatever* is currently running for `job_id`: the job runner
/// (when the row is `Running` / `AwaitingReview` / `Queued`), every
/// in-flight chat turn whose `session_id` is this job, or both. The
/// umbrella around `stop_job` + `cancel_chat_task` so the UI's stop
/// button has a single endpoint to call regardless of which spawn
/// path is alive. Idempotent — neither path firing is `Ok(())` with
/// `stopped_job: false` and an empty `cancelled_chat_task_ids`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StopActiveArgs {
    pub job_id: JobId,
}

/// What `stop_active` actually did. The UI uses this to surface a
/// "stopped the chat turn" / "stopped the job" / "stopped both" /
/// "nothing was running" status without a second round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StopActiveResult {
    /// `true` when the umbrella issued `stop_job` against a row in
    /// `Running` / `AwaitingReview` / `Queued`. `false` when the row
    /// was already terminal (or paused), so only the chat side could
    /// possibly have fired.
    pub stopped_job: bool,
    /// Per-turn `TaskId`s whose cancel tokens were fired. Empty when
    /// no chat turn was scoped to this job at call time.
    pub cancelled_chat_task_ids: Vec<TaskId>,
}

/// `assistant.listThreads`. No filters in v1 — threads are unscoped
/// (no per-repo / per-job FK by design, see `AssistantThread`), so the
/// list is just "every thread the operator has on this host". Returned
/// rows are ordered by `updated_at` descending so the most recently
/// touched conversation lands at the top of the rail without the UI
/// having to re-sort.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListAssistantThreadsArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListAssistantThreadsResult {
    pub threads: Vec<AssistantThread>,
}

/// `assistant.createThread`. The title is optional at create time so
/// the UI can mint a thread on first message and let the assistant
/// pick a title later (or leave the default). Empty / all-whitespace
/// titles are normalised to the default "New thread" so listings
/// never render a blank rail entry.
///
/// `persona_id` is required (PS5 -- `DOCS/PLUGIN-SUBSTRATE.md` item
/// 5): a thread declares its persona at creation and the runner reads
/// `system_prompt`, `allowed_tools`, `default_model_family`, and
/// `default_attachments_policy` off that row at agent-call time. The
/// substrate doc explicitly forbids a silent default, so an omitted
/// or empty `persona_id` returns `InvalidArgument`; an unknown
/// persona returns `NotFound`. The persona is immutable for the
/// thread's lifetime once persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CreateAssistantThreadArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub persona_id: String,
}

/// `assistant.deleteThread`. Cascades through `assistant_messages` and
/// `assistant_attachments` via the SQLite FK; the on-disk
/// `<codeless-data>/threads/<thread_id>/` directory is removed in the
/// same call so attachments do not outlive the row. Idempotent —
/// `NotFound` is returned for an unknown id so the UI can distinguish
/// "already deleted" from "delete failed".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeleteAssistantThreadArgs {
    pub thread_id: AssistantThreadId,
}

/// `assistant.setThreadMode` (job `assistant-fs-tools` stage 3).
/// Flip a thread's filesystem-tool permission posture to one of the
/// three `AssistantThreadMode` variants. The mode is consulted
/// server-side on every tool dispatch (SCOPE.md "Constraints" — "UI
/// hints, server enforces"), so a stale client cannot keep
/// approve-edits / bypass behaviour after the operator dropped the
/// thread back to read-only.
///
/// `NotFound` for an unknown thread id. `updated_at` is *not* bumped
/// — switching permission posture is not a conversational event and
/// must not re-sort the rail. The wire form for `mode` is one of
/// `read-only` / `approve-edits` / `bypass`; a serde decode failure
/// (typo, unknown variant) surfaces before the handler runs, so the
/// SQLite column never sees an out-of-band string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SetAssistantThreadModeArgs {
    pub thread_id: AssistantThreadId,
    pub mode: AssistantThreadMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SetAssistantThreadModeResult {
    pub thread_id: AssistantThreadId,
    pub mode: AssistantThreadMode,
}

/// `assistant.uploadAttachment`. Drop a binary blob into a thread's
/// attachments directory under
/// `<codeless-data>/threads/<thread_id>/attachments/<id>-<filename>`
/// and insert the index row. The UI references the returned
/// `AssistantAttachment.id` in subsequent turns; the model reads the
/// file from a path the runtime resolves server-side, so the wire
/// never carries a host filesystem path.
///
/// `NotFound` for an unknown thread; `InvalidArgument` for a filename
/// that fails the basename sanitiser or an undecodable base64 body;
/// `Internal` when the runtime has no `<codeless-data>` root
/// configured (tests that omit `with_assistant_data_dir`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UploadAssistantAttachmentArgs {
    pub thread_id: AssistantThreadId,
    /// Basename only — directory components are stripped server-side.
    pub filename: String,
    /// Standard base64 (with or without padding). Decoded server-side;
    /// invalid input returns `InvalidArgument`.
    pub content_b64: String,
    /// Optional MIME type the UI sniffed at drop time. Surfaced back
    /// to the model in the chat preamble so it can pick the right
    /// reading strategy when the runner supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UploadAssistantAttachmentResult {
    pub attachment: AssistantAttachment,
}

/// `assistant.listMessages`. The full transcript for one thread,
/// ordered by `created_at` ascending so the UI can render top-to-bottom
/// without an extra sort. Empty result for a freshly-minted thread —
/// callers distinguish that from a missing thread by issuing the
/// list against a known id; this method itself does not 404 because
/// the alternative ("which is empty, the rail entry or this list?")
/// is the same response for the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListAssistantMessagesArgs {
    pub thread_id: AssistantThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListAssistantMessagesResult {
    pub messages: Vec<AssistantMessage>,
}

/// `assistant.appendMessage`. Persist a user turn into a thread and
/// synthesise an assistant reply in the same round-trip. Stage 6 ships
/// a no-op responder — the assistant message is a fixed acknowledgement
/// — so the surface is end-to-end testable before the planner / tool
/// loop lands. Later stages swap the responder for the real runner
/// without changing the wire shape; the UI keeps treating
/// `AppendAssistantMessageResult` as "two rows the rail should
/// re-render."
///
/// The thread's `updated_at` is bumped so a chat-only interaction
/// re-sorts the rail to the top, matching the touch semantics
/// `upload_assistant_attachment` already established.
///
/// `NotFound` for an unknown `thread_id`; `InvalidArgument` for an
/// empty / all-whitespace `content` (rejecting it server-side keeps
/// the rail from filling with blank rows when the UI accidentally
/// fires send on an empty composer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AppendAssistantMessageArgs {
    pub thread_id: AssistantThreadId,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AppendAssistantMessageResult {
    pub user_message: AssistantMessage,
    pub assistant_message: AssistantMessage,
    /// Action-card rows the planner emitted alongside the text reply.
    /// Each entry is a persisted assistant-role message whose
    /// `meta_json` decodes to an [`AssistantActionCard`]; the UI
    /// appends them to the transcript in order, exactly as it would
    /// after a re-list. Empty when the turn produced no tool calls
    /// (the common case for plain Q&A).
    #[serde(default)]
    pub cards: Vec<AssistantMessage>,
}

/// `assistant.confirmAction`. Dispatch the tool call proposed by a
/// prior `Assistant`-role message whose `meta_json` carries an
/// `AssistantActionCard`. The runtime executes the same `RpcServer`
/// method a direct caller would invoke, then writes a trailing
/// `Tool`-role message with a structured result summary so the
/// transcript captures both the proposal and what happened. The
/// proposal row's `meta_json.status` is flipped to `confirmed` or
/// `failed` in the same transaction so the UI does not have to keep
/// confirm/cancel buttons live after the click.
///
/// `NotFound` for an unknown `message_id`; `InvalidArgument` when the
/// message is not an action card or is no longer pending (already
/// confirmed/cancelled — confirming twice is a no-op the UI shouldn't
/// be able to trigger, but the server is the source of truth).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ConfirmAssistantActionArgs {
    pub thread_id: AssistantThreadId,
    pub message_id: AssistantMessageId,
}

/// Result of an action confirmation. `card` is the proposal row after
/// the status flip — the UI swaps it in place to retire the buttons.
/// `tool_message` is the newly-appended `Tool`-role row carrying the
/// structured outcome (`content` is the human summary; `meta_json`
/// carries the original action + a serde-tagged result payload so the
/// renderer can show e.g. a list of jobs without re-querying).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ConfirmAssistantActionResult {
    pub card: AssistantMessage,
    pub tool_message: AssistantMessage,
}

/// `assistant.cancelAction`. Flip a pending action card's status to
/// `cancelled` and do **nothing else** — no RPC is dispatched, no
/// `Tool` message appended. The card row stays in the transcript so
/// the user can see what they declined, but its confirm/cancel
/// buttons retire.
///
/// `NotFound` for an unknown `message_id`; `InvalidArgument` when
/// the row is not an action card or is no longer pending. The empty
/// success case returns the updated card so the UI can re-render
/// without a follow-up list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CancelAssistantActionArgs {
    pub thread_id: AssistantThreadId,
    pub message_id: AssistantMessageId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CancelAssistantActionResult {
    pub card: AssistantMessage,
}

/// Re-exported here so wire-snapshot consumers see the action-card
/// types in the same module they see the other assistant args. The
/// proposal lives on `AssistantMessage.meta_json` as a JSON-encoded
/// `AssistantActionCard`; exposing the type at the methods boundary
/// keeps the TS bindings honest — without this, `meta_json` would
/// remain an opaque string on the wire surface generated for the UI.
pub type AssistantActionCardPayload = AssistantActionCard;
pub type AssistantActionPayload = AssistantAction;

/// `list_personas`. Snapshot of the `personas` SQLite table, ordered
/// with built-ins first (by id) and user rows after (by `created_at`
/// ascending). Empty result is impossible in practice — migration
/// 0011 seeds the five built-ins — but the wire shape tolerates it so
/// a freshly migrated test database does not need a special case.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListPersonasArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListPersonasResult {
    pub personas: Vec<Persona>,
}

/// `get_persona`. Returns `NotFound` when the id does not resolve. The
/// UI uses this for the per-stage / chat-side lookup once it migrates
/// off the KV-only cache; the cache itself is hydrated by
/// `list_personas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GetPersonaArgs {
    pub id: String,
}

/// `upsert_persona`. Creates a row when `id` is unknown; replaces the
/// editable fields (everything except `built_in`, `created_at`) when
/// `id` already exists. Built-in rows accept body edits — the UI lets
/// users tweak the seeded `Coder` prompt — but the `built_in` flag is
/// preserved by the runtime so a built-in stays a built-in (it cannot
/// be deleted; see `delete_persona`).
///
/// `id` is supplied by the caller. The UI mints user ids with its own
/// prefix (`a-…`); the runtime treats them as opaque. `created_at`
/// and `updated_at` are stamped server-side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpsertPersonaArgs {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub instructions: String,
    pub use_for_jobs: bool,
    #[serde(default)]
    pub default_model: Option<String>,
    pub allowed_subagents: Vec<String>,
    #[serde(default)]
    pub default_snippets: Vec<String>,
}

/// `delete_persona`. Removes one row. Built-in rows (`built_in = 1`,
/// the `builtin:<slug>` ids seeded by migration 0011) are refused with
/// `Conflict` so the UI cannot leave the user with no `Coder`
/// fallback. `NotFound` for an unknown id; idempotent retries against
/// an already-deleted user row return `NotFound` rather than `Ok` so
/// the UI can distinguish "stale list" from "successfully removed".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DeletePersonaArgs {
    pub id: String,
}

/// Approve a proposed scope patch through the UI. Wraps the
/// `codeless patches approve` workflow: removes the entry from
/// `<repo>/DOCS/SCOPE-PROPOSED.md`, stages the queue file plus the
/// proposal's `target_path` plus any `include`d paths, and creates a
/// human-authored commit using the repo-local `git config
/// user.{name,email}` identity. A `Codeless-Approved-By: ui` trailer
/// distinguishes UI-driven approvals from CLI ones in `git log`.
///
/// Idempotent. A second call against an already-resolved patch returns
/// `AlreadyResolved` rather than an error, so a stale UI window can
/// recover its view without surfacing a red toast — see
/// `DOCS/SCOPE-MUTABLE-UI.md` Dependency #3.
///
/// The human must have edited the rulebook target on disk before the
/// call; the RPC does not interpret the proposal body. `target_path`
/// missing from the worktree returns `InvalidArgument` so the UI can
/// guide the user toward editing the file first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ApproveScopePatchArgs {
    pub repo_id: RepoId,
    pub patch_id: ScopePatchId,
    /// Optional override for the commit subject. Defaults to
    /// `scope-patch <kind>: <rationale>` — the same subject the CLI's
    /// `codeless patches approve` produces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Extra repo-relative paths to commit alongside the queue edit and
    /// the proposal's `target_path` (e.g. a paired predicate file and
    /// its fixture). Each path must live inside the worktree root;
    /// outside paths return `InvalidArgument`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
}

/// Reject a proposed scope patch through the UI. Mirrors `codeless
/// patches reject`: removes the entry from `DOCS/SCOPE-PROPOSED.md` and
/// commits the queue edit with a rejection commit body. No rulebook
/// file is touched. Same idempotence semantics as
/// `approve_scope_patch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RejectScopePatchArgs {
    pub repo_id: RepoId,
    pub patch_id: ScopePatchId,
    /// Optional free-form rejection reason recorded in the commit body.
    /// Audit trail only — the runtime does not act on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Edit the body of a proposed scope patch in-place. Mirrors `codeless
/// patches edit` but without the `$EDITOR` round-trip — the UI submits
/// the new markdown directly. The runtime re-parses the supplied
/// payload as a single proposal block; the parsed proposal must have
/// the same `id` as `patch_id`. `body` is the *complete* rendered
/// proposal as it would appear in `DOCS/SCOPE-PROPOSED.md` (i.e.
/// `Proposal::render`'s output), so the round-trip stays loss-free.
///
/// Editing does not produce a commit — the operator typically follows
/// up with `approve_scope_patch`. Idempotency applies in the same
/// shape: editing an already-resolved patch returns `AlreadyResolved`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EditScopePatchArgs {
    pub repo_id: RepoId,
    pub patch_id: ScopePatchId,
    /// Full rendered proposal block as it would appear in the queue
    /// file (heading + bulleted metadata + `### Rationale` + `### Body`
    /// sections). Validated server-side; an unparseable buffer returns
    /// `InvalidArgument`.
    pub rendered: String,
}

/// Undo a previously-applied scope-patch approval by running `git
/// revert <commit_sha> --no-edit` against the worktree at `repo_id`'s
/// `local_path`. Used by the patch-inbox's 10-second undo toast
/// (decision OQ#3): the toast surfaces the approval SHA and a one-click
/// revert, so changing your mind preserves both events in `git log`.
///
/// Not idempotent — calling twice against the same approval SHA
/// produces two revert commits, each undoing the prior one. The UI
/// only exposes this from the transient post-approval toast, so a
/// double-click in practice means the operator intended the second
/// revert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RevertScopePatchArgs {
    pub repo_id: RepoId,
    /// SHA of the approval commit the operator wants to undo. Returned
    /// on the matching `ScopePatchActionResult::Approved { commit_sha }`
    /// from the prior `approve_scope_patch` call.
    pub commit_sha: String,
}

/// Outcome of `revert_scope_patch`. Separate from
/// `ScopePatchActionResult` because revert has no `AlreadyResolved`
/// shape — calling it after the approval has itself been reverted just
/// produces another revert commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RevertScopePatchResult {
    /// SHA of the new revert commit on the current branch.
    pub commit_sha: String,
}

/// Which terminal state a previously-acted-on patch ended up in. Used
/// by `ScopePatchActionResult::AlreadyResolved` so a stale UI window
/// can render the right "this patch was {approved,rejected} by another
/// window" message without a follow-up RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ScopePatchResolution {
    Approved,
    Rejected,
}

/// Outcome of an `approve_scope_patch` / `reject_scope_patch` /
/// `edit_scope_patch` call. The same wire shape is returned by all
/// three so the UI's idempotency handling code path stays uniform:
/// `AlreadyResolved` is a successful response, not an error, and the
/// UI swaps its row to the resolved view without a toast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ScopePatchActionResult {
    /// `approve_scope_patch` produced a human-authored approval commit.
    /// `commit_sha` is the SHA of the new commit, suitable for linking
    /// to a `commit/<sha>` route.
    Approved { commit_sha: String },
    /// `reject_scope_patch` produced a rejection commit.
    Rejected { commit_sha: String },
    /// `edit_scope_patch` rewrote the queue entry. No commit is
    /// produced — `edit` is a pre-approval ergonomic.
    Edited,
    /// The patch had already been resolved by a previous call (possibly
    /// from another window, possibly from the CLI). `resolution`
    /// distinguishes approved vs rejected; `commit_sha` carries the
    /// SHA of the existing resolution commit when the runtime could
    /// locate it in `git log`, `None` when the commit predates the
    /// markers the runtime greps for (legacy approvals).
    AlreadyResolved {
        resolution: ScopePatchResolution,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit_sha: Option<String>,
    },
}

/// Snapshot the unresolved patch queue across one or all repos. Powers
/// Surface C (cross-workspace patch worklist) in
/// `DOCS/SCOPE-MUTABLE-UI.md`: the editor's standing view across every
/// repo, independent of any single job. Wraps the same
/// `scope_patch_queue::load_queue` helper the CLI's `codeless patches
/// list` uses, lifting the per-entry shape into a mobile-safe DTO
/// (`ProposedScopePatch`).
///
/// `repo_id = Some(_)` filters to one repo and returns `NotFound` when
/// that repo row does not exist. `repo_id = None` walks every repo and
/// concatenates the results — repos with no `DOCS/SCOPE-PROPOSED.md`
/// contribute zero entries (not an error), and per-repo parse failures
/// surface as `Internal` so the worklist refuses to half-render under
/// a corrupted queue file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListProposedPatchesArgs {
    /// Restrict the walk to a single repo. `None` lists across every
    /// repo row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<RepoId>,
}

/// One entry in the cross-repo listing — pairs the queue row with the
/// `RepoId` that owns it so Surface C can group by repo without a
/// follow-up `list_repos` join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProposedPatchListEntry {
    pub repo_id: RepoId,
    pub patch: ProposedScopePatch,
}

/// Snapshot the unresolved queue. Ordering is **newest-first by
/// `proposed_at`**, with `None`-timestamped entries (legacy data
/// predating the field) sorted last in `id` order. Surface C layers
/// its 14-day-decay filter and group-by-repo on top of this order; the
/// runtime does not pre-filter so a "show everything" toggle remains
/// implementable client-side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListProposedPatchesResult {
    pub entries: Vec<ProposedPatchListEntry>,
}

/// Argument shape for `set_job_policy`. Replaces the job's
/// `auto_bypass_policy` column with `policy`; `None` clears it so a
/// future stage failure halts the job rather than auto-bypassing.
///
/// The RPC refuses with `Conflict` while the row is `Running` or
/// `Queued` (`DOCS/AUTO-BYPASS-DECISIONS.md` Q5) — those states race
/// the stage-failed handler, so the operator must `pause_job` first.
/// `Draft`, `Stopped`, and `Paused` are accepted. Setting the same
/// policy twice is a no-op success: the second call returns `Ok(())`
/// and emits no `JobPolicyChanged` event, which keeps cross-window
/// invalidation traffic bounded and lets the UI call defensively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SetJobPolicyArgs {
    pub job_id: JobId,
    /// New policy, or `None` to clear the existing one.
    #[serde(default)]
    pub policy: Option<AutoBypassPolicy>,
}

/// Append one message to the per-Job chat thread (`DOCS/JOB-CHAT.md`).
/// Used by the web chat input, the Telegram and Slack adapters, the
/// CLI, and the supervisor agent — every voice ends up as one row in
/// `chat_messages`. The ULID `MessageId` and the `created_at` stamp
/// are minted server-side so a clock-skewed transport cannot reorder
/// the thread.
///
/// `external_id` is the transport-native message id (Telegram
/// `chat:id`, Slack `ts`). Required on Telegram and Slack by SQL
/// invariant — the partial unique index narrows
/// `(transport, external_id)` to non-NULL rows, so a redelivered
/// inbound message would land on a `Conflict` error rather than
/// double-ingest. Web, CLI, and supervisor rows leave it NULL.
///
/// `role` defaults to `User` because that is the overwhelmingly
/// common case (every human transport); the supervisor sets it
/// explicitly to `Assistant`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PostJobMessageArgs {
    pub job_id: JobId,
    pub transport: ChatTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_key: Option<String>,
    pub author: String,
    #[serde(default = "default_post_role")]
    pub role: ChatRole,
    pub body: String,
    /// Raw transport-extras JSON text (attachments, formatting,
    /// outbound delivery receipts). Passed through verbatim to the
    /// `metadata_json` column so the substrate stays opaque per
    /// `codeless_types::chat::ChatMessage::metadata_json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_json: Option<String>,
}

fn default_post_role() -> ChatRole {
    ChatRole::User
}

/// Paginate one Job's chat history newest-first by `created_at` with
/// the message id as tiebreaker. The web `CHAT` tab calls this on
/// mount; Telegram and Slack adapters call it on `/codeless bind` to
/// compose a single condensed "joining mid-thread" summary (full
/// replay would spam the channel — see `JOB-CHAT.md` "Cold-load").
///
/// `before` is the seek cursor: pass `None` to fetch the most recent
/// `limit` rows, then pass the oldest returned `MessageId` to walk
/// further back. The runtime caps `limit` so a runaway caller cannot
/// pull the whole table in one shot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListJobMessagesArgs {
    pub job_id: JobId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<MessageId>,
    pub limit: u32,
}

/// Returned messages are ordered oldest-first within the returned
/// page so a UI can render top-to-bottom without sorting. To walk
/// further back, the caller passes the *oldest* id of this page as
/// the next `before`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListJobMessagesResult {
    pub messages: Vec<ChatMessage>,
}

/// Bind `(transport, channel_id, thread_id)` to a Job so the
/// adapter's inbound path can resolve an arriving message to the
/// right chat thread (`JOB-CHAT.md` "Data model"). Called by
/// `/codeless bind <job_id>` on Telegram / Slack. The web UI never
/// needs this — it already has `job_id` from the URL.
///
/// `thread_id` is normalised to the empty string `""` on the server
/// side when omitted to match the primary-key invariant on
/// `chat_bindings`. The call is idempotent on the PK: a second bind
/// of the same `(transport, channel, thread)` to the same Job
/// returns the existing row stamped with the new `bound_at` /
/// `bound_by`; binding the same key to a *different* Job overwrites
/// (the user re-pointed the channel) and emits `ChatBindingCreated`
/// once goal events land. The runtime stamps `bound_at` itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BindChatThreadArgs {
    pub transport: ChatTransport,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub job_id: JobId,
    pub bound_by: String,
}

/// Record one transport's outbound delivery receipt against a
/// `chat_messages` row. Called by the Telegram / Slack outbound
/// forwarders after a successful platform send so the next event-bus
/// fan-out (after a restart, or to a second forwarder on the same
/// transport) can presence-check `metadata_json.delivery.<transport>`
/// and skip the duplicate post. Per `JOB-CHAT.md` "Transport adapters"
/// the column owners that the runtime guarantees never to overwrite —
/// `body` and `external_id` — stay immutable; only `metadata_json` is
/// mutated, and even there only the substrate-owned `delivery.<transport>`
/// key is touched (OQ-CHAT-5 §metadata keyspace).
///
/// `platform_id` is the id the platform returned for the *outbound*
/// send (Telegram `message_id`, Slack `ts`) — NOT the originating
/// transport's `external_id`. Mixing the two would conflate the source
/// of truth with the delivery receipt; the immutability bias in
/// JOB-CHAT.md exists exactly to keep that line clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UpdateChatMessageDeliveryArgs {
    pub message_id: MessageId,
    pub transport: ChatTransport,
    pub platform_id: String,
}

/// Forward lookup on `chat_bindings`: resolve an inbound
/// `(transport, channel, thread)` to the Job that owns the
/// conversation. Returns `None` when the channel was never
/// `/codeless bind`-ed — the adapter treats that as "drop the
/// message" (the substrate refuses to ingest text the operator has
/// not pointed at a Job). `thread_id` defaults to the empty-string
/// sentinel server-side so callers that come from a non-threaded
/// platform message can pass `None` and still hit the PK row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GetChatBindingArgs {
    pub transport: ChatTransport,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct GetChatBindingResult {
    pub binding: Option<ChatBinding>,
}

/// Reverse lookup of `chat_bindings`: every `(channel, thread)` on the
/// given transport that points at the supplied Job. The outbound
/// forwarder on each transport adapter calls this when a
/// `ChatMessageAppended` fires for a Job it cares about — the bindings
/// it returns are the set of platform-side destinations the message
/// must be forwarded to. The forward direction is the inverse of
/// `get_chat_binding` (which serves the inbound resolver path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListChatBindingsForJobArgs {
    pub job_id: JobId,
    pub transport: ChatTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListChatBindingsForJobResult {
    pub bindings: Vec<ChatBinding>,
}
