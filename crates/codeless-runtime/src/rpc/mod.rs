use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use codeless_adapters_host::{HostFs, WorktreeManager};
use codeless_rpc::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, AppendAssistantMessageArgs,
    AppendAssistantMessageResult, ApproveReviewArgs, CancelAssistantActionArgs,
    CancelAssistantActionResult, CancelChatTaskArgs, CommentReviewArgs, ConfirmAssistantActionArgs,
    ConfirmAssistantActionResult, CreateAssistantThreadArgs, DeleteAssistantThreadArgs,
    DeleteJobFileArgs, DeletePersonaArgs, DraftJobFromConversationArgs, EventFilter, EventStream,
    FsCreateDirArgs, FsCreateFileArgs, FsCwdResult, FsDeleteArgs, FsMoveArgs, FsReadDirArgs,
    FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs,
    GcWorktreesArgs, GcWorktreesResult, GetJobArgs, GetPersonaArgs, JobDiffArgs, JobDiffResult,
    JobReportArgs, ListAssistantMessagesArgs, ListAssistantMessagesResult,
    ListAssistantThreadsArgs, ListAssistantThreadsResult, ListJobFilesArgs, ListJobFilesResult,
    ListJobsArgs, ListJobsResult, ListPersonasArgs, ListPersonasResult, ListReposResult,
    ListReviewsArgs, ListReviewsResult, ListStagesArgs, ListStagesResult,
    OverridePreCheckAndResumeArgs, PauseJobArgs, ReadJobFileArgs, ReadJobFileResult,
    RemoveRepoArgs, RerunJobArgs, ResetJobArgs, ResumeJobArgs, RpcError, RpcResult, RpcServer,
    SetJobPolicyArgs, Since, StartJobArgs, StopActiveArgs, StopActiveResult, StopJobArgs,
    StopReviewArgs, SubmitJobArgs, UpdateJobScopeArgs, UpdateJobScopeResult, UpdateJobTemplateArgs,
    UpdateJobTemplateResult, UploadAssistantAttachmentArgs, UploadAssistantAttachmentResult,
    UploadChatAttachmentArgs, UploadChatAttachmentResult, UpsertPersonaArgs, WriteHandoverArgs,
    WriteHandoverResult, WriteJobFileArgs, WriteJobFileResult,
};
use codeless_types::{AssistantThread, Job, Persona, Repo, Review, TaskId};
use sqlx::SqlitePool;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::migrations::MIGRATOR;
use crate::store::SqliteStore;
use crate::time::now_ms;

pub(crate) mod assistant;
pub(crate) mod assistant_planner;
pub(crate) mod chat;
pub(crate) mod fs;
pub(crate) mod job_files;
pub(crate) mod jobs;
pub(crate) mod personas;
pub(crate) mod repos;
pub(crate) mod reviews;
pub(crate) mod scope_patches;
pub(crate) mod workspaces;

/// In-process `RpcServer`. The CLI's `codeless run --once` path talks
/// to this directly without serialising over a wire; the hosted server
/// hands the same struct to `axum` handlers. Repo, job, and event rows
/// are persisted in SQLite via `SqliteStore` and `EventBus`; the
/// broadcast channel inside the bus is the live fan-out for in-process
/// subscribers and the catch-up path goes through the events table.
pub struct InProcessRpc {
    pub(crate) store: Arc<SqliteStore>,
    pub(crate) bus: Arc<EventBus>,
    /// Optional filesystem adapter. When `None`, every `fs_*` method
    /// returns `Internal` so callers see a typed failure rather than a
    /// panic — this is the path tests and CLI-local mode take when no
    /// workspace root is configured.
    pub(crate) fs: Option<Arc<HostFs>>,
    /// Optional worktree manager. `None` matches a serve mode booted
    /// without `--worktree-root`; in that mode `gc_worktrees` answers
    /// `Internal` so the UI can show a clear "no root configured"
    /// message rather than a misleading empty sweep.
    pub(crate) worktrees: Option<Arc<WorktreeManager>>,
    /// Optional CLI-runner registry that powers `agent_chat`. `None`
    /// means the footer agent panel is not wired — calls return
    /// `Internal`. Tests construct the runtime without it; the hosted
    /// server installs `Registry::with_defaults()` once at boot.
    pub(crate) agent_chat_registry: Option<Arc<ai_runner::Registry>>,
    /// Working directory CLI runners are invoked in for a chat turn.
    /// Defaults to the process cwd at runtime construction time.
    pub(crate) agent_chat_cwd: Option<std::path::PathBuf>,
    /// Cancellation tokens for in-flight `agent_chat` turns, keyed by
    /// the per-turn `TaskId`. The `agent_chat` spawn inserts the entry
    /// before launching the runner; a drop-guard removes it when the
    /// task completes. `cancel_chat_task` fires one token;
    /// `stop_active` fires all tokens whose `job_id` matches. Held
    /// behind a `parking_lot::Mutex` because every access is a brief
    /// map operation — never crosses an `await`.
    pub(crate) chat_cancels: ChatCancels,
    /// Filesystem root the assistant surface writes attachments under
    /// (per `ASSISTANT-SCOPE.md` Decisions §1). Threads land in
    /// `<root>/threads/<thread_id>/attachments/`. `None` matches a
    /// runtime built without `with_assistant_data_dir`; in that mode
    /// `upload_assistant_attachment` returns `Internal` so the UI
    /// surfaces a typed error rather than a panic.
    pub(crate) assistant_data_dir: Option<std::path::PathBuf>,
}

