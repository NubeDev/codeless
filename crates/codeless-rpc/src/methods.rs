use codeless_types::{
    FsEntry, FsEntryKind, GitAuth, Job, JobId, Repo, RepoId, Review, ReviewId, ReviewStatus, Stage,
    StageId, TaskId, UnixMillis,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StopJobArgs {
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

/// Paths in every `fs_*` arg are interpreted relative to the
/// configured server root. The host adapter rejects any path that
/// escapes the root (`..` segments, absolute paths, symlinks pointing
/// outside) before touching disk — the wire shape carries no notion
/// of "outside root" because callers should never need to express it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsReadDirArgs {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsReadDirResult {
    pub entries: Vec<FsEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsReadFileArgs {
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
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsStatArgs {
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

/// Result of `fs_cwd`. The path is the absolute server root the
/// `fs_*` methods are scoped under. The UI uses this to populate the
/// explorer when no terminal has yet set a working directory, so the
/// first browser visit against a real server shows the workspace
/// contents instead of an empty pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsCwdResult {
    pub path: String,
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

/// Seed (or overwrite) the per-run handover. JOB-MODEL.md says
/// handover lives at `<worktree>/runs/<job_id>/handover.md`; this
/// RPC writes the structured `Handover` shape through the runtime's
/// existing `write_handover` helper. The job must have a worktree
/// (`job.worktree_path` non-null); raw-prompt jobs whose runner has
/// not yet provisioned one get `Conflict`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WriteHandoverArgs {
    pub job_id: JobId,
    pub handover: codeless_types::Handover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WriteHandoverResult {
    /// Absolute path the runtime wrote, so the UI can surface it
    /// (e.g. for "open in editor tab"). Always inside the job's
    /// worktree under `runs/<job_id>/handover.md`.
    pub path: String,
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
