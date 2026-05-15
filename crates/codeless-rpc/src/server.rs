use async_trait::async_trait;
use codeless_types::{
    AttachWorkspaceArgs, AttachWorkspaceResult, DetachWorkspaceArgs, Job, ListWorkspacesResult,
    Repo, Review, ValidateWorkspacePathArgs, ValidateWorkspacePathResult,
};

use crate::error::RpcResult;
use crate::methods::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, AppendAssistantMessageArgs,
    AppendAssistantMessageResult, ApproveReviewArgs, CancelAssistantActionArgs,
    CancelAssistantActionResult, CancelChatTaskArgs, CommentReviewArgs, ConfirmAssistantActionArgs,
    ConfirmAssistantActionResult, CreateAssistantThreadArgs, DeleteAssistantThreadArgs,
    DeleteJobArgs, DeleteJobFileArgs, DraftJobFromConversationArgs, FsCreateDirArgs,
    FsCreateFileArgs, FsCwdResult, FsDeleteArgs, FsMoveArgs, FsReadDirArgs, FsReadDirResult,
    FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GcWorktreesArgs,
    GcWorktreesResult, GetJobArgs, JobDiffArgs, JobDiffResult, JobReportArgs, JobReportResult,
    ListAssistantMessagesArgs, ListAssistantMessagesResult, ListAssistantThreadsArgs,
    ListAssistantThreadsResult, ListJobFilesArgs, ListJobFilesResult, ListJobsArgs, ListJobsResult,
    ListReposResult, ListReviewsArgs, ListReviewsResult, ListStagesArgs, ListStagesResult,
    PauseJobArgs, ReadJobFileArgs, ReadJobFileResult, RemoveRepoArgs, RerunJobArgs, ResumeJobArgs,
    StartJobArgs, StopActiveArgs, StopActiveResult, StopJobArgs, StopReviewArgs, SubmitJobArgs,
    UpdateJobArgs, UpdateJobScopeArgs, UpdateJobScopeResult, UpdateJobTemplateArgs,
    UpdateJobTemplateResult, UploadAssistantAttachmentArgs, UploadAssistantAttachmentResult,
    UploadChatAttachmentArgs, UploadChatAttachmentResult, WriteHandoverArgs, WriteHandoverResult,
    WriteJobFileArgs, WriteJobFileResult,
};
use crate::subscribe::{EventFilter, EventStream, Since};
use codeless_types::AssistantThread;

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

    /// Patch mutable fields on a job that is not currently running.
    /// Only `Draft` and terminal states (`Stopped`, `Failed`,
    /// `Completed`) are editable. Errors with `Conflict` if the job
    /// is `Running`, `Queued`, `Paused`, or `AwaitingReview`.
    async fn update_job(&self, args: UpdateJobArgs) -> RpcResult<Job>;

    /// Hard-delete a job row and all associated events, stages, and
    /// tasks. The on-disk job directory (under `.codeless/jobs/`) is
    /// left intact so the user can recover files manually. Errors
    /// with `Conflict` if the job is `Running` or `Queued` — stop
    /// it first.
    async fn delete_job(&self, args: DeleteJobArgs) -> RpcResult<()>;

    /// Pause a `Running` (or `AwaitingReview`) job so the user can
    /// resume it later from the captured `Stage.session_id`. The
    /// runner is cancelled at the next `await` boundary; the row
    /// transitions to `Paused`, not `Stopped`. Distinct from
    /// `stop_job` because the *intent* differs — pause is
    /// "I'll come back," stop is "I'm done." See SCOPE.md hard
    /// rule #1.
    async fn pause_job(&self, args: PauseJobArgs) -> RpcResult<()>;

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

    /// Structured cost / session / activity report for one job.
    /// Aggregates stage rows + `ai-message-complete` and `tool-call`
    /// events into a single response so the UI's Summary tab can
    /// render the full picture in one round trip. Works mid-run
    /// (snapshot of state-so-far) and post-run. Returns `NotFound`
    /// for an unknown job id.
    async fn job_report(&self, args: JobReportArgs) -> RpcResult<JobReportResult>;

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

    /// Create a file. Empty content when `content` is null. Rejects
    /// with `Conflict` when `overwrite` is false and the path exists.
    async fn fs_create_file(&self, args: FsCreateFileArgs) -> RpcResult<()>;

    /// Create a directory. When `recursive` is true, missing ancestors
    /// are created.
    async fn fs_create_dir(&self, args: FsCreateDirArgs) -> RpcResult<()>;

    /// Move (rename) a path within the sandbox. Rejects with
    /// `Conflict` when `overwrite` is false and the target exists.
    async fn fs_move(&self, args: FsMoveArgs) -> RpcResult<()>;

    /// Delete a file or directory. When `recursive` is true,
    /// directories are removed with all contents.
    async fn fs_delete(&self, args: FsDeleteArgs) -> RpcResult<()>;

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

    /// Fire the cancellation token for an in-flight `agent_chat` turn
    /// so the underlying CLI runner exits at its next `await`. The
    /// chat-cancel registry is in-memory, single-tenant, and lives on
    /// the runtime; entries are removed automatically when the spawned
    /// task completes, so calling this against an already-finished
    /// task returns `Ok(())` rather than `NotFound`. The UI relies on
    /// that idempotence to race the natural end of the stream.
    async fn cancel_chat_task(&self, args: CancelChatTaskArgs) -> RpcResult<()>;

    /// Stop *whatever* is running for `job_id`. Composes `stop_job`
    /// (when the row is `Running` / `AwaitingReview` / `Queued`) with
    /// `cancel_chat_task` against every in-flight chat turn whose
    /// `session_id` is this job, returning a structured summary so
    /// the UI can show "stopped the chat", "stopped the job", or
    /// both. Idempotent — neither path firing is `Ok(())` with both
    /// fields zeroed. The UI's unified stop button calls this
    /// instead of `stop_job` so it works on chat-turns over a
    /// terminal job too.
    async fn stop_active(&self, args: StopActiveArgs) -> RpcResult<StopActiveResult>;

    /// Register a `repos` row as an editor-side workspace: persist a
    /// row in `attached_workspaces` keyed by the canonical filesystem
    /// path so the `fs.*` surface accepts paths under it and the UI
    /// surfaces it in the workspaces sidebar. Idempotent on the
    /// canonical path; a second call with the same effective root
    /// returns `RpcError::Workspace(WorkspaceError::AlreadyAttached)`
    /// so the UI can present a clear "already attached" rather than a
    /// generic Conflict. Path-shape problems (system path, dotfile
    /// escape, missing dir) surface as
    /// `WorkspaceError::PathRejected { problems }`.
    async fn attach_workspace(&self, args: AttachWorkspaceArgs)
        -> RpcResult<AttachWorkspaceResult>;

    /// Drop the `attached_workspaces` row for `repo_id`. The `repos`
    /// row is left in place — destructive removal is `remove_repo`.
    /// `DetachPolicy::Refuse` (the default) returns
    /// `WorkspaceError::RunningJobs` with the in-flight job ids so the
    /// UI can prompt; `Stop` cancels them first; `LeaveRunning`
    /// detaches the editor surface without touching runner worktrees.
    /// An unknown repo or one with no attachment row returns
    /// `WorkspaceError::NotAttached`.
    async fn detach_workspace(&self, args: DetachWorkspaceArgs) -> RpcResult<()>;

    /// Snapshot the live attached set. Ordered by `attached_at` so the
    /// sidebar renders in the order the operator attached. Returns an
    /// empty list when no `--fs-root` was passed and no UI-driven
    /// attach has happened yet.
    async fn list_workspaces(&self) -> RpcResult<ListWorkspacesResult>;

    /// Dry-run validation for the workspace picker: canonicalise the
    /// path, surface every per-row problem the UI needs to render
    /// inline (system path, not a directory, not a git repo, already
    /// attached, ...). The call never mutates state; the result is a
    /// `WorkspaceProblem` list rather than a hard error so the picker
    /// can render the row even when the path is unusable.
    async fn validate_workspace_path(
        &self,
        args: ValidateWorkspacePathArgs,
    ) -> RpcResult<ValidateWorkspacePathResult>;

    /// List every assistant thread on this host, newest-touched first.
    /// Unfiltered by design — see `AssistantThread`: threads are not
    /// scoped to a repo or job. Empty list when the operator has never
    /// opened the assistant surface.
    async fn list_assistant_threads(
        &self,
        args: ListAssistantThreadsArgs,
    ) -> RpcResult<ListAssistantThreadsResult>;

    /// Mint a new assistant thread row. The returned `AssistantThread`
    /// carries the freshly-minted ULID id and `created_at` / `updated_at`
    /// stamped at the same instant; the UI uses the id to select the
    /// thread in the rail without a round-trip back to `list_assistant_threads`.
    async fn create_assistant_thread(
        &self,
        args: CreateAssistantThreadArgs,
    ) -> RpcResult<AssistantThread>;

    /// Delete an assistant thread row and cascade through its messages
    /// and attachments. The on-disk attachments directory is removed
    /// in the same call so blobs do not outlive the row. `NotFound`
    /// for an unknown id — the UI surfaces this as "already deleted".
    async fn delete_assistant_thread(&self, args: DeleteAssistantThreadArgs) -> RpcResult<()>;

    /// Upload a binary blob into an assistant thread's attachments
    /// directory. The runtime decodes the base64 body, writes the file
    /// under `<codeless-data>/threads/<thread_id>/attachments/`, and
    /// inserts the index row. Returns the persisted
    /// `AssistantAttachment` so the UI can render the entry without a
    /// follow-up list.
    async fn upload_assistant_attachment(
        &self,
        args: UploadAssistantAttachmentArgs,
    ) -> RpcResult<UploadAssistantAttachmentResult>;

    /// Read every persisted turn for one thread. Returned messages are
    /// ordered by `created_at` ascending so the UI renders the conversation
    /// top-to-bottom without sorting. An unknown thread returns an empty
    /// list rather than `NotFound` — the rail row that pointed at it is
    /// authoritative for "does this thread exist", and the chat view's
    /// only sensible response to a missing thread is the same empty
    /// canvas it shows on a freshly-minted one.
    async fn list_assistant_messages(
        &self,
        args: ListAssistantMessagesArgs,
    ) -> RpcResult<ListAssistantMessagesResult>;

    /// Persist the user's turn and synthesise an assistant response in
    /// the same call. The thread's `updated_at` is bumped so the rail
    /// re-sorts to put the conversation at the top. The stage-6
    /// responder is a fixed no-op acknowledgement so the surface is
    /// end-to-end testable before the real planner / tool loop lands;
    /// later stages swap the body without changing this wire shape.
    async fn append_assistant_message(
        &self,
        args: AppendAssistantMessageArgs,
    ) -> RpcResult<AppendAssistantMessageResult>;

    /// Dispatch the tool call carried on a pending action card. The
    /// runtime executes the underlying `RpcServer` method, flips the
    /// proposal's `meta_json.status` to `Confirmed` or `Failed`, and
    /// appends a `Tool`-role message carrying the structured result.
    /// Capabilities are derived **server-side** from the proposal row,
    /// never trusted from the client (see SCOPE.md — the `kind` prop on
    /// `CommonChat` is UI-only).
    ///
    /// `NotFound` for an unknown message; `InvalidArgument` when the
    /// row is not an action card or is no longer pending; downstream
    /// errors (`Conflict`, `NotFound`, …) propagate after the status
    /// flip so the UI can render the failure inline.
    async fn confirm_assistant_action(
        &self,
        args: ConfirmAssistantActionArgs,
    ) -> RpcResult<ConfirmAssistantActionResult>;

    /// Decline a pending action card. Flips `meta_json.status` to
    /// `Cancelled` and does nothing else — no RPC is dispatched, no
    /// trailing `Tool` message is appended. The card row stays in the
    /// transcript as a record of the declined proposal.
    async fn cancel_assistant_action(
        &self,
        args: CancelAssistantActionArgs,
    ) -> RpcResult<CancelAssistantActionResult>;

    /// Rewrite a job's `SCOPE.md` from chat. Rejects with `Conflict`
    /// when the job is `Running`, `Queued`, or `AwaitingReview` so the
    /// runner is not racing the user's spec edit (the assistant
    /// surface's "pause first" affordance is the recovery path). Other
    /// states — `Draft`, `Paused`, and the terminal trio — go through
    /// the same commit pipeline as the Spec pane's save.
    /// `NotFound` for an unknown job; `InvalidArgument` for an empty
    /// body so a missed-`--` slash command can't silently clobber the
    /// file with whitespace.
    async fn update_job_scope(&self, args: UpdateJobScopeArgs) -> RpcResult<UpdateJobScopeResult>;

    /// Materialise a fresh `Draft` job from the most recent pending
    /// `DraftJob` action card on the given assistant thread. The
    /// proposal lives on an `Assistant`-role message's `meta_json`;
    /// this RPC walks the transcript newest-to-oldest, decodes the
    /// card, and forwards the captured fields into `submit_job` with
    /// `start_immediately = false`. `NotFound` for an unknown thread;
    /// `InvalidArgument` when no pending `DraftJob` card is present so
    /// the UI can guide the user to issue `/draft` first.
    async fn draft_job_from_conversation(
        &self,
        args: DraftJobFromConversationArgs,
    ) -> RpcResult<codeless_types::Job>;
}