/// One entry in the chat-cancel registry. `job_id` is the
/// `agent_chat` `session_id` the caller passed in; for the per-job
/// chat panel the UI uses the live `JobId` as the session, which is
/// what makes `stop_active(job_id)` able to fan out to the chat
/// turn(s) scoped to that job.
pub struct ChatCancelEntry {
    pub job_id: codeless_types::JobId,
    pub token: tokio_util::sync::CancellationToken,
}

/// In-memory, single-tenant registry of cancellation tokens for
/// in-flight `agent_chat` turns. See `InProcessRpc::chat_cancels`.
pub type ChatCancels = Arc<parking_lot::Mutex<HashMap<TaskId, ChatCancelEntry>>>;

const DEFAULT_EVENT_BUFFER: usize = 1024;

impl InProcessRpc {
    /// Shortcut for tests: a fresh `sqlite::memory:` pool with
    /// migrations applied. sqlx's pool keeps a single dedicated
    /// connection alive for `:memory:` so successive queries see the
    /// same data.
    pub async fn new() -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        Self::with_db(pool).await
    }

    /// Open the runtime against a file-backed SQLite database. The
    /// file is created if missing so a first-run CLI invocation does
    /// not have to bootstrap state by hand.
    pub async fn with_file(path: &std::path::Path) -> Result<Self, sqlx::Error> {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        Self::with_db(pool).await
    }

    /// Build a runtime around a caller-supplied pool. Migrations are
    /// applied here so a fresh database file works the same as a
    /// pre-migrated one and the caller never has to run the migrator
    /// separately.
    pub async fn with_db(pool: SqlitePool) -> Result<Self, sqlx::Error> {
        MIGRATOR.run(&pool).await?;
        let bus = Arc::new(EventBus::new(pool.clone(), DEFAULT_EVENT_BUFFER));
        let store = Arc::new(SqliteStore::new(pool));
        // Startup reaper: any task whose lease expired before the
        // previous core died is still marked `running` in the DB.
        // Returning it to `enqueued` lets the new core re-pick it up.
        let reclaimed = store.release_expired_leases(now_ms()).await?;
        if reclaimed > 0 {
            tracing::info!(reclaimed, "released expired task leases at startup");
        }
        Ok(Self {
            store,
            bus,
            fs: None,
            worktrees: None,
            agent_chat_registry: None,
            agent_chat_cwd: None,
            chat_cancels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            assistant_data_dir: None,
        })
    }

    /// Attach a filesystem adapter so the `fs_*` RPC surface becomes
    /// live.
    pub fn with_fs(mut self, fs: Arc<HostFs>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Attach the worktree manager so the `gc_worktrees` RPC can see
    /// the on-disk state. Same Arc the driver uses — they read and
    /// write the same `<base>/job-<id>` tree from different sides.
    pub fn with_worktrees(mut self, worktrees: Arc<WorktreeManager>) -> Self {
        self.worktrees = Some(worktrees);
        self
    }

    /// Wire the CLI-runner registry the footer agent panel dispatches
    /// onto. `cwd` is the directory each chat turn runs in; passing
    /// the operator's launch cwd means a quickstart `codeless serve`
    /// "just works" without an extra flag.
    pub fn with_agent_chat(
        mut self,
        registry: Arc<ai_runner::Registry>,
        cwd: std::path::PathBuf,
    ) -> Self {
        self.agent_chat_registry = Some(registry);
        self.agent_chat_cwd = Some(cwd);
        self
    }

    /// Configure the filesystem root the `assistant.*` RPCs write
    /// attachments under. The caller (the CLI / desktop shell) decides
    /// the actual location — typically the same `<codeless-data>` root
    /// the SQLite file lives in. Without this, attachment uploads
    /// return `Internal`; threads still create and delete.
    pub fn with_assistant_data_dir(mut self, root: std::path::PathBuf) -> Self {
        self.assistant_data_dir = Some(root);
        self
    }

    pub fn store(&self) -> &Arc<SqliteStore> {
        &self.store
    }

    pub fn bus(&self) -> &Arc<EventBus> {
        &self.bus
    }

    /// Pool handle for callers that need to issue ad-hoc queries
    /// against the same database the store is writing to.
    pub fn pool(&self) -> &SqlitePool {
        self.store.pool()
    }

    /// Direct access to the chat-cancel registry. Exposed for tests
    /// that drive `cancel_chat_task` against a synthetic token without
    /// having to spawn a real CLI runner.
    pub fn chat_cancels(&self) -> &ChatCancels {
        &self.chat_cancels
    }
}

