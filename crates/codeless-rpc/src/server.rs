use async_trait::async_trait;
use codeless_types::{
    AttachWorkspaceArgs, AttachWorkspaceResult, ChatBinding, ChatMessage, DetachWorkspaceArgs, Job,
    ListChatAdaptersResult, ListRunnersResult, ListWorkspacesResult, Persona, Repo,
    RestartServerArgs, RestartServerResult, Review, SetChatAdapterEnabledArgs,
    SetRunnerEnabledArgs, ValidateChatAdapterSecretsArgs, ValidateChatAdapterSecretsResult,
    ValidateWorkspacePathArgs, ValidateWorkspacePathResult,
};

use crate::error::RpcResult;
use crate::methods::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, AppendAssistantMessageArgs,
    AppendAssistantMessageResult, ApproveReviewArgs, ApproveScopePatchArgs, BindChatThreadArgs,
    CancelAssistantActionArgs, CancelAssistantActionResult, CancelChatTaskArgs, CommentReviewArgs,
    ConfirmAssistantActionArgs, ConfirmAssistantActionResult, CreateAssistantThreadArgs,
    DeleteAssistantThreadArgs, DeleteJobArgs, DeleteJobFileArgs, DeletePersonaArgs,
    DraftJobFromConversationArgs, EditScopePatchArgs, FsCreateDirArgs, FsCreateFileArgs, FsCwdArgs,
    FsCwdResult, FsDeleteArgs, FsMoveArgs, FsReadDirArgs, FsReadDirResult, FsReadFileArgs,
    FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GcWorktreesArgs,
    GcWorktreesResult, GetChatBindingArgs, GetChatBindingResult, GetJobArgs, GetPersonaArgs,
    JobDiffArgs, JobDiffResult, JobReportArgs, JobReportResult, ListAssistantMessagesArgs,
    ListAssistantMessagesResult, ListAssistantThreadsArgs, ListAssistantThreadsResult,
    ListChatBindingsForJobArgs, ListChatBindingsForJobResult, ListJobFilesArgs, ListJobFilesResult,
    ListJobMessagesArgs, ListJobMessagesResult, ListJobsArgs, ListJobsResult, ListPersonasArgs,
    ListPersonasResult, ListProposedPatchesArgs, ListProposedPatchesResult, ListReposResult,
    ListReviewsArgs, ListReviewsResult, ListScheduledPausePointsArgs,
    ListScheduledPausePointsResult, ListStagesArgs, ListStagesResult,
    OverridePreCheckAndResumeArgs, PauseJobArgs, PostJobMessageArgs, ReadJobFileArgs,
    ReadJobFileResult, RejectScopePatchArgs, RemoveRepoArgs, RerunJobArgs, ResetJobArgs,
    ResumeJobArgs, RevertScopePatchArgs, RevertScopePatchResult, ScopePatchActionResult,
    SetJobPolicyArgs, StartJobArgs, StopActiveArgs, StopActiveResult, StopJobArgs, StopReviewArgs,
    SubmitJobArgs, UpdateChatMessageDeliveryArgs, UpdateJobArgs, UpdateJobScopeArgs,
    UpdateJobScopeResult, UpdateJobTemplateArgs, UpdateJobTemplateResult,
    UploadAssistantAttachmentArgs, UploadAssistantAttachmentResult, UploadChatAttachmentArgs,
    UploadChatAttachmentResult, UpsertPersonaArgs, WriteHandoverArgs, WriteHandoverResult,
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

    /// Operator-explicit recovery from a `StopReason::ReviewPreCheck`
    /// failure. Sets the one-shot `precheck_override_once` flag, then
    /// runs the same resume path as `resume_job` (so caps bump,
    /// pending-operator-comment threading, status transition, and
    /// `JobResumed` publish all share one implementation). Refuses an
    /// empty `comment` because the audit signal — and the model
    /// guidance — depends on the operator stating *why* the gate is
    /// being bypassed. `Conflict` if the job is not in a resumable
    /// state; `NotFound` for an unknown id.
    async fn override_pre_check_and_resume(
        &self,
        args: OverridePreCheckAndResumeArgs,
    ) -> RpcResult<Job>;

    /// Manual recovery hatch for jobs the driver loop could not move
    /// out of `Queued` (worktree provisioning kept failing past the
    /// retry budget, runner not enabled on this core, template parse
    /// errors), plus the symmetric escape from `Failed` and `Stopped`
    /// back to an editable `Draft`. Reaps the captured worktree
    /// best-effort, clears `worktree_path` / `stop_reason` / `ended_at`,
    /// and publishes `Event::JobReset`. Refused for `Running`,
    /// `Paused`, `AwaitingReview`, and `Completed` — those are not
    /// stuck-states (use `stop_job` / `pause_job` / `resume_job`
    /// instead). `NotFound` for an unknown id.
    async fn reset_job(&self, args: ResetJobArgs) -> RpcResult<Job>;

    /// List the stages of a job, each enriched with rolled-up
    /// `cost_cents` (sum over the stage's tasks) and a `task_count`.
    /// Returns an empty list when the job has no persisted stages
    /// (pre-recorder jobs, mock jobs without a template). The UI
    /// renders an event-derived fallback view in that case.
    async fn list_stages(&self, args: ListStagesArgs) -> RpcResult<ListStagesResult>;

    /// List the operator-declared pause points for one job, in YAML
    /// order. Read-only surface for the Stage-overview planned-pause
    /// chip and the chat divider label lookup. Returns an empty list
    /// when the job carries no schedule; `NotFound` for an unknown
    /// `job_id`.
    async fn list_scheduled_pause_points(
        &self,
        args: ListScheduledPausePointsArgs,
    ) -> RpcResult<ListScheduledPausePointsResult>;

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
    /// under for `args.repo_id`. Returns `Internal` when no filesystem
    /// adapter is configured — same shape as the other `fs_*` methods
    /// when the runtime was built without `with_fs`. Returns a typed
    /// `NotFound` when the `repo_id` is unknown or no longer attached.
    async fn fs_cwd(&self, args: FsCwdArgs) -> RpcResult<FsCwdResult>;

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

    /// Snapshot every chat-adapter row, ordered by `(kind, instance_id)`
    /// so the Settings → Adapters page renders deterministically. Empty
    /// list before any boot upsert or UI write has landed.
    async fn list_chat_adapters(&self) -> RpcResult<ListChatAdaptersResult>;

    /// Flip the `enabled` bit on one chat-adapter row. Enabling
    /// (`enabled = true`) refuses with `AdapterError::MissingSecrets`
    /// when the required secret keys are absent from the
    /// `SecretStore`, and with `AdapterError::ValidationFailed` when
    /// no prior successful `validate_chat_adapter_secrets` for the
    /// same `(kind, instance_id)` is cached for this server process.
    /// Disabling (`enabled = false`) skips both checks so a broken
    /// adapter can always be turned off. The change is persisted
    /// immediately and arms the row for the next `restart_server`;
    /// stage 2 (hot-reload) is explicitly deferred per
    /// `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry".
    async fn set_chat_adapter_enabled(&self, args: SetChatAdapterEnabledArgs) -> RpcResult<()>;

    /// Dry-run secret check for one chat-adapter instance. The runtime
    /// hits the upstream identity endpoint (Slack `auth.test`, Telegram
    /// `getMe`) under a 5s hard timeout and a 5/s per-`(kind,
    /// instance_id)` token bucket. A successful result is cached
    /// in-process for the lifetime of the server so the matching
    /// `set_chat_adapter_enabled(true)` is accepted; a restart clears
    /// the cache and forces re-validation. Result variants are
    /// observations the UI renders inline — RPC-level errors
    /// (`Internal`, `NotConfigured`) only fire on shape / wiring
    /// failures, never on a credential rejection.
    async fn validate_chat_adapter_secrets(
        &self,
        args: ValidateChatAdapterSecretsArgs,
    ) -> RpcResult<ValidateChatAdapterSecretsResult>;

    /// Snapshot every runner-config row, ordered by `runner_id`. Same
    /// "empty before any boot upsert" contract as
    /// `list_chat_adapters`. The result is the source of truth for the
    /// effective enabled set — the boot-time `--enable-claude` /
    /// `--enable-anthropic` flags upsert these rows before the
    /// `RunnerConfig` is built.
    async fn list_runners(&self) -> RpcResult<ListRunnersResult>;

    /// Flip the `enabled` bit on one runner row. No validation step
    /// (runners are local binaries, not credentialed services); the
    /// change is persisted immediately and arms the row for the next
    /// `restart_server`. Unknown `runner_id` values are accepted —
    /// a future runner crate registers itself the same way without a
    /// schema change.
    async fn set_runner_enabled(&self, args: SetRunnerEnabledArgs) -> RpcResult<()>;

    /// Restart the server so the adapter / runner registry rows take
    /// effect. Three execution contexts share this verb:
    ///
    /// - **Supervised CLI** (`init-session.sh`, systemd, or
    ///   `--respawn-on-exit`): the runtime fires its shutdown signal
    ///   so the parent observes a process exit with sentinel code 75
    ///   `EX_TEMPFAIL` and re-execs the child.
    /// - **Tauri desktop sidecar**: the desktop shell brokers the
    ///   restart; the runtime returns success and emits the shutdown
    ///   signal so the sidecar drops, then the shell respawns it.
    /// - **Bare CLI without a supervisor**: refuses with
    ///   `AdapterError::RestartUnsupervised { hint }` carrying a
    ///   copy-pasteable manual restart command.
    ///
    /// Pre-condition: when `force = false` (the default), the runtime
    /// enumerates every `Running` job, partitions them into
    /// *resumable* (template-driven runner with a recent stage
    /// transition) vs *killed* (PTY-bound runner or stale
    /// checkpoint), and refuses with
    /// `AdapterError::RestartHasRunningJobs { resumable, killed }`
    /// unless the operator opts into `force = true`. The validate
    /// cache is cleared by the restart; see
    /// `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry".
    async fn restart_server(&self, args: RestartServerArgs) -> RpcResult<RestartServerResult>;

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

    /// Snapshot the `personas` table. Built-in rows come first (ordered
    /// by id), then user rows (ordered by `created_at` ascending). The
    /// UI's `ai-agents` KV store mirrors this list — see the
    /// agent-personas job's stage 7. Persona rows are not per-user
    /// scoped (R5 single-tenant trust), so the call carries no filter.
    async fn list_personas(&self, args: ListPersonasArgs) -> RpcResult<ListPersonasResult>;

    /// Read one persona by id. `NotFound` for an unknown id. Used by
    /// callers that already have a single id in hand (per-stage
    /// override resolution, MCP prompt rendering); the cache-mirror
    /// path uses `list_personas` instead.
    async fn get_persona(&self, args: GetPersonaArgs) -> RpcResult<Persona>;

    /// Insert or replace a persona row. `created_at` / `updated_at`
    /// are stamped server-side; the runtime preserves the existing
    /// `built_in` flag for rows already in the table so a user-edited
    /// built-in stays a built-in (and remains undeletable). New rows
    /// land with `built_in = 0`. `InvalidArgument` for an empty id or
    /// for `name` / `instructions` that fail basic non-empty checks.
    async fn upsert_persona(&self, args: UpsertPersonaArgs) -> RpcResult<Persona>;

    /// Hard-delete a user-created persona. Built-in rows return
    /// `Conflict` so the seeded `Coder` / `Architect` / … cannot be
    /// removed by the user; `NotFound` for an unknown id.
    async fn delete_persona(&self, args: DeletePersonaArgs) -> RpcResult<()>;

    /// Approve a proposed scope patch through the UI. Wraps the
    /// `codeless patches approve` workflow and produces a human-authored
    /// commit signed with the repo-local `git config user.{name,email}`
    /// identity plus a `Codeless-Approved-By: ui` trailer. Idempotent:
    /// a second call against an already-resolved patch returns
    /// `ScopePatchActionResult::AlreadyResolved` rather than an error so
    /// a stale UI window can recover its view without a red toast.
    /// Emits `ScopePatchApproved` on the event bus so cross-window
    /// subscribers can invalidate their inbox caches.
    async fn approve_scope_patch(
        &self,
        args: ApproveScopePatchArgs,
    ) -> RpcResult<ScopePatchActionResult>;

    /// Reject a proposed scope patch through the UI. Same idempotence
    /// and identity semantics as `approve_scope_patch`; emits
    /// `ScopePatchRejected` on the event bus.
    async fn reject_scope_patch(
        &self,
        args: RejectScopePatchArgs,
    ) -> RpcResult<ScopePatchActionResult>;

    /// Edit the body of a proposed scope patch in-place. The UI submits
    /// the full rendered proposal block (matching `Proposal::render`'s
    /// output); the runtime re-parses and replaces the queue entry
    /// without producing a commit — the operator typically follows up
    /// with `approve_scope_patch`. Same idempotence semantics as the
    /// resolve RPCs: editing an already-resolved patch returns
    /// `AlreadyResolved` rather than an error.
    async fn edit_scope_patch(&self, args: EditScopePatchArgs)
        -> RpcResult<ScopePatchActionResult>;

    /// Undo a previously-applied approval commit. Runs `git revert
    /// <sha> --no-edit` against the repo's worktree and returns the new
    /// revert commit's SHA. Not idempotent: a second call produces a
    /// further revert. The UI exposes this only from the 10-second
    /// post-approval undo toast (decision OQ#3); reverts beyond that
    /// window happen out-of-band through plain `git`.
    async fn revert_scope_patch(
        &self,
        args: RevertScopePatchArgs,
    ) -> RpcResult<RevertScopePatchResult>;

    /// Snapshot the unresolved scope-patch queue across one or all
    /// repos. Wraps `scope_patch_queue::load_queue` per repo, projects
    /// each entry onto `ProposedScopePatch`, and concatenates the
    /// results sorted newest-first by `proposed_at` (legacy entries
    /// with no timestamp sort last). Powers Surface C — the
    /// cross-workspace patch worklist — and is mobile-safe (the result
    /// DTO lives in `codeless-types`). A missing `DOCS/SCOPE-PROPOSED.md`
    /// on a repo is treated as an empty contribution, not a failure.
    async fn list_proposed_patches(
        &self,
        args: ListProposedPatchesArgs,
    ) -> RpcResult<ListProposedPatchesResult>;

    /// Replace the job-level `auto_bypass_policy`. Refused with
    /// `Conflict` while the row is `Running` or `Queued` so the
    /// stage-failed handler cannot race the write — the operator must
    /// `pause_job` first per `DOCS/AUTO-BYPASS-DECISIONS.md` Q5.
    /// Setting the policy to the value already on the row is a no-op
    /// success: the second call returns `Ok(())` and emits no
    /// `JobPolicyChanged`, so a defensive UI write never floods the
    /// bus. `NotFound` for an unknown job id.
    async fn set_job_policy(&self, args: SetJobPolicyArgs) -> RpcResult<()>;

    /// Append one message to the per-Job chat thread
    /// (`DOCS/JOB-CHAT.md`). Used by the web chat input, the Telegram
    /// and Slack adapters, the CLI, and the supervisor agent — every
    /// voice ends up as one row in `chat_messages` keyed by `job_id`.
    /// The runtime mints the `MessageId` ULID and stamps `created_at`
    /// so a clock-skewed transport cannot reorder the thread.
    ///
    /// Returns `NotFound` for an unknown `job_id`. Returns `Conflict`
    /// when a redelivered inbound on Telegram or Slack collides on the
    /// partial unique index over `(transport, external_id)` — that is
    /// the duplicate-ingest defence and lets the adapter skip the
    /// re-send idempotently.
    async fn post_job_message(&self, args: PostJobMessageArgs) -> RpcResult<ChatMessage>;

    /// Paginate one Job's chat history newest-first by `created_at`
    /// (id as tiebreaker). `before = None` returns the most recent
    /// `limit` rows; pass the oldest returned id back as `before` to
    /// walk further into the past. Returned messages are ordered
    /// oldest-first within the page so a UI renders top-to-bottom
    /// without sorting. `limit` is capped server-side so a runaway
    /// caller cannot pull the whole table. `NotFound` for an unknown
    /// `job_id`.
    async fn list_job_messages(
        &self,
        args: ListJobMessagesArgs,
    ) -> RpcResult<ListJobMessagesResult>;

    /// Bind `(transport, channel_id, thread_id)` to a Job so the
    /// adapter's inbound path can resolve an arriving message to the
    /// right chat thread. Idempotent on the PK: a second bind of the
    /// same key to the same Job returns the existing row stamped with
    /// the new `bound_at` / `bound_by`. Re-pointing the same
    /// `(transport, channel, thread)` at a different Job overwrites —
    /// the row's PK guarantees only one Job can own a given channel
    /// thread at a time. `NotFound` for an unknown `job_id`.
    async fn bind_chat_thread(&self, args: BindChatThreadArgs) -> RpcResult<ChatBinding>;

    /// Record one transport's outbound delivery receipt against a
    /// `chat_messages` row. The originating columns (`body`,
    /// `external_id`) are never touched — only
    /// `metadata_json.delivery.<transport>` is set to `platform_id` so
    /// the outbound forwarder can presence-check the receipt on restart
    /// and skip a duplicate send (`JOB-CHAT.md` "Transport adapters"
    /// idempotency rule). `NotFound` for a message id that no longer
    /// exists (the row was deleted between the original append and the
    /// delivery write — treated as already-handled by the forwarder).
    async fn update_chat_message_delivery(
        &self,
        args: UpdateChatMessageDeliveryArgs,
    ) -> RpcResult<ChatMessage>;

    /// Reverse `chat_bindings` lookup: every `(channel, thread)` on
    /// the given transport that points at this Job. The outbound
    /// forwarders on each transport adapter call this when a
    /// `ChatMessageAppended` fires so they know where to fan the
    /// message out. Empty result is the normal "no binding yet" path
    /// (the Job has never been `/codeless bind`-ed on this transport);
    /// `NotFound` is reserved for an unknown `job_id`.
    async fn list_chat_bindings_for_job(
        &self,
        args: ListChatBindingsForJobArgs,
    ) -> RpcResult<ListChatBindingsForJobResult>;

    /// Forward lookup of `chat_bindings`: the Job (if any) that owns
    /// the conversation on `(transport, channel, thread)`. Returns
    /// `binding = None` when the channel was never `/codeless bind`-
    /// ed; transport adapters treat that as "drop the message" so the
    /// substrate refuses to ingest text the operator has not pointed
    /// at a Job.
    async fn get_chat_binding(&self, args: GetChatBindingArgs) -> RpcResult<GetChatBindingResult>;
}
