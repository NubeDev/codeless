use async_trait::async_trait;
use codeless_types::{Job, Repo, Review};

use crate::error::RpcResult;
use crate::methods::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, ApproveReviewArgs, CommentReviewArgs,
    DeleteJobFileArgs, FsCwdResult, FsReadDirArgs, FsReadDirResult, FsReadFileArgs,
    FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GcWorktreesArgs,
    GcWorktreesResult, GetJobArgs, JobDiffArgs, JobDiffResult, ListJobFilesArgs,
    ListJobFilesResult, ListJobsArgs, ListJobsResult, ListReposResult, ListReviewsArgs,
    ListReviewsResult, ListStagesArgs, ListStagesResult, ReadJobFileArgs, ReadJobFileResult,
    RemoveRepoArgs, RerunJobArgs, ResumeJobArgs, StartJobArgs, StopJobArgs, StopReviewArgs,
    SubmitJobArgs,
    UpdateJobTemplateArgs, UpdateJobTemplateResult, UploadChatAttachmentArgs,
    UploadChatAttachmentResult, WriteHandoverArgs, WriteHandoverResult, WriteJobFileArgs,
    WriteJobFileResult,
};
use crate::subscribe::{EventFilter, EventStream, Since};

/// The single typed entry point every transport adapts. Browser SSE/REST,
/// Tauri IPC, and the CLI's in-process call site all reach the runtime
/// through this trait — see SCOPE.md "Rule 1 — One transport interface,
/// many implementations".
///
/// Why the entire surface lives on one trait, instead of splitting per
/// resource: it makes the wire schema enumerable. Phase 3 walks the
/// methods, generates HTTP routes and a `specta` TS interface, and the
/// browser side is shaped automatically. Splitting the trait would
/// force the same enumeration to live in a separate registry.
///
/// `async_trait` is used (rather than native `async fn` in traits) so
/// the trait remains object-safe for `Arc<dyn RpcServer>` storage in
/// transport adapters.
#[async_trait]
pub trait RpcServer: Send + Sync + 'static {
    async fn add_repo(&self, args: AddRepoArgs) -> RpcResult<Repo>;
    async fn remove_repo(&self, args: RemoveRepoArgs) -> RpcResult<()>;
    async fn list_repos(&self) -> RpcResult<ListReposResult>;

    async fn submit_job(&self, args: SubmitJobArgs) -> RpcResult<Job>;
    async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job>;
    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult>;
    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()>;

    /// Promote a `Draft` job to `Queued` so the background driver
    /// picks it up. The user calls this once they have edited the
    /// spec / docs / handover and are happy for the job to run.
    /// Errors with `Conflict` if the job is not currently in `Draft`,
    /// and `NotFound` for an unknown id.
    async fn start_job(&self, args: StartJobArgs) -> RpcResult<Job>;

    /// Re-queue a terminal-but-recoverable job, reusing its
    /// per-stage `Stage.session_id` so the next claude invocation
    /// resumes the same conversation via `--continue`. See SCOPE.md
    /// hard rule #1 (the stage is the session boundary) and
    /// `ResumeJobArgs` for the cap-bump semantics. Errors with
    /// `Conflict` if the job is not in a resumable state (`Stopped`
    /// or `Failed`), `NotFound` for an unknown id.
    async fn resume_job(&self, args: ResumeJobArgs) -> RpcResult<Job>;

    /// List the stages of a job, each enriched with rolled-up
    /// `cost_cents` (sum over the stage's tasks) and a `task_count`.
    /// Returns an empty list when the job has no persisted stages
    /// (pre-recorder jobs, mock jobs without a template). The UI
    /// renders an event-derived fallback view in that case.
    async fn list_stages(&self, args: ListStagesArgs) -> RpcResult<ListStagesResult>;

    /// Mint a new job that clones the prompt/runner/caps/repo of an
    /// existing one. Returns the newly-queued job. The original job
    /// is untouched. Errors with `NotFound` for an unknown source id.
    async fn rerun_job(&self, args: RerunJobArgs) -> RpcResult<Job>;

    /// Inspect, and optionally remove, on-disk worktrees the user is
    /// done with. Dry-run returns the matching entries (with sizes)
    /// without touching anything so a UI can preview before
    /// committing. With a real run, per-entry `git worktree remove`
    /// failures land on `GcWorktreeEntry.error` rather than failing
    /// the whole call — partial reclamation is observable. Errors
    /// with `Internal` if no worktree root is configured on the
    /// runtime; `Conflict` is not used because there's no per-job
    /// state to clash with.
    async fn gc_worktrees(&self, args: GcWorktreesArgs) -> RpcResult<GcWorktreesResult>;

    /// Compute the diff between the job's branch and the repo's
    /// default branch. Works whether or not the worktree is still on
    /// disk — the branch is the durable artefact. Errors with
    /// `NotFound` for an unknown job or a missing branch; wraps
    /// `git diff` failures as `Internal` with stderr included.
    async fn job_diff(&self, args: JobDiffArgs) -> RpcResult<JobDiffResult>;

    async fn list_reviews(&self, args: ListReviewsArgs) -> RpcResult<ListReviewsResult>;
    /// Resolve a `Pending` review to `Approved`. Rejects with
    /// `Conflict` if the review has already been resolved; rejects
    /// with `NotFound` for an unknown id. Publishes `review-approved`
    /// on success.
    async fn approve_review(&self, args: ApproveReviewArgs) -> RpcResult<Review>;
    /// Attach a comment to a review. Only the comment field changes
    /// — the status stays put, even for already-resolved reviews, so
    /// post-mortem notes remain possible. Publishes `review-commented`.
    async fn comment_review(&self, args: CommentReviewArgs) -> RpcResult<Review>;
    /// Resolve a `Pending` review to `Stopped`. Same conflict / not-
    /// found semantics as `approve_review`. Publishes `review-stopped`.
    async fn stop_review(&self, args: StopReviewArgs) -> RpcResult<Review>;

    /// Streaming subscription. The returned stream replays events
    /// strictly after `since` (if `Some`) and then continues live.
    async fn subscribe(&self, filter: EventFilter, since: Since) -> RpcResult<EventStream>;

    /// List one directory's immediate children. The path is relative
    /// to the server root; traversal outside the root is rejected by
    /// the host adapter, not at the wire level.
    async fn fs_read_dir(&self, args: FsReadDirArgs) -> RpcResult<FsReadDirResult>;

    /// Read a utf-8 text file. Binary and over-limit handling are not
    /// yet wired; non-utf-8 content surfaces as `InvalidArgument`.
    async fn fs_read_file(&self, args: FsReadFileArgs) -> RpcResult<FsReadFileResult>;

    /// Write a utf-8 text file. Parent directories must already exist
    /// — `fs_write_file` is for editor saves on known paths, not for
    /// scaffolding new project layouts (that surface arrives with the
    /// explorer's "new file" affordance and gets its own method).
    async fn fs_write_file(&self, args: FsWriteFileArgs) -> RpcResult<()>;

    /// Stat a single path. Missing paths return `kind: None` rather
    /// than `NotFound` so callers can probe existence without catching
    /// errors.
    async fn fs_stat(&self, args: FsStatArgs) -> RpcResult<FsStatResult>;

    /// Report the absolute server root the `fs_*` methods are scoped
    /// under. Returns `Internal` when no filesystem adapter is
    /// configured — same shape as the other `fs_*` methods when the
    /// runtime was built without `with_fs`.
    async fn fs_cwd(&self) -> RpcResult<FsCwdResult>;

    /// List the user-authored files under
    /// `<repo>/.codeless/jobs/<template.name>/`. The result also
    /// carries a `layout` marker that the UI surfaces as the
    /// legacy-flat hint — see `DOCS/JOB-DIR.md` "Layout marker".
    /// Errors with `InvalidArgument` for non-template jobs (those
    /// without a parseable `template_yaml`) and `NotFound` for an
    /// unknown `job_id`.
    async fn list_job_files(&self, args: ListJobFilesArgs) -> RpcResult<ListJobFilesResult>;

    /// Read one file from the job directory by basename. Errors with
    /// `InvalidArgument` if the filename fails the sanitiser (path
    /// traversal, dotfile), `NotFound` if the file does not exist.
    async fn read_job_file(&self, args: ReadJobFileArgs) -> RpcResult<ReadJobFileResult>;

    /// Create or overwrite one file in the job directory.
    /// `template.yaml` is reserved — callers use `update_job_template`
    /// for the spec. The first write against a legacy-flat job
    /// promotes it to the directory layout in separate commits so the
    /// migration shows up in `git log`. The returned `name` is the
    /// normalised filename (`design` → `design.md`).
    async fn write_job_file(&self, args: WriteJobFileArgs) -> RpcResult<WriteJobFileResult>;

    /// Delete one file from the job directory. `template.yaml` is
    /// reserved. `NotFound` is returned for an absent file so the UI
    /// can distinguish "you already deleted this" from a sanitiser
    /// rejection.
    async fn delete_job_file(&self, args: DeleteJobFileArgs) -> RpcResult<()>;

    /// Replace the job's spec YAML. Validates via `JobTemplate::parse_yaml`
    /// (returns `InvalidArgument` on failure), rejects renames with
    /// `Conflict`, writes `<repo>/.codeless/jobs/<name>/template.yaml`
    /// — migrating from the legacy flat layout when needed — refreshes
    /// the `template_yaml` column on the job row, and commits
    /// `update template: <name>`.
    async fn update_job_template(
        &self,
        args: UpdateJobTemplateArgs,
    ) -> RpcResult<UpdateJobTemplateResult>;

    /// Seed (or replace) the job's handover at
    /// `<worktree>/runs/<job_id>/handover.md`. Returns `Conflict` if
    /// the job has no worktree provisioned yet (runner hasn't run).
    /// `NotFound` for an unknown job id.
    async fn write_handover(&self, args: WriteHandoverArgs) -> RpcResult<WriteHandoverResult>;

    /// Invoke a CLI coder runner for a single chat turn. The call
    /// returns once the runner has been spawned; output streams through
    /// the regular event bus tagged with `args.session_id` as the
    /// envelope `job_id`, so the caller subscribes via
    /// `EventFilter::Job { job_id: session_id }` to receive
    /// `AiToken` / `ToolCall` / `AiMessageComplete` envelopes.
    ///
    /// `InvalidArgument` for an unknown or non-CLI runner id;
    /// `Internal` when the host has no registry configured (the
    /// `Open`-auth test harness paths). The runner itself runs in a
    /// detached task — its success/failure surfaces as events, not as
    /// the RPC result, because the wire shape commits to streaming.
    async fn agent_chat(&self, args: AgentChatArgs) -> RpcResult<AgentChatResult>;

    /// Drop a binary blob into the job worktree's chat-attachments
    /// scratch dir so the next `agent_chat` turn can reference it by
    /// path. The runner runs with the worktree as cwd, so a file
    /// written here is readable by the model without any extra
    /// plumbing. `Conflict` if the job has no worktree yet;
    /// `InvalidArgument` for an undecodable base64 body or a filename
    /// that fails the basename sanitiser.
    async fn upload_chat_attachment(
        &self,
        args: UploadChatAttachmentArgs,
    ) -> RpcResult<UploadChatAttachmentResult>;
}