pub(super) fn db_err(e: sqlx::Error) -> RpcError {
    RpcError::Internal(format!("db: {e}"))
}

#[async_trait]
impl RpcServer for InProcessRpc {
    async fn add_repo(&self, args: AddRepoArgs) -> RpcResult<Repo> {
        repos::add_repo(self, args).await
    }

    async fn remove_repo(&self, args: RemoveRepoArgs) -> RpcResult<()> {
        repos::remove_repo(self, args).await
    }

    async fn list_repos(&self) -> RpcResult<ListReposResult> {
        repos::list_repos(self).await
    }

    async fn submit_job(&self, args: SubmitJobArgs) -> RpcResult<Job> {
        jobs::submit_job(self, args).await
    }

    async fn start_job(&self, args: StartJobArgs) -> RpcResult<Job> {
        jobs::start_job(self, args).await
    }

    async fn resume_job(&self, args: ResumeJobArgs) -> RpcResult<Job> {
        jobs::resume_job(self, args).await
    }

    async fn override_pre_check_and_resume(
        &self,
        args: OverridePreCheckAndResumeArgs,
    ) -> RpcResult<Job> {
        jobs::override_pre_check_and_resume(self, args).await
    }

    async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job> {
        jobs::get_job(self, args).await
    }

    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult> {
        jobs::list_jobs(self, args).await
    }

    async fn list_stages(&self, args: ListStagesArgs) -> RpcResult<ListStagesResult> {
        jobs::list_stages(self, args).await
    }

    async fn job_report(&self, args: JobReportArgs) -> RpcResult<codeless_rpc::JobReportResult> {
        jobs::job_report(self, args).await
    }

    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()> {
        jobs::stop_job(self, args).await
    }

    async fn update_job(&self, args: codeless_rpc::UpdateJobArgs) -> RpcResult<Job> {
        jobs::update_job_fields(self, args).await
    }

    async fn delete_job(&self, args: codeless_rpc::DeleteJobArgs) -> RpcResult<()> {
        jobs::delete_job(self, args).await
    }

    async fn pause_job(&self, args: PauseJobArgs) -> RpcResult<()> {
        jobs::pause_job(self, args).await
    }

    async fn rerun_job(&self, args: RerunJobArgs) -> RpcResult<Job> {
        jobs::rerun_job(self, args).await
    }

    async fn reset_job(&self, args: ResetJobArgs) -> RpcResult<Job> {
        jobs::reset_job(self, args).await
    }

    async fn gc_worktrees(&self, args: GcWorktreesArgs) -> RpcResult<GcWorktreesResult> {
        jobs::gc_worktrees(self, args).await
    }

    async fn job_diff(&self, args: JobDiffArgs) -> RpcResult<JobDiffResult> {
        job_files::job_diff(self, args).await
    }

    async fn list_reviews(&self, args: ListReviewsArgs) -> RpcResult<ListReviewsResult> {
        reviews::list_reviews(self, args).await
    }

    async fn approve_review(&self, args: ApproveReviewArgs) -> RpcResult<Review> {
        reviews::approve_review(self, args).await
    }

    async fn comment_review(&self, args: CommentReviewArgs) -> RpcResult<Review> {
        reviews::comment_review(self, args).await
    }

    async fn stop_review(&self, args: StopReviewArgs) -> RpcResult<Review> {
        reviews::stop_review(self, args).await
    }

    async fn subscribe(&self, filter: EventFilter, since: Since) -> RpcResult<EventStream> {
        let local = match filter {
            EventFilter::All => SubscribeFilter::All,
            EventFilter::Job { job_id } => SubscribeFilter::Job(job_id),
        };
        self.bus.subscribe_since(local, since).await.map_err(db_err)
    }

    async fn fs_read_dir(&self, args: FsReadDirArgs) -> RpcResult<FsReadDirResult> {
        fs::fs_read_dir(self, args).await
    }

    async fn fs_read_file(&self, args: FsReadFileArgs) -> RpcResult<FsReadFileResult> {
        fs::fs_read_file(self, args).await
    }

    async fn fs_write_file(&self, args: FsWriteFileArgs) -> RpcResult<()> {
        fs::fs_write_file(self, args).await
    }

    async fn fs_stat(&self, args: FsStatArgs) -> RpcResult<FsStatResult> {
        fs::fs_stat(self, args).await
    }

    async fn fs_cwd(&self) -> RpcResult<FsCwdResult> {
        fs::fs_cwd(self).await
    }

    async fn fs_create_file(&self, args: FsCreateFileArgs) -> RpcResult<()> {
        fs::fs_create_file(self, args).await
    }

    async fn fs_create_dir(&self, args: FsCreateDirArgs) -> RpcResult<()> {
        fs::fs_create_dir(self, args).await
    }

    async fn fs_move(&self, args: FsMoveArgs) -> RpcResult<()> {
        fs::fs_move(self, args).await
    }

    async fn fs_delete(&self, args: FsDeleteArgs) -> RpcResult<()> {
        fs::fs_delete(self, args).await
    }

    async fn list_job_files(&self, args: ListJobFilesArgs) -> RpcResult<ListJobFilesResult> {
        job_files::list_job_files(self, args).await
    }

    async fn read_job_file(&self, args: ReadJobFileArgs) -> RpcResult<ReadJobFileResult> {
        job_files::read_job_file(self, args).await
    }

    async fn write_job_file(&self, args: WriteJobFileArgs) -> RpcResult<WriteJobFileResult> {
        job_files::write_job_file(self, args).await
    }

    async fn delete_job_file(&self, args: DeleteJobFileArgs) -> RpcResult<()> {
        job_files::delete_job_file(self, args).await
    }

    async fn update_job_template(
        &self,
        args: UpdateJobTemplateArgs,
    ) -> RpcResult<UpdateJobTemplateResult> {
        job_files::update_job_template(self, args).await
    }

    async fn write_handover(&self, args: WriteHandoverArgs) -> RpcResult<WriteHandoverResult> {
        job_files::write_handover(self, args).await
    }

    async fn agent_chat(&self, args: AgentChatArgs) -> RpcResult<AgentChatResult> {
        chat::agent_chat(self, args).await
    }

    async fn upload_chat_attachment(
        &self,
        args: UploadChatAttachmentArgs,
    ) -> RpcResult<UploadChatAttachmentResult> {
        chat::upload_chat_attachment(self, args).await
    }

    async fn cancel_chat_task(&self, args: CancelChatTaskArgs) -> RpcResult<()> {
        chat::cancel_chat_task(self, args).await
    }

    async fn stop_active(&self, args: StopActiveArgs) -> RpcResult<StopActiveResult> {
        chat::stop_active(self, args).await
    }

    async fn attach_workspace(
        &self,
        args: codeless_rpc::AttachWorkspaceArgs,
    ) -> RpcResult<codeless_rpc::AttachWorkspaceResult> {
        workspaces::attach_workspace(self, args).await
    }

    async fn detach_workspace(&self, args: codeless_rpc::DetachWorkspaceArgs) -> RpcResult<()> {
        workspaces::detach_workspace(self, args).await
    }

    async fn list_workspaces(&self) -> RpcResult<codeless_rpc::ListWorkspacesResult> {
        workspaces::list_workspaces(self).await
    }

    async fn validate_workspace_path(
        &self,
        args: codeless_rpc::ValidateWorkspacePathArgs,
    ) -> RpcResult<codeless_rpc::ValidateWorkspacePathResult> {
        workspaces::validate_workspace_path(self, args).await
    }

    async fn list_assistant_threads(
        &self,
        args: ListAssistantThreadsArgs,
    ) -> RpcResult<ListAssistantThreadsResult> {
        assistant::list_assistant_threads(self, args).await
    }

    async fn create_assistant_thread(
        &self,
        args: CreateAssistantThreadArgs,
    ) -> RpcResult<AssistantThread> {
        assistant::create_assistant_thread(self, args).await
    }

    async fn delete_assistant_thread(&self, args: DeleteAssistantThreadArgs) -> RpcResult<()> {
        assistant::delete_assistant_thread(self, args).await
    }

    async fn upload_assistant_attachment(
        &self,
        args: UploadAssistantAttachmentArgs,
    ) -> RpcResult<UploadAssistantAttachmentResult> {
        assistant::upload_assistant_attachment(self, args).await
    }

    async fn list_assistant_messages(
        &self,
        args: ListAssistantMessagesArgs,
    ) -> RpcResult<ListAssistantMessagesResult> {
        assistant::list_assistant_messages(self, args).await
    }

    async fn append_assistant_message(
        &self,
        args: AppendAssistantMessageArgs,
    ) -> RpcResult<AppendAssistantMessageResult> {
        assistant::append_assistant_message(self, args).await
    }

    async fn confirm_assistant_action(
        &self,
        args: ConfirmAssistantActionArgs,
    ) -> RpcResult<ConfirmAssistantActionResult> {
        assistant::confirm_assistant_action(self, args).await
    }

    async fn cancel_assistant_action(
        &self,
        args: CancelAssistantActionArgs,
    ) -> RpcResult<CancelAssistantActionResult> {
        assistant::cancel_assistant_action(self, args).await
    }

    async fn update_job_scope(&self, args: UpdateJobScopeArgs) -> RpcResult<UpdateJobScopeResult> {
        jobs::update_job_scope(self, args).await
    }

    async fn draft_job_from_conversation(
        &self,
        args: DraftJobFromConversationArgs,
    ) -> RpcResult<Job> {
        jobs::draft_job_from_conversation(self, args).await
    }

    async fn list_personas(&self, args: ListPersonasArgs) -> RpcResult<ListPersonasResult> {
        personas::list_personas(self, args).await
    }

    async fn get_persona(&self, args: GetPersonaArgs) -> RpcResult<Persona> {
        personas::get_persona(self, args).await
    }

    async fn upsert_persona(&self, args: UpsertPersonaArgs) -> RpcResult<Persona> {
        personas::upsert_persona(self, args).await
    }

    async fn delete_persona(&self, args: DeletePersonaArgs) -> RpcResult<()> {
        personas::delete_persona(self, args).await
    }

    async fn approve_scope_patch(
        &self,
        args: codeless_rpc::ApproveScopePatchArgs,
    ) -> RpcResult<codeless_rpc::ScopePatchActionResult> {
        scope_patches::approve_scope_patch(self, args).await
    }

    async fn reject_scope_patch(
        &self,
        args: codeless_rpc::RejectScopePatchArgs,
    ) -> RpcResult<codeless_rpc::ScopePatchActionResult> {
        scope_patches::reject_scope_patch(self, args).await
    }

    async fn edit_scope_patch(
        &self,
        args: codeless_rpc::EditScopePatchArgs,
    ) -> RpcResult<codeless_rpc::ScopePatchActionResult> {
        scope_patches::edit_scope_patch(self, args).await
    }

    async fn revert_scope_patch(
        &self,
        args: codeless_rpc::RevertScopePatchArgs,
    ) -> RpcResult<codeless_rpc::RevertScopePatchResult> {
        scope_patches::revert_scope_patch(self, args).await
    }

    async fn list_proposed_patches(
        &self,
        args: codeless_rpc::ListProposedPatchesArgs,
    ) -> RpcResult<codeless_rpc::ListProposedPatchesResult> {
        scope_patches::list_proposed_patches(self, args).await
    }

    async fn set_job_policy(&self, args: SetJobPolicyArgs) -> RpcResult<()> {
        jobs::set_job_policy(self, args).await
    }
}
