use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use codeless_adapters_host::{
    FsError, GitCommitError, GitDiffError, HostFs, WorktreeManager, commit_paths, diff_against,
};
use codeless_rpc::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, ApproveReviewArgs, CancelChatTaskArgs, ChatMode,
    CommentReviewArgs, DeleteJobFileArgs, EventFilter, EventStream, FsCwdResult, FsReadDirArgs,
    FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs,
    GcWorktreeEntry, GcWorktreesArgs, GcWorktreesResult, GetJobArgs, JobDiffArgs, JobDiffFile,
    JobDiffResult, JobFileEntry, JobReportArgs, JobReportEventTally, JobReportResult,
    JobReportSpecChange, JobReportStage, JobReportToolCall, JobReportTurn, ListJobFilesArgs,
    ListJobFilesResult, ListJobsArgs, ListJobsResult, ListReposResult, ListReviewsArgs,
    ListReviewsResult, ListStagesArgs, ListStagesResult, PauseJobArgs, ReadJobFileArgs,
    ReadJobFileResult, RemoveRepoArgs, RerunJobArgs, ResumeJobArgs, RpcError, RpcResult, RpcServer,
    Since, StartJobArgs, StopActiveArgs, StopActiveResult, StopJobArgs, StopReviewArgs,
    SubmitJobArgs, UpdateJobTemplateArgs, UpdateJobTemplateResult, UploadChatAttachmentArgs,
    UploadChatAttachmentResult, WriteHandoverArgs, WriteHandoverResult, WriteJobFileArgs,
    WriteJobFileResult,
};
use codeless_types::{
    CostCents, Event, Job, JobId, JobStatus, Repo, RepoId, Review, ReviewStatus, StopReason, TaskId,
};
use sqlx::SqlitePool;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::job_dir::{
    self, FilenameError, JobLayout, directory_path, flat_yaml_path, sanitise_filename,
    template_yaml_path,
};
use crate::migrations::MIGRATOR;
use crate::store::SqliteStore;
use crate::template::JobTemplate;
use crate::time::now_ms;

/// In-process `RpcServer`. The CLI's `codeless run --once` path talks
/// to this directly without serialising over a wire; the hosted
/// server will hand the same struct to `axum` handlers. Repo, job,
/// and event rows are persisted in SQLite via `SqliteStore` and
/// `EventBus`; the broadcast channel inside the bus is the live
/// fan-out for in-process subscribers and the catch-up path goes
/// through the events table.
pub struct InProcessRpc {
    store: Arc<SqliteStore>,
    bus: Arc<EventBus>,
    /// Optional filesystem adapter. When `None`, every `fs_*` method
    /// returns `Internal` so callers see a typed failure rather than a
    /// panic — this is the path tests and CLI-local mode take when no
    /// workspace root is configured. The hosted server constructs the
    /// runtime with `with_fs` to expose the explorer/editor surfaces.
    fs: Option<Arc<HostFs>>,
    /// Optional worktree manager. `None` matches a serve mode booted
    /// without `--worktree-root`; in that mode `gc_worktrees` answers
    /// `Internal` so the UI can show a clear "no root configured"
    /// message rather than a misleading empty sweep. The driver
    /// receives its own clone of this Arc from the CLI layer.
    worktrees: Option<Arc<WorktreeManager>>,
    /// Optional CLI-runner registry that powers `agent_chat`. `None`
    /// means the footer agent panel is not wired — calls return
    /// `Internal`. Tests construct the runtime without it; the hosted
    /// server installs `Registry::with_defaults()` once at boot.
    agent_chat_registry: Option<Arc<ai_runner::Registry>>,
    /// Working directory CLI runners are invoked in for a chat turn.
    /// Defaults to the process cwd at runtime construction time so a
    /// quickstart server "just runs" against whatever directory the
    /// operator launched `codeless serve` from. A future "select
    /// folder" UI surface lands here.
    agent_chat_cwd: Option<std::path::PathBuf>,
    /// Cancellation tokens for in-flight `agent_chat` turns, keyed by
    /// the per-turn `TaskId`. The `agent_chat` spawn inserts the entry
    /// before launching the runner; a drop-guard removes it when the
    /// task completes (success, error, or panic). `cancel_chat_task`
    /// looks up the entry and fires the token; `stop_active` walks the
    /// map and fires every token whose `job_id` matches. Held behind a
    /// `parking_lot::Mutex` because every access is a brief map
    /// operation — never crosses an `await`.
    chat_cancels: ChatCancels,
}

/// One entry in the chat-cancel registry. `job_id` is the
/// `agent_chat` `session_id` the caller passed in; for the per-job
/// chat panel the UI uses the live `JobId` as the session, which is
/// what makes `stop_active(job_id)` able to fan out to the chat
/// turn(s) scoped to that job.
pub struct ChatCancelEntry {
    pub job_id: JobId,
    pub token: tokio_util::sync::CancellationToken,
}

/// In-memory, single-tenant registry of cancellation tokens for
/// in-flight `agent_chat` turns. See `InProcessRpc::chat_cancels`.
pub type ChatCancels = Arc<parking_lot::Mutex<HashMap<TaskId, ChatCancelEntry>>>;

/// Default event-broadcast lag tolerance per subscriber. See
/// `EventBus::new` for the trade-off; 1024 is the starting point and
/// has not yet been tuned against real load.
const DEFAULT_EVENT_BUFFER: usize = 1024;

impl InProcessRpc {
    /// Shortcut for tests: a fresh `sqlite::memory:` pool with the
    /// Appendix A migrations applied. `:memory:` databases are
    /// per-connection in raw SQLite, but sqlx's pool keeps a single
    /// dedicated connection alive for the lifetime of the pool when
    /// the URL is `sqlite::memory:` — so successive queries against
    /// the same `InProcessRpc::new()` see the same data.
    pub async fn new() -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        Self::with_db(pool).await
    }

    /// Open the runtime against a file-backed SQLite database. The
    /// file is created if missing so a first-run CLI invocation does
    /// not have to bootstrap state by hand. Used by the local-mode
    /// CLI's `--db` flag; the test suite continues to call `new()`
    /// for the in-memory pool.
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
    /// pre-migrated one and the caller never has to remember to run
    /// the migrator separately. Forward-only migration semantics —
    /// see `migrations::MIGRATOR`.
    pub async fn with_db(pool: SqlitePool) -> Result<Self, sqlx::Error> {
        MIGRATOR.run(&pool).await?;
        let bus = Arc::new(EventBus::new(pool.clone(), DEFAULT_EVENT_BUFFER));
        let store = Arc::new(SqliteStore::new(pool));
        // Startup reaper: any task whose lease expired before the
        // previous core died is still marked `running` in the DB.
        // Returning it to `enqueued` lets the new core re-pick it up.
        // SCOPE.md "Worktrees: failed worktrees are reaped on core
        // restart" — same idea, queue side.
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
        })
    }

    /// Attach a filesystem adapter so the `fs_*` RPC surface becomes
    /// live. Without this call those methods return `Internal` so
    /// transports get a typed failure rather than a panic.
    pub fn with_fs(mut self, fs: Arc<HostFs>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Attach the worktree manager so the `gc_worktrees` RPC can see
    /// the on-disk state. Same Arc the driver uses for per-job
    /// provisioning — they read and write the same `<base>/job-<id>`
    /// tree, just from different sides.
    pub fn with_worktrees(mut self, worktrees: Arc<WorktreeManager>) -> Self {
        self.worktrees = Some(worktrees);
        self
    }

    /// Wire the CLI-runner registry the footer agent panel dispatches
    /// onto. The hosted server constructs a single
    /// `Registry::with_defaults()` at boot and clones the Arc into
    /// both this method and the boot-time readiness probe that fills
    /// `ServerInfo.available_cli_runners`. `cwd` is the directory each
    /// chat turn runs in; passing the operator's launch cwd means a
    /// quickstart `codeless serve` "just works" without an extra flag.
    pub fn with_agent_chat(
        mut self,
        registry: Arc<ai_runner::Registry>,
        cwd: std::path::PathBuf,
    ) -> Self {
        self.agent_chat_registry = Some(registry);
        self.agent_chat_cwd = Some(cwd);
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
    /// having to spawn a real CLI runner; production callers reach the
    /// registry through `agent_chat` / `cancel_chat_task` only.
    pub fn chat_cancels(&self) -> &ChatCancels {
        &self.chat_cancels
    }

    /// Remove one worktree referenced by a GC entry. Resolves the
    /// source repo path via the job row when the entry's directory
    /// name parses as a `JobId`; falls back to a plain directory
    /// removal otherwise (stray `job-foo` left over from a renamed
    /// repo). Returns the failure as a string so callers can attach
    /// it to the entry without short-circuiting the whole sweep.
    async fn remove_one_worktree(
        &self,
        manager: &Arc<WorktreeManager>,
        entry: &GcWorktreeEntry,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let repo_path: Option<std::path::PathBuf> = if let Some(jid) = entry.job_id {
            let job = self
                .store
                .get_job(jid)
                .await
                .map_err(|e| format!("db: {e}"))?;
            let job = job.ok_or_else(|| format!("job {jid} not in store"))?;
            let repo = self
                .store
                .get_repo(job.repo_id)
                .await
                .map_err(|e| format!("db: {e}"))?;
            repo.map(|r| std::path::PathBuf::from(r.local_path))
        } else {
            None
        };
        let manager = Arc::clone(manager);
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || match repo_path {
            Some(rp) => manager.remove(&rp, &path).map_err(|e| e.to_string()),
            None => std::fs::remove_dir_all(&path).map_err(|e| e.to_string()),
        })
        .await
        .map_err(|e| format!("join: {e}"))?
    }
}

fn db_err(e: sqlx::Error) -> RpcError {
    RpcError::Internal(format!("db: {e}"))
}

/// RAII guard that removes a chat-cancel entry when the spawned chat
/// task ends. Held across the `run_chat` future so success, error,
/// and panic all evict the token; without this the registry would
/// leak entries every time a turn completes naturally.
struct ChatCancelGuard {
    cancels: ChatCancels,
    task_id: TaskId,
}

impl Drop for ChatCancelGuard {
    fn drop(&mut self) {
        self.cancels.lock().remove(&self.task_id);
    }
}

/// Shared "resolve a Pending review to a terminal status" helper for
/// `approve_review` and `stop_review`. Centralises the conflict /
/// not-found checks so the two RPCs cannot drift on which transitions
/// they accept. The caller publishes the corresponding event after we
/// return so the event-name choice stays at the call site.
async fn resolve_pending_review(
    rpc: &InProcessRpc,
    review_id: codeless_types::ReviewId,
    next: ReviewStatus,
) -> RpcResult<Review> {
    let mut review = rpc
        .store
        .get_review(review_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("review {review_id}")))?;
    if review.status != ReviewStatus::Pending {
        return Err(RpcError::Conflict(format!(
            "review {review_id} is already resolved ({:?})",
            review.status
        )));
    }
    let now = now_ms();
    review.status = next;
    review.resolved_at = Some(now);
    rpc.store.update_review(&review).await.map_err(db_err)?;
    Ok(review)
}

#[async_trait]
impl RpcServer for InProcessRpc {
    async fn add_repo(&self, args: AddRepoArgs) -> RpcResult<Repo> {
        let now = now_ms();
        let repo = Repo {
            id: RepoId::new(),
            name: args.name,
            clone_url: args.clone_url,
            default_branch: args.default_branch,
            local_path: args.local_path,
            git_auth: args.git_auth,
            concurrency_cap: args.concurrency_cap,
            default_runner: args.default_runner,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_repo(&repo).await.map_err(db_err)?;
        self.bus
            .publish(None, None, None, Event::RepoAdded { repo_id: repo.id }, now)
            .await
            .map_err(db_err)?;
        Ok(repo)
    }

    async fn remove_repo(&self, args: RemoveRepoArgs) -> RpcResult<()> {
        let removed = self.store.remove_repo(args.repo_id).await.map_err(db_err)?;
        if !removed {
            return Err(RpcError::NotFound(format!("repo {}", args.repo_id)));
        }
        self.bus
            .publish(
                None,
                None,
                None,
                Event::RepoRemoved {
                    repo_id: args.repo_id,
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn list_repos(&self) -> RpcResult<ListReposResult> {
        Ok(ListReposResult {
            repos: self.store.list_repos().await.map_err(db_err)?,
        })
    }

    async fn submit_job(&self, args: SubmitJobArgs) -> RpcResult<Job> {
        let repo = self
            .store
            .get_repo(args.repo_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("repo {}", args.repo_id)))?;
        let now = now_ms();

        // Enforce the one-in_repo-per-repo invariant. A second in_repo
        // job against the same repo would fight over the working copy.
        let mode = args.workspace_mode.unwrap_or_default();
        if mode == codeless_types::WorkspaceMode::InRepo {
            if let Some(existing) = self
                .store
                .active_in_repo_job(args.repo_id)
                .await
                .map_err(db_err)?
            {
                return Err(RpcError::Conflict(format!(
                    "repo {} is already in use by job {} in in_repo mode; \
                     stop it or submit as worktree",
                    args.repo_id, existing.id,
                )));
            }
        }

        // If the submit carries a template that parses into the
        // canonical `JobTemplate` shape, scaffold the on-disk job
        // directory *before* the Job row lands. The user never has
        // to "promote" a job later — the spec exists from the moment
        // the row exists, ready for editing in the SPEC pane.
        //
        // CLI submits whose YAML is the wrapper format
        // (`repo: …, runner: …, stages: [{name: …}]`) don't parse
        // here; they fall through unscaffolded and continue to behave
        // as today (the wrapper format is a `codeless-cli` concern,
        // not the runtime's). Prompt-only submits also fall through.
        if let Some(template_src) = args.template_yaml.as_deref() {
            if JobTemplate::parse_yaml(template_src).is_ok() {
                seed_job_directory(&repo.local_path, template_src)?;
            }
        }

        // Default landing state is `Draft` so the user can edit
        // spec / docs / handover before the driver picks the job up.
        // `start_immediately = true` is the legacy / power-user path
        // that skips the draft and queues the job for immediate
        // execution. JobQueued is emitted in both cases (it carries
        // the repo_id needed for the dashboard's row mount); the
        // status itself is what gates the driver.
        let initial_status = if args.start_immediately {
            JobStatus::Queued
        } else {
            JobStatus::Draft
        };
        let job = Job {
            id: JobId::new(),
            repo_id: args.repo_id,
            status: initial_status,
            stop_reason: None,
            template_yaml: args.template_yaml,
            prompt: args.prompt,
            runner: args.runner,
            branch: args.branch,
            workspace_mode: args.workspace_mode.unwrap_or_default(),
            worktree_path: None,
            cost_cap_cents: CostCents(args.cost_cap_cents),
            wall_clock_cap_ms: args.wall_clock_cap_ms,
            cost_cents: CostCents::ZERO,
            model: args.model,
            permission_mode: args.permission_mode,
            effort: args.effort,
            started_at: None,
            ended_at: None,
            created_at: now,
        };
        self.store.insert_job(&job).await.map_err(db_err)?;
        self.bus
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobQueued {
                    job_id: job.id,
                    repo_id: job.repo_id,
                },
                now,
            )
            .await
            .map_err(db_err)?;
        Ok(job)
    }

    async fn start_job(&self, args: StartJobArgs) -> RpcResult<Job> {
        let mut job = self
            .store
            .get_job(args.job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
        if job.status != JobStatus::Draft {
            return Err(RpcError::Conflict(format!(
                "job {} is {:?}, not Draft — only Draft jobs can be started",
                job.id, job.status
            )));
        }
        self.resync_template_from_disk(&mut job).await?;
        crate::state_machine::transition_job(job.status, JobStatus::Queued).map_err(|e| {
            RpcError::Conflict(format!(
                "illegal job transition from {:?} to Queued: {e}",
                job.status
            ))
        })?;
        job.status = JobStatus::Queued;
        if !self.store.update_job(&job).await.map_err(db_err)? {
            return Err(RpcError::NotFound(format!("job {}", args.job_id)));
        }
        // Reuse the long-defined-but-never-emitted `JobPromoted`
        // variant for Draft → Queued. The dashboard already maps it
        // to "running" optimistically; we'll refine the UI side to
        // show the real new status next.
        self.bus
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobPromoted { job_id: job.id },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(job)
    }

    async fn resume_job(&self, args: ResumeJobArgs) -> RpcResult<Job> {
        let mut job = self
            .store
            .get_job(args.job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
        if !matches!(
            job.status,
            JobStatus::Stopped | JobStatus::Failed | JobStatus::Paused
        ) {
            return Err(RpcError::Conflict(format!(
                "job {} is {:?}; only Stopped, Failed, or Paused jobs are \
                 resumable. Use stop_job or pause_job to interrupt a running job.",
                job.id, job.status
            )));
        }
        self.resync_template_from_disk(&mut job).await?;
        crate::state_machine::transition_job(job.status, JobStatus::Queued).map_err(|e| {
            RpcError::Conflict(format!(
                "illegal job transition from {:?} to Queued: {e}",
                job.status
            ))
        })?;
        // Cap bumps are additive on the previous values. Saturating
        // add so a user who passes a huge number doesn't overflow the
        // SQLite-side i64 and produce a negative cap that the watcher
        // would trip immediately.
        let previous_reason = job.stop_reason;
        if let Some(bump) = args.additional_cost_cap_cents {
            if bump > 0 {
                job.cost_cap_cents = CostCents(job.cost_cap_cents.0.saturating_add(bump));
            }
        }
        if let Some(bump) = args.additional_wall_clock_cap_ms {
            if bump > 0 {
                job.wall_clock_cap_ms = job.wall_clock_cap_ms.saturating_add(bump);
            }
        }
        job.status = JobStatus::Queued;
        // Clearing `stop_reason` here would erase the original
        // outcome from the row, which is the only place future
        // history (re-run-with-feedback, audit, dashboards) can read
        // why the job ended. `previous_reason` rides on the
        // `JobResumed` event for now; once A1's handover synthesiser
        // runs at stage boundaries, the `stop_reason` will be
        // captured into the handover and *then* cleared.
        job.stop_reason = None;
        // `ended_at` likewise clears — the job is live again. The
        // captured worktree path, branch, and per-stage `session_id`
        // values are untouched; the driver picks them up exactly as
        // they are.
        job.ended_at = None;
        if !self.store.update_job(&job).await.map_err(db_err)? {
            return Err(RpcError::NotFound(format!("job {}", args.job_id)));
        }
        self.bus
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobResumed {
                    job_id: job.id,
                    previous_reason,
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(job)
    }

    async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job> {
        self.store
            .get_job(args.job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))
    }

    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult> {
        Ok(ListJobsResult {
            jobs: self.store.list_jobs(args.repo_id).await.map_err(db_err)?,
        })
    }

    async fn list_stages(&self, args: ListStagesArgs) -> RpcResult<ListStagesResult> {
        let rows = self
            .store
            .list_stages_for_job(args.job_id)
            .await
            .map_err(db_err)?;
        let stages = rows
            .into_iter()
            .map(|row| codeless_rpc::StageRollup {
                stage: row.stage,
                cost_cents: row.cost_cents,
                task_count: row.task_count,
            })
            .collect();
        Ok(ListStagesResult { stages })
    }

    async fn job_report(&self, args: JobReportArgs) -> RpcResult<JobReportResult> {
        use sqlx::Row;
        let job = self
            .store
            .get_job(args.job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
        let job_id_s = args.job_id.to_string();
        let pool = self.store.pool();

        // Stage rows in chronological order so a stage that was retried
        // (cost-cap → resume) shows two entries with `attempt` 0 and 1
        // for the same ordinal. The recorder writes a fresh row per
        // attempt, so ordering by `started_at` is enough.
        let stage_rows = sqlx::query(
            "SELECT ordinal, name, status, session_id, started_at, ended_at \
             FROM stages WHERE job_id = ? ORDER BY COALESCE(started_at, 0)",
        )
        .bind(&job_id_s)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;

        let mut attempt_seen: HashMap<u32, u32> = HashMap::new();
        let mut stages: Vec<JobReportStage> = Vec::with_capacity(stage_rows.len());
        for r in stage_rows {
            let ordinal = r.try_get::<i64, _>("ordinal").map_err(db_err)? as u32;
            let attempt = attempt_seen.entry(ordinal).or_insert(0);
            let started_at: Option<i64> = r.try_get("started_at").map_err(db_err)?;
            let ended_at: Option<i64> = r.try_get("ended_at").map_err(db_err)?;
            stages.push(JobReportStage {
                ordinal,
                attempt: *attempt,
                title: r.try_get("name").map_err(db_err)?,
                status: r.try_get("status").map_err(db_err)?,
                session_id: r.try_get("session_id").map_err(db_err)?,
                // Filled in below from the turn buckets so a stage that
                // didn't persist a task row still gets the right number.
                cost_cents: 0,
                duration_ms: match (started_at, ended_at) {
                    (Some(s), Some(e)) => Some(e - s),
                    _ => None,
                },
                started_at,
                ended_at,
            });
            *attempt += 1;
        }

        // ai-message-complete = one Claude reply. Cost lives in the
        // payload; the row's `task_id` is the per-turn correlation id
        // and `stage_id` is null in this dataset, so we bucket turns
        // into stages by timestamp window below.
        let turn_rows = sqlx::query(
            "SELECT task_id, payload, created_at FROM events \
             WHERE job_id = ? AND type = 'ai-message-complete' \
             ORDER BY created_at",
        )
        .bind(&job_id_s)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;

        let mut turns: Vec<JobReportTurn> = Vec::with_capacity(turn_rows.len());
        for r in turn_rows {
            let task_id: Option<String> = r.try_get("task_id").map_err(db_err)?;
            let payload: String = r.try_get("payload").map_err(db_err)?;
            let at: i64 = r.try_get("created_at").map_err(db_err)?;
            let v: serde_json::Value =
                serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null);
            let cost_cents = v.get("cost_cents").and_then(|x| x.as_i64()).unwrap_or(0);
            let input_tokens = v.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
            let output_tokens = v.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
            // Find the stage whose window contains `at`. Stages may
            // overlap with their own retried attempt, so the *latest*
            // matching window wins (a turn after a resume belongs to
            // the resumed attempt, not the failed one).
            let stage_ordinal = stages
                .iter()
                .rev()
                .find(|s| match (s.started_at, s.ended_at) {
                    (Some(start), Some(end)) => at >= start && at <= end,
                    (Some(start), None) => at >= start,
                    _ => false,
                })
                .map(|s| s.ordinal);
            turns.push(JobReportTurn {
                task_id: task_id.unwrap_or_default(),
                stage_ordinal,
                cost_cents,
                input_tokens,
                output_tokens,
                at,
            });
        }

        // Fold per-turn cost back into the matching stage attempt so
        // the report has accurate per-stage spend even when the tasks
        // table is empty.
        for turn in &turns {
            if let Some(ord) = turn.stage_ordinal {
                if let Some(target) = stages.iter_mut().rev().find(|s| {
                    s.ordinal == ord
                        && match (s.started_at, s.ended_at) {
                            (Some(start), Some(end)) => turn.at >= start && turn.at <= end,
                            (Some(start), None) => turn.at >= start,
                            _ => false,
                        }
                }) {
                    target.cost_cents += turn.cost_cents;
                }
            }
        }

        let tool_rows = sqlx::query(
            "SELECT COALESCE(json_extract(payload, '$.tool'), '<unknown>') AS tool, \
                    COUNT(*) AS n \
             FROM events WHERE job_id = ? AND type = 'tool-call' \
             GROUP BY tool ORDER BY n DESC",
        )
        .bind(&job_id_s)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;
        let tool_calls: Vec<JobReportToolCall> = tool_rows
            .into_iter()
            .map(|r| {
                Ok::<_, sqlx::Error>(JobReportToolCall {
                    tool: r.try_get("tool")?,
                    count: r.try_get::<i64, _>("n")? as u32,
                })
            })
            .collect::<Result<_, _>>()
            .map_err(db_err)?;

        let tally_rows = sqlx::query(
            "SELECT type AS kind, COUNT(*) AS n FROM events WHERE job_id = ? \
             GROUP BY type ORDER BY n DESC",
        )
        .bind(&job_id_s)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;
        let event_tally: Vec<JobReportEventTally> = tally_rows
            .into_iter()
            .map(|r| {
                Ok::<_, sqlx::Error>(JobReportEventTally {
                    kind: r.try_get("kind")?,
                    count: r.try_get::<i64, _>("n")? as u32,
                })
            })
            .collect::<Result<_, _>>()
            .map_err(db_err)?;

        // Bucket spec-edit events by file. `JobTemplateUpdated` has no
        // filename in the payload (the file is implicit — there is one
        // `template.yaml` per job) and lands under `kind: "template"`;
        // `JobFileUpdated` carries a `filename` field and lands under
        // `kind: "file"` so the UI can render two distinct rows for the
        // same `SCOPE.md` if the user both edited it and ran a chat
        // turn that triggered a resync.
        let spec_rows = sqlx::query(
            "SELECT type AS kind, \
                    json_extract(payload, '$.filename') AS filename, \
                    COUNT(*) AS n, \
                    MAX(created_at) AS last_at \
             FROM events \
             WHERE job_id = ? AND type IN ('job-template-updated', 'job-file-updated') \
             GROUP BY type, filename \
             ORDER BY last_at DESC",
        )
        .bind(&job_id_s)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;
        let spec_changes: Vec<JobReportSpecChange> = spec_rows
            .into_iter()
            .map(|r| {
                let raw_kind: String = r.try_get("kind")?;
                let kind = match raw_kind.as_str() {
                    "job-template-updated" => "template".to_owned(),
                    "job-file-updated" => "file".to_owned(),
                    other => other.to_owned(),
                };
                Ok::<_, sqlx::Error>(JobReportSpecChange {
                    kind,
                    filename: r.try_get("filename")?,
                    count: r.try_get::<i64, _>("n")? as u32,
                    last_at: r.try_get("last_at")?,
                })
            })
            .collect::<Result<_, _>>()
            .map_err(db_err)?;

        let started_at = job.started_at.map(|t| t.0);
        let ended_at = job.ended_at.map(|t| t.0);
        let wall_clock_ms = match (started_at, ended_at) {
            (Some(s), Some(e)) => Some(e - s),
            _ => None,
        };

        Ok(JobReportResult {
            job_id: args.job_id,
            status: format!("{:?}", job.status).to_lowercase(),
            stop_reason: job.stop_reason.map(|r| format!("{:?}", r).to_lowercase()),
            cost_cents: job.cost_cents.0,
            cost_cap_cents: job.cost_cap_cents.0,
            started_at,
            ended_at,
            wall_clock_ms,
            stages,
            turns,
            tool_calls,
            event_tally,
            spec_changes,
        })
    }

    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()> {
        let Some(mut job) = self.store.get_job(args.job_id).await.map_err(db_err)? else {
            return Err(RpcError::NotFound(format!("job {}", args.job_id)));
        };
        match job.status {
            JobStatus::Completed | JobStatus::Failed | JobStatus::Stopped => {
                return Err(RpcError::Conflict(format!(
                    "job {} is already terminal ({:?})",
                    job.id, job.status
                )));
            }
            _ => {}
        }
        let now = now_ms();
        job.status = JobStatus::Stopped;
        job.stop_reason = Some(StopReason::User);
        job.ended_at = Some(now);
        self.store.update_job(&job).await.map_err(db_err)?;
        self.bus
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobStopped {
                    job_id: job.id,
                    reason: StopReason::User,
                },
                now,
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn pause_job(&self, args: PauseJobArgs) -> RpcResult<()> {
        let Some(mut job) = self.store.get_job(args.job_id).await.map_err(db_err)? else {
            return Err(RpcError::NotFound(format!("job {}", args.job_id)));
        };
        if !matches!(job.status, JobStatus::Running | JobStatus::AwaitingReview) {
            return Err(RpcError::Conflict(format!(
                "job {} is {:?}; only Running or AwaitingReview jobs can be paused. \
                 Use start_job to promote a Draft, or resume_job to restart a paused/stopped row.",
                job.id, job.status
            )));
        }
        crate::state_machine::transition_job(job.status, JobStatus::Paused).map_err(|e| {
            RpcError::Conflict(format!(
                "illegal job transition from {:?} to Paused: {e}",
                job.status
            ))
        })?;
        let now = now_ms();
        job.status = JobStatus::Paused;
        job.stop_reason = Some(StopReason::User);
        job.ended_at = Some(now);
        self.store.update_job(&job).await.map_err(db_err)?;
        // The cap-watcher subscribes to the bus and fires the
        // runner's cancellation token when it sees a `JobPaused`
        // (or `JobStopped` / `JobFailed`) it didn't author. That's
        // how the in-flight runner finds out the row has moved
        // out from under it.
        self.bus
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobPaused {
                    job_id: job.id,
                    reason: StopReason::User,
                },
                now,
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn rerun_job(&self, args: RerunJobArgs) -> RpcResult<Job> {
        let Some(source) = self
            .store
            .get_job(args.source_job_id)
            .await
            .map_err(db_err)?
        else {
            return Err(RpcError::NotFound(format!("job {}", args.source_job_id)));
        };
        let now = now_ms();
        // Empty branch makes `WorktreeManager` fall back to the
        // canonical `codeless/job-<new_id>` so a rerun never collides
        // with the source job's branch.
        let job = Job {
            id: JobId::new(),
            repo_id: source.repo_id,
            status: JobStatus::Draft,
            stop_reason: None,
            template_yaml: source.template_yaml,
            prompt: source.prompt,
            runner: source.runner,
            branch: String::new(),
            workspace_mode: source.workspace_mode,
            worktree_path: None,
            cost_cap_cents: source.cost_cap_cents,
            wall_clock_cap_ms: source.wall_clock_cap_ms,
            cost_cents: CostCents::ZERO,
            model: source.model,
            permission_mode: source.permission_mode,
            effort: source.effort,
            started_at: None,
            ended_at: None,
            created_at: now,
        };
        self.store.insert_job(&job).await.map_err(db_err)?;
        self.bus
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobQueued {
                    job_id: job.id,
                    repo_id: job.repo_id,
                },
                now,
            )
            .await
            .map_err(db_err)?;
        Ok(job)
    }

    async fn gc_worktrees(&self, args: GcWorktreesArgs) -> RpcResult<GcWorktreesResult> {
        let Some(worktrees) = self.worktrees.clone() else {
            return Err(RpcError::Internal(
                "gc_worktrees: no worktree root configured on the server".into(),
            ));
        };
        let manager = worktrees.clone();
        let on_disk = tokio::task::spawn_blocking(move || manager.list_on_disk())
            .await
            .map_err(|e| RpcError::Internal(format!("gc list join: {e}")))?
            .map_err(|e| RpcError::Internal(format!("gc list: {e}")))?;
        let root = worktrees.base().to_string_lossy().into_owned();

        let now_i64: i64 = now_ms().as_i64();
        let cutoff = args.older_than_ms.map(|d| now_i64.saturating_sub(d.max(0)));
        let id_filter: Option<std::collections::HashSet<String>> = args
            .job_ids
            .as_ref()
            .map(|ids| ids.iter().map(|id| id.to_string()).collect());

        let mut entries: Vec<GcWorktreeEntry> = Vec::with_capacity(on_disk.len());
        let mut total: i64 = 0;
        let mut removed: i64 = 0;

        for entry in on_disk {
            if let Some(set) = &id_filter {
                if !set.contains(&entry.job_id) {
                    continue;
                }
            }
            if let Some(c) = cutoff {
                let mtime = entry.mtime_ms.unwrap_or(now_i64);
                if mtime > c {
                    continue;
                }
            }
            total = total.saturating_add(entry.size_bytes);

            // Parse the directory's job_id back to a `JobId`. If
            // parsing fails (the user left a stray `job-foo` dir
            // around) the entry still surfaces — just without a
            // typed id and without an automatic remove, since we
            // cannot resolve a source repo for a non-job tree.
            let parsed_id: Option<codeless_types::JobId> = entry.job_id.parse().ok();

            let mut gc_entry = GcWorktreeEntry {
                job_id: parsed_id,
                path: entry.path.to_string_lossy().into_owned(),
                size_bytes: entry.size_bytes,
                mtime_ms: entry.mtime_ms,
                removed: false,
                error: None,
            };

            if !args.dry_run {
                match self
                    .remove_one_worktree(&worktrees, &gc_entry, &entry.path)
                    .await
                {
                    Ok(()) => {
                        gc_entry.removed = true;
                        removed += 1;
                    }
                    Err(e) => {
                        gc_entry.error = Some(e);
                    }
                }
            }

            entries.push(gc_entry);
        }

        Ok(GcWorktreesResult {
            entries,
            total_size_bytes: total,
            removed_count: removed,
            root: Some(root),
        })
    }

    async fn job_diff(&self, args: JobDiffArgs) -> RpcResult<JobDiffResult> {
        let Some(job) = self.store.get_job(args.job_id).await.map_err(db_err)? else {
            return Err(RpcError::NotFound(format!("job {}", args.job_id)));
        };
        let Some(repo) = self.store.get_repo(job.repo_id).await.map_err(db_err)? else {
            return Err(RpcError::NotFound(format!("repo {}", job.repo_id)));
        };
        // `Job.branch` is the canonical branch name: `submit_job`
        // accepts the wizard's value, the runtime writes the actually
        // created branch back to the row on worktree provisioning, and
        // diffs against the repo's default branch as the merge base.
        // Falls back to `codeless/job-<id>` for legacy rows that
        // pre-date the honour-non-empty-branch behaviour.
        let head = if job.branch.trim().is_empty() {
            format!("codeless/job-{}", job.id)
        } else {
            job.branch.clone()
        };
        let base = repo.default_branch.clone();
        let repo_path = std::path::PathBuf::from(&repo.local_path);
        // The diff is intentionally synchronous (git is fast at the
        // file counts a single job touches). Wrap with `spawn_blocking`
        // so a slow git invocation does not stall the tokio reactor.
        let head_clone = head.clone();
        let base_clone = base.clone();
        let files =
            tokio::task::spawn_blocking(move || diff_against(&repo_path, &base_clone, &head_clone))
                .await
                .map_err(|e| RpcError::Internal(format!("git diff join: {e}")))?
                .map_err(diff_err)?;
        let files = files
            .into_iter()
            .map(|f| JobDiffFile {
                path: f.path,
                status: f.status,
                additions: f.additions,
                deletions: f.deletions,
                is_binary: f.is_binary,
                patch: f.patch,
            })
            .collect();
        Ok(JobDiffResult { base, head, files })
    }

    async fn list_reviews(&self, args: ListReviewsArgs) -> RpcResult<ListReviewsResult> {
        Ok(ListReviewsResult {
            reviews: self
                .store
                .list_reviews(args.job_id, args.stage_id, args.status)
                .await
                .map_err(db_err)?,
        })
    }

    async fn approve_review(&self, args: ApproveReviewArgs) -> RpcResult<Review> {
        let review = resolve_pending_review(self, args.review_id, ReviewStatus::Approved).await?;
        self.bus
            .publish(
                None,
                Some(review.stage_id),
                None,
                Event::ReviewApproved {
                    review_id: review.id,
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(review)
    }

    async fn comment_review(&self, args: CommentReviewArgs) -> RpcResult<Review> {
        let mut review = self
            .store
            .get_review(args.review_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("review {}", args.review_id)))?;
        review.comment = Some(args.comment.clone());
        self.store.update_review(&review).await.map_err(db_err)?;
        self.bus
            .publish(
                None,
                Some(review.stage_id),
                None,
                Event::ReviewCommented {
                    review_id: review.id,
                    comment: args.comment,
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(review)
    }

    async fn stop_review(&self, args: StopReviewArgs) -> RpcResult<Review> {
        let review = resolve_pending_review(self, args.review_id, ReviewStatus::Stopped).await?;
        self.bus
            .publish(
                None,
                Some(review.stage_id),
                None,
                Event::ReviewStopped {
                    review_id: review.id,
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(review)
    }

    async fn subscribe(&self, filter: EventFilter, since: Since) -> RpcResult<EventStream> {
        let local = match filter {
            EventFilter::All => SubscribeFilter::All,
            EventFilter::Job { job_id } => SubscribeFilter::Job(job_id),
        };
        self.bus.subscribe_since(local, since).await.map_err(db_err)
    }

    async fn fs_read_dir(&self, args: FsReadDirArgs) -> RpcResult<FsReadDirResult> {
        let fs = self.fs.as_ref().ok_or_else(fs_not_configured)?;
        let entries = fs.read_dir(&args.path).await.map_err(fs_err)?;
        Ok(FsReadDirResult { entries })
    }

    async fn fs_read_file(&self, args: FsReadFileArgs) -> RpcResult<FsReadFileResult> {
        let fs = self.fs.as_ref().ok_or_else(fs_not_configured)?;
        let content = fs.read_file(&args.path).await.map_err(fs_err)?;
        Ok(FsReadFileResult { content })
    }

    async fn fs_write_file(&self, args: FsWriteFileArgs) -> RpcResult<()> {
        let fs = self.fs.as_ref().ok_or_else(fs_not_configured)?;
        fs.write_file(&args.path, &args.content)
            .await
            .map_err(fs_err)?;
        Ok(())
    }

    async fn fs_stat(&self, args: FsStatArgs) -> RpcResult<FsStatResult> {
        let fs = self.fs.as_ref().ok_or_else(fs_not_configured)?;
        let entry = fs.stat(&args.path).await.map_err(fs_err)?;
        Ok(match entry {
            Some((kind, size, mtime)) => FsStatResult {
                kind: Some(kind),
                size,
                mtime,
            },
            None => FsStatResult {
                kind: None,
                size: None,
                mtime: None,
            },
        })
    }

    async fn fs_cwd(&self) -> RpcResult<FsCwdResult> {
        let fs = self.fs.as_ref().ok_or_else(fs_not_configured)?;
        Ok(FsCwdResult {
            path: fs.root().to_string_lossy().into_owned(),
        })
    }

    async fn list_job_files(&self, args: ListJobFilesArgs) -> RpcResult<ListJobFilesResult> {
        let (repo_path, name) = self.resolve_repo_and_template_name(args.job_id).await?;
        let layout = job_dir::resolve(&repo_path, &name);
        let mut entries: Vec<JobFileEntry> = Vec::new();
        let mut directory_path_str: Option<String> = None;

        if matches!(layout, JobLayout::Directory | JobLayout::FlatPreferred) {
            let dir = directory_path(&repo_path, &name);
            directory_path_str = Some(dir.to_string_lossy().into_owned());
            let read_dir = std::fs::read_dir(&dir)
                .map_err(|e| RpcError::Internal(format!("read job dir {}: {e}", dir.display())))?;
            let mut files: Vec<std::path::PathBuf> = read_dir
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            files.sort();

            let mut tpl: Option<JobFileEntry> = None;
            for path in files {
                let base = match path.file_name().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let lower = base.to_ascii_lowercase();
                let entry = JobFileEntry {
                    name: base.clone(),
                    is_template: lower == "template.yaml",
                    is_scope: lower == "scope.md",
                    is_workflow: lower == "workflow.md",
                };
                if entry.is_template {
                    tpl = Some(entry);
                } else {
                    entries.push(entry);
                }
            }
            if let Some(t) = tpl {
                entries.insert(0, t);
            }
        }

        Ok(ListJobFilesResult {
            entries,
            layout: layout.wire_name().to_string(),
            directory_path: directory_path_str,
        })
    }

    async fn read_job_file(&self, args: ReadJobFileArgs) -> RpcResult<ReadJobFileResult> {
        let (repo_path, name) = self.resolve_repo_and_template_name(args.job_id).await?;
        let filename = sanitise_filename(&args.filename).map_err(filename_err)?;
        let path = directory_path(&repo_path, &name).join(&filename);
        let content = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                RpcError::NotFound(format!("job file {name}/{filename}"))
            }
            _ => RpcError::Internal(format!("read {}: {e}", path.display())),
        })?;
        Ok(ReadJobFileResult { content })
    }

    async fn write_job_file(&self, args: WriteJobFileArgs) -> RpcResult<WriteJobFileResult> {
        let (repo_path, name) = self.resolve_repo_and_template_name(args.job_id).await?;
        let filename = sanitise_filename(&args.filename).map_err(filename_err)?;

        let layout = job_dir::resolve(&repo_path, &name);
        if matches!(layout, JobLayout::Flat | JobLayout::FlatPreferred) {
            migrate_flat_to_directory(&repo_path, &name)?;
        }

        let dir = directory_path(&repo_path, &name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| RpcError::Internal(format!("create job dir {}: {e}", dir.display())))?;
        let path = dir.join(&filename);
        std::fs::write(&path, &args.content)
            .map_err(|e| RpcError::Internal(format!("write {}: {e}", path.display())))?;
        commit_paths(
            &repo_path,
            &format!("update job-file: {name}/{filename}"),
            std::slice::from_ref(&path),
        )
        .map_err(git_commit_err)?;

        self.bus
            .publish(
                Some(args.job_id),
                None,
                None,
                Event::JobFileUpdated {
                    job_id: args.job_id,
                    filename: filename.clone(),
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;

        Ok(WriteJobFileResult { name: filename })
    }

    async fn delete_job_file(&self, args: DeleteJobFileArgs) -> RpcResult<()> {
        let (repo_path, name) = self.resolve_repo_and_template_name(args.job_id).await?;
        let filename = sanitise_filename(&args.filename).map_err(filename_err)?;
        let path = directory_path(&repo_path, &name).join(&filename);
        if !path.exists() {
            return Err(RpcError::NotFound(format!("job file {name}/{filename}")));
        }
        std::fs::remove_file(&path)
            .map_err(|e| RpcError::Internal(format!("delete {}: {e}", path.display())))?;
        commit_paths(
            &repo_path,
            &format!("delete job-file: {name}/{filename}"),
            &[path],
        )
        .map_err(git_commit_err)?;
        self.bus
            .publish(
                Some(args.job_id),
                None,
                None,
                Event::JobFileUpdated {
                    job_id: args.job_id,
                    filename,
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn update_job_template(
        &self,
        args: UpdateJobTemplateArgs,
    ) -> RpcResult<UpdateJobTemplateResult> {
        let parsed = JobTemplate::parse_yaml(&args.template_yaml)
            .map_err(|e| RpcError::InvalidArgument(format!("template parse: {e}")))?;

        let mut job = self
            .store
            .get_job(args.job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
        let prev_name = match job.template_yaml.as_deref() {
            Some(prev) => match JobTemplate::parse_yaml(prev) {
                Ok(tpl) => tpl.name,
                Err(_) => parsed.name.clone(),
            },
            None => parsed.name.clone(),
        };
        if prev_name != parsed.name {
            return Err(RpcError::Conflict(format!(
                "rename refused: spec name is `{prev_name}`, cannot become `{}`. Submit a fresh job to rename.",
                parsed.name,
            )));
        }

        let repo = self
            .store
            .get_repo(job.repo_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
        let repo_path = std::path::PathBuf::from(repo.local_path);

        let layout = job_dir::resolve(&repo_path, &parsed.name);
        if matches!(layout, JobLayout::Flat | JobLayout::FlatPreferred) {
            migrate_flat_to_directory(&repo_path, &parsed.name)?;
        }

        let tpl_path = template_yaml_path(&repo_path, &parsed.name);
        if let Some(parent) = tpl_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RpcError::Internal(format!("create job dir {}: {e}", parent.display()))
            })?;
        }
        std::fs::write(&tpl_path, &args.template_yaml)
            .map_err(|e| RpcError::Internal(format!("write {}: {e}", tpl_path.display())))?;
        commit_paths(
            &repo_path,
            &format!("update template: {}", parsed.name),
            std::slice::from_ref(&tpl_path),
        )
        .map_err(git_commit_err)?;

        job.template_yaml = Some(args.template_yaml);
        if !self.store.update_job(&job).await.map_err(db_err)? {
            return Err(RpcError::NotFound(format!("job {}", args.job_id)));
        }

        self.bus
            .publish(
                Some(args.job_id),
                None,
                None,
                Event::JobTemplateUpdated {
                    job_id: args.job_id,
                },
                now_ms(),
            )
            .await
            .map_err(db_err)?;

        Ok(UpdateJobTemplateResult { name: parsed.name })
    }

    async fn write_handover(&self, args: WriteHandoverArgs) -> RpcResult<WriteHandoverResult> {
        let job = self
            .store
            .get_job(args.job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
        let worktree = job.worktree_path.as_deref().ok_or_else(|| {
            RpcError::Conflict(format!(
                "job {} has no worktree yet; the runner must run before a handover can be seeded",
                args.job_id
            ))
        })?;
        let path = crate::handover::write_handover(
            std::path::Path::new(worktree),
            args.job_id,
            &args.handover,
        )
        .await
        .map_err(|e| RpcError::Internal(format!("write handover: {e}")))?;
        Ok(WriteHandoverResult {
            path: path.to_string_lossy().into_owned(),
        })
    }

    async fn agent_chat(&self, args: AgentChatArgs) -> RpcResult<AgentChatResult> {
        let registry = self.agent_chat_registry.as_ref().ok_or_else(|| {
            RpcError::Internal("agent_chat registry is not configured on this runtime".to_owned())
        })?;
        let default_cwd = self.agent_chat_cwd.clone().ok_or_else(|| {
            RpcError::Internal("agent_chat cwd is not configured on this runtime".to_owned())
        })?;
        // Per-call cwd override (used by the per-job chat panel so a
        // question can read files that only exist on the job's branch).
        // The path must be a real directory under one of the configured
        // fs roots; otherwise reject with InvalidArgument rather than
        // silently fall back, so callers see the misconfiguration.
        let cwd = match args.cwd.as_deref() {
            Some(p) => {
                let abs = std::path::PathBuf::from(p);
                let canon = std::fs::canonicalize(&abs).map_err(|_| {
                    RpcError::InvalidArgument(format!("agent_chat cwd does not exist: {p}"))
                })?;
                if !canon.is_dir() {
                    return Err(RpcError::InvalidArgument(format!(
                        "agent_chat cwd is not a directory: {p}"
                    )));
                }
                let fs_allowed = self
                    .fs
                    .as_ref()
                    .map(|fs| fs.is_path_allowed(&canon))
                    .unwrap_or(false);
                // Also allow cwd under any registered repo's local_path
                // so the per-job chat panel can target repos that sit
                // outside the primary --fs-root.
                let repo_allowed = if !fs_allowed {
                    let repos = self.store.list_repos().await.map_err(db_err)?;
                    repos.iter().any(|r| {
                        std::fs::canonicalize(&r.local_path)
                            .map(|rp| canon.starts_with(&rp))
                            .unwrap_or(false)
                    })
                } else {
                    false
                };
                if !fs_allowed && !repo_allowed {
                    return Err(RpcError::InvalidArgument(format!(
                        "agent_chat cwd is outside the configured fs roots: {p}"
                    )));
                }
                canon
            }
            None => default_cwd,
        };
        let provider =
            codeless_adapters_host::parse_cli_runner_id(&args.runner).ok_or_else(|| {
                RpcError::InvalidArgument(format!("unknown CLI runner id `{}`", args.runner))
            })?;

        let session_id = args.session_id;
        let task_id = codeless_types::TaskId::new();
        let bus = Arc::clone(&self.bus);
        let registry = Arc::clone(registry);
        // When the chat session_id maps to a real job, fold that job's
        // template.yaml + SCOPE.md + WORKFLOW.md into the preamble so
        // the agent answers grounded in the job's spec instead of
        // reverse-engineering it from filesystem clues. The lookup is
        // best-effort: footer-panel turns pass a fresh correlation id
        // that does not resolve to a job, and a job in the directory
        // layout might lack one of the supporting files. Both cases
        // skip the block silently rather than failing the turn.
        let mode = args.mode.unwrap_or_default();
        let job_spec_block = self.load_chat_job_spec(session_id).await;
        let prompt = build_chat_prompt(
            args.context.as_ref(),
            job_spec_block.as_deref(),
            mode,
            &args.prompt,
        );
        // Spec mode: clamp the agent to the read + edit tools so it
        // can author the job spec but cannot run `Bash`, hit the
        // network, or `git commit` over repo source. The wrapper takes
        // a comma-separated list; entries match the claude-code tool
        // names (Read / Edit / Write / Glob / Grep / LS / TodoWrite).
        // Work mode passes `None` to keep the existing full-tool
        // behaviour.
        let allowed_tools = match mode {
            ChatMode::Spec => Some(SPEC_MODE_ALLOWED_TOOLS.to_owned()),
            ChatMode::Work => None,
        };

        // Register the cancel token before the spawn so a racing
        // `cancel_chat_task` issued between `agent_chat` returning and
        // the spawned task being scheduled still finds an entry to
        // fire. The drop-guard inside the task removes the entry on
        // any exit (success / error / panic), so the registry never
        // leaks even if the runner crashes mid-turn.
        let cancel = tokio_util::sync::CancellationToken::new();
        self.chat_cancels.lock().insert(
            task_id,
            ChatCancelEntry {
                job_id: session_id,
                token: cancel.clone(),
            },
        );
        let cancels = Arc::clone(&self.chat_cancels);

        // Detached: the call returns once the runner has been spawned;
        // its tokens / tool-calls / completion event flow back through
        // the bus, keyed by `session_id` so the caller's subscribe
        // filter matches them. A panicked task only kills the chat
        // turn — log it and let other turns continue.
        tokio::spawn(async move {
            let _guard = ChatCancelGuard { cancels, task_id };
            let publish = move |event: codeless_types::Event| {
                let bus = Arc::clone(&bus);
                async move {
                    bus.publish(Some(session_id), None, Some(task_id), event, now_ms())
                        .await
                        .map(|_| ())
                }
            };
            if let Err(e) = codeless_adapters_host::run_chat(
                registry,
                provider,
                prompt,
                cwd,
                task_id,
                allowed_tools,
                publish,
                cancel,
            )
            .await
            {
                tracing::warn!(error = %e, "agent_chat run failed");
            }
        });

        Ok(AgentChatResult {
            session_id,
            task_id,
        })
    }

    async fn upload_chat_attachment(
        &self,
        args: UploadChatAttachmentArgs,
    ) -> RpcResult<UploadChatAttachmentResult> {
        use base64::Engine as _;

        // Worktree-scoped: the runner runs with the worktree as cwd
        // (see `agent_chat`'s per-call cwd resolution wired by the UI),
        // so attachments dropped under `.codeless/chat-attachments/`
        // are reachable by relative path. Conflict if the runner has
        // not provisioned a worktree yet — there is no sensible
        // fallback location that survives a re-run.
        let job = self
            .store
            .get_job(args.job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))?;
        let worktree = job.worktree_path.as_deref().ok_or_else(|| {
            RpcError::Conflict(format!(
                "job {} has no worktree yet; submit/run the job before attaching files",
                args.job_id
            ))
        })?;

        // Reuse the job-file sanitiser for path traversal / dotfile
        // rejection. `template.yaml` is harmless here (we are not in
        // the job dir) but the sanitiser still catches it; rename if
        // someone really needs to attach one.
        let safe = sanitise_filename(&args.filename).map_err(filename_err)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(args.content_b64.as_bytes())
            .or_else(|_| {
                base64::engine::general_purpose::STANDARD_NO_PAD.decode(args.content_b64.as_bytes())
            })
            .map_err(|e| RpcError::InvalidArgument(format!("content_b64: {e}")))?;

        let dir = std::path::Path::new(worktree)
            .join(".codeless")
            .join("chat-attachments");
        std::fs::create_dir_all(&dir).map_err(|e| {
            RpcError::Internal(format!(
                "create chat-attachments dir {}: {e}",
                dir.display()
            ))
        })?;

        // Unique prefix: millis + a per-process atomic counter. Two
        // uploads from the same UI in the same millisecond still get
        // distinct names; the original basename is preserved as the
        // suffix so model-side mentions stay recognisable.
        let stamp = now_ms().0;
        let seq = ATTACHMENT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stored = format!("{stamp}-{seq}-{safe}");
        let abs = dir.join(&stored);
        std::fs::write(&abs, &bytes)
            .map_err(|e| RpcError::Internal(format!("write {}: {e}", abs.display())))?;

        let relative_path = format!(".codeless/chat-attachments/{stored}");
        Ok(UploadChatAttachmentResult {
            relative_path,
            absolute_path: abs.to_string_lossy().into_owned(),
        })
    }

    async fn cancel_chat_task(&self, args: CancelChatTaskArgs) -> RpcResult<()> {
        // Idempotent by design: a missing entry means the chat turn
        // either already completed (the drop-guard removed it) or was
        // cancelled by a previous call. Returning `Ok(())` lets the UI
        // race the natural end of the stream without distinguishing
        // "stopped" from "already over".
        if let Some(entry) = self.chat_cancels.lock().get(&args.task_id) {
            entry.token.cancel();
        }
        Ok(())
    }

    async fn stop_active(&self, args: StopActiveArgs) -> RpcResult<StopActiveResult> {
        // Job side: only call `stop_job` when the row is in a state
        // it accepts; checking up front avoids a misleading
        // `Conflict` error for the common "stop a chat over a
        // completed job" path. The match must mirror the guard in
        // `stop_job`, hence the same set of variants here.
        let stopped_job = match self.store.get_job(args.job_id).await.map_err(db_err)? {
            Some(job)
                if matches!(
                    job.status,
                    JobStatus::Running
                        | JobStatus::AwaitingReview
                        | JobStatus::Queued
                        | JobStatus::Paused
                        | JobStatus::Draft
                ) =>
            {
                self.stop_job(StopJobArgs {
                    job_id: args.job_id,
                })
                .await?;
                true
            }
            Some(_) => false,
            None => return Err(RpcError::NotFound(format!("job {}", args.job_id))),
        };

        // Chat side: snapshot the matching entries under the lock so
        // we can fire the tokens outside it. The drop-guards on the
        // spawned tasks evict the entries themselves; we deliberately
        // leave them in place so a racing second `stop_active` is a
        // no-op fire rather than a spurious "nothing was running".
        let cancelled_chat_task_ids: Vec<TaskId> = {
            let map = self.chat_cancels.lock();
            map.iter()
                .filter(|(_, entry)| entry.job_id == args.job_id)
                .map(|(task_id, entry)| {
                    entry.token.cancel();
                    *task_id
                })
                .collect()
        };

        Ok(StopActiveResult {
            stopped_job,
            cancelled_chat_task_ids,
        })
    }
}

/// Per-process counter to disambiguate attachment uploads that land in
/// the same millisecond. Wraps cheaply; the millis prefix ensures
/// uniqueness in practice.
static ATTACHMENT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Render the optional `ChatContext` into a deterministic preamble
/// prepended to the user prompt before the runner is spawned. Kept
/// out of the trait method body so the prompt-shaping rules are
/// covered by unit tests independently of the bus / registry plumbing.
///
/// The preamble is only emitted when at least one context field is
/// populated; otherwise the prompt passes through unchanged so
/// short-prompt fidelity (e.g. "what time is it") is preserved.
fn build_chat_prompt(
    ctx: Option<&codeless_rpc::ChatContext>,
    job_spec_block: Option<&str>,
    mode: ChatMode,
    prompt: &str,
) -> String {
    let ctx_has_any = ctx.is_some_and(|c| {
        c.ui_location.is_some()
            || c.selection.is_some()
            || !c.attachments.is_empty()
            || !c.user_prompts.is_empty()
    });
    let job_has_any = job_spec_block.is_some_and(|s| !s.is_empty());
    let spec_mode = mode == ChatMode::Spec;
    if !ctx_has_any && !job_has_any && !spec_mode {
        return prompt.to_owned();
    }

    let mut out = String::new();
    if spec_mode {
        out.push_str(SPEC_MODE_BANNER);
        out.push('\n');
    }
    out.push_str("# Context\n\n");
    if let Some(block) = job_spec_block.filter(|s| !s.is_empty()) {
        out.push_str(block);
        if !block.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    let Some(ctx) = ctx else {
        out.push_str("# Request\n\n");
        out.push_str(prompt);
        return out;
    };
    if let Some(loc) = ctx.ui_location.as_deref() {
        out.push_str(&format!("User is viewing: {loc}\n\n"));
    }
    if !ctx.attachments.is_empty() {
        out.push_str("Files attached (paths are relative to the working directory):\n");
        for a in &ctx.attachments {
            match a.mime_type.as_deref() {
                Some(mt) => out.push_str(&format!("- {} ({mt})\n", a.relative_path)),
                None => out.push_str(&format!("- {}\n", a.relative_path)),
            }
        }
        out.push('\n');
    }
    if let Some(sel) = ctx.selection.as_deref() {
        out.push_str("Current selection:\n```\n");
        out.push_str(sel);
        if !sel.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n\n");
    }
    for snippet in &ctx.user_prompts {
        out.push_str(&format!("## {}\n\n{}\n\n", snippet.label, snippet.body));
    }
    out.push_str("# Request\n\n");
    out.push_str(prompt);
    out
}

impl InProcessRpc {
    /// Resolve a `job_id` to the repo's on-disk path and the job's
    /// directory name. Template jobs use the template's `name` field;
    /// prompt-only jobs fall back to `job-<id>` so the file surface
    /// (chat.md, supporting docs) works even without a template.
    async fn resolve_repo_and_template_name(
        &self,
        job_id: codeless_types::JobId,
    ) -> RpcResult<(std::path::PathBuf, String)> {
        let job = self
            .store
            .get_job(job_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("job {job_id}")))?;
        let name = match job.template_yaml.as_ref() {
            Some(yaml) => {
                let template = JobTemplate::parse_yaml(yaml).map_err(|e| {
                    RpcError::InvalidArgument(format!("job {job_id} template parse: {e}"))
                })?;
                template.name
            }
            None => format!("job-{job_id}"),
        };
        let repo = self
            .store
            .get_repo(job.repo_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
        Ok((std::path::PathBuf::from(repo.local_path), name))
    }

    /// Re-read `template.yaml` from disk and refresh the job's
    /// `template_yaml` DB column when it differs. Called from
    /// `start_job` and `resume_job` so chat-driven filesystem edits
    /// (made by the AI agent through its ambient `Edit`/`Write`
    /// tools) land in SQLite before the driver reads the template.
    ///
    /// No-op when the job has no on-disk `template.yaml` (e.g. a
    /// prompt-only job or one whose dir was never seeded), when the
    /// DB and disk contents match, or when the job has no
    /// `template_yaml` mirror in the DB yet — promotion of a fresh
    /// prompt-only job into a templated job is not the resync's job.
    ///
    /// A parse failure surfaces as `InvalidArgument` so the user
    /// sees the line/column from the YAML parser when they click
    /// **run** on a broken spec. A `name:` field that no longer
    /// matches the job's recorded name is `Conflict` — renames are
    /// rejected by `update_job_template` too, so chat edits must
    /// not be able to bypass that rule by writing to disk.
    async fn resync_template_from_disk(&self, job: &mut codeless_types::Job) -> RpcResult<()> {
        let Some(db_yaml) = job.template_yaml.clone() else {
            return Ok(());
        };
        let prev = JobTemplate::parse_yaml(&db_yaml).map_err(|e| {
            RpcError::Internal(format!("job {} stored template parse: {e}", job.id))
        })?;

        let repo = self
            .store
            .get_repo(job.repo_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
        let repo_path = std::path::PathBuf::from(&repo.local_path);
        let tpl_path = template_yaml_path(&repo_path, &prev.name);

        let disk_yaml = match std::fs::read_to_string(&tpl_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(RpcError::Internal(format!(
                    "read {}: {e}",
                    tpl_path.display()
                )));
            }
        };
        if disk_yaml == db_yaml {
            return Ok(());
        }

        let parsed = JobTemplate::parse_yaml(&disk_yaml).map_err(|e| {
            RpcError::InvalidArgument(format!(
                "{} on disk does not parse: {e}",
                tpl_path.display()
            ))
        })?;
        if parsed.name != prev.name {
            return Err(RpcError::Conflict(format!(
                "rename refused: spec name is `{}`, cannot become `{}`. \
                 Restore `name:` in template.yaml or submit a fresh job to rename.",
                prev.name, parsed.name,
            )));
        }

        commit_paths(
            &repo_path,
            &format!("update template: {} (chat)", parsed.name),
            std::slice::from_ref(&tpl_path),
        )
        .map_err(git_commit_err)?;

        job.template_yaml = Some(disk_yaml);
        self.bus
            .publish(
                Some(job.id),
                None,
                None,
                Event::JobTemplateUpdated { job_id: job.id },
                now_ms(),
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// Best-effort fetch of the job's spec for the chat preamble.
    /// Returns `None` when the session id is not a real job (e.g.
    /// footer-panel correlation ids), when the job lacks a parseable
    /// template, or when none of the spec files are present. Reads
    /// stay bounded by `MAX_CHAT_SPEC_BYTES` per file so a runaway
    /// SCOPE.md cannot blow out the model's context budget — large
    /// files are truncated with a marker rather than dropped, because
    /// even a partial spec is more useful than the agent fumbling
    /// from filesystem clues.
    async fn load_chat_job_spec(&self, session_id: codeless_types::JobId) -> Option<String> {
        let job = self.store.get_job(session_id).await.ok().flatten()?;
        let template_yaml = job.template_yaml.as_ref()?;
        let template = JobTemplate::parse_yaml(template_yaml).ok()?;
        let repo = self.store.get_repo(job.repo_id).await.ok().flatten()?;
        let job_dir = std::path::Path::new(&repo.local_path)
            .join(".codeless")
            .join("jobs")
            .join(&template.name);

        let mut out = String::new();
        out.push_str(&format!(
            "Active job: {} (id `{}`, status `{:?}`).\n\
             Spec lives at `.codeless/jobs/{}/` in the repo. The files \
             reproduced below are the source of truth for this job; \
             prefer them over anything else when answering.\n\n",
            template.name, session_id, job.status, template.name,
        ));
        out.push_str("## template.yaml\n\n```yaml\n");
        out.push_str(&truncate_for_chat(template_yaml));
        if !template_yaml.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n\n");

        for (label, filename) in [("SCOPE.md", "SCOPE.md"), ("WORKFLOW.md", "WORKFLOW.md")] {
            let path = job_dir.join(filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                out.push_str(&format!("## {label}\n\n"));
                out.push_str(&truncate_for_chat(&content));
                if !content.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
        }

        out.push_str(CHAT_JOB_SPEC_AUTHORING_PRIMER);
        Some(out)
    }
}

/// Tells the chat agent it owns the job's spec files and how to edit
/// them safely. Appended after the spec fold so the agent has the
/// current contents in mind before it reads the rules.
///
/// Disk is the source of truth at run-time: `start_job` / `resume_job`
/// re-parse `template.yaml` from disk and refresh the DB row before
/// transitioning to Queued, so direct filesystem edits land without a
/// separate "save" gesture. The agent must not touch `CHAT.md` — the
/// runtime appends to it on every turn.
/// Top-of-prompt banner the spec-mode preamble opens with. Mirrors
/// the claude-code plan-mode model: the user has explicitly flipped
/// the chat into "I am here to shape the job spec, not to run it,"
/// and the agent must respect that intent even when the conversation
/// drifts toward implementation details.
const SPEC_MODE_BANNER: &str = "# Spec mode (active)\n\n\
The user has flipped this chat into SPEC MODE. You are authoring the \
job's spec, not implementing it. Edit only files under \
`.codeless/jobs/<name>/` (template.yaml, SCOPE.md, WORKFLOW.md, and \
per-stage `*.md`). Do NOT edit repo source code, run shell commands, \
commit, push, or invoke the network. Your tool surface has been \
restricted to read + edit + write + grep on the worktree; calls to \
disallowed tools will fail.\n\n\
If the user asks you to implement something rather than describe it, \
remind them they are in spec mode and either (a) capture the request \
as a stage in `template.yaml` for them to run later, or (b) suggest \
they flip back to work mode.\n";

/// Tool list passed to the CLI wrapper via `CliCfg::allowed_tools`
/// when the chat turn is spec-mode. Comma-separated; entries match
/// the claude-code tool names that the wrapper recognises. Keep this
/// in sync with the banner above — if a tool is mentioned there as
/// "available" it must be in this list, and vice versa.
const SPEC_MODE_ALLOWED_TOOLS: &str = "Read,Edit,Write,Glob,Grep,LS,TodoWrite";

const CHAT_JOB_SPEC_AUTHORING_PRIMER: &str = "## Job-spec authoring\n\n\
You may edit this job's spec directly using your ambient `Edit`, `Write`, \
and `Read` tools on files under `.codeless/jobs/<name>/`:\n\n\
- `template.yaml` — name, goal, `stages[]`. The `name:` field is \
immutable; changing it will cause the next `start_job` to fail. Other \
edits land on the next run.\n\
- `SCOPE.md` — load-bearing scope, folded into every stage prompt.\n\
- `WORKFLOW.md` — per-stage protocol, end-of-stage gate, drift rules.\n\
- Per-stage `*.md` — referenced from `stages[i].docs:` and folded into \
that stage's prompt only.\n\n\
Do NOT touch `CHAT.md`; the runtime appends to it on every turn.\n\n\
When the user clicks **run**, the runtime re-parses `template.yaml` \
from disk into SQLite, so your edits take effect without any explicit \
save. A malformed `template.yaml` will surface as an `InvalidArgument` \
on `start_job` — keep YAML valid before handing back.\n";

/// Per-file byte budget when folding job spec files into the chat
/// preamble. Sized to leave room for two large files plus the user's
/// transcript without crowding the model's input window. Files larger
/// than this are truncated with a trailing marker.
const MAX_CHAT_SPEC_BYTES: usize = 8 * 1024;

fn truncate_for_chat(s: &str) -> std::borrow::Cow<'_, str> {
    if s.len() <= MAX_CHAT_SPEC_BYTES {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut cut = MAX_CHAT_SPEC_BYTES;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 64);
    out.push_str(&s[..cut]);
    out.push_str("\n\n[…truncated for chat preamble; read the full file from disk if needed…]\n");
    std::borrow::Cow::Owned(out)
}

/// Promote a legacy flat `<name>.yaml` to the directory layout. Two
/// separate commits — write the new file first, then delete the
/// flat YAML — so `git log` records the move as two atomic steps and
/// a crash between them leaves both files on disk, which `JobLayout`
/// surfaces as `FlatPreferred` and a retry resolves.
fn migrate_flat_to_directory(repo: &std::path::Path, name: &str) -> RpcResult<()> {
    let flat = flat_yaml_path(repo, name);
    let tpl = template_yaml_path(repo, name);
    let body = std::fs::read_to_string(&flat)
        .map_err(|e| RpcError::Internal(format!("read flat {}: {e}", flat.display())))?;
    if let Some(parent) = tpl.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RpcError::Internal(format!("create job dir {}: {e}", parent.display())))?;
    }
    std::fs::write(&tpl, &body)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", tpl.display())))?;
    commit_paths(
        repo,
        &format!("migrate template: {name} → directory layout"),
        &[tpl],
    )
    .map_err(git_commit_err)?;

    std::fs::remove_file(&flat)
        .map_err(|e| RpcError::Internal(format!("remove flat {}: {e}", flat.display())))?;
    commit_paths(
        repo,
        &format!("migrate template: {name} (remove flat YAML)"),
        &[flat],
    )
    .map_err(git_commit_err)?;
    Ok(())
}

fn filename_err(e: FilenameError) -> RpcError {
    match e {
        FilenameError::PathTraversal => {
            RpcError::InvalidArgument("filename contains path traversal".to_owned())
        }
        FilenameError::Dotfile => RpcError::InvalidArgument("dotfiles are not allowed".to_owned()),
        FilenameError::ReservedTemplateYaml => {
            RpcError::InvalidArgument("template.yaml is reserved; use the spec editor".to_owned())
        }
        FilenameError::Empty => RpcError::InvalidArgument("filename is empty".to_owned()),
    }
}

fn git_commit_err(e: GitCommitError) -> RpcError {
    RpcError::Internal(format!("git: {e}"))
}

fn fs_not_configured() -> RpcError {
    RpcError::Internal("fs.* not available: runtime has no filesystem root configured".to_owned())
}

/// Seed a fresh job directory at `<repo>/.codeless/jobs/<name>/` with
/// `template.yaml`, `SCOPE.md`, and `WORKFLOW.md`, and commit them in
/// a single commit. Called from `submit_job` so the user never has to
/// "promote" a prompt into a template — every UI submit lands a job
/// whose spec already exists on disk and is editable from the moment
/// the row appears in the dashboard.
///
/// Refuses (`Conflict`) if the directory already exists. Renaming is
/// out of scope here — submit a fresh job to use a different name.
/// `template.yaml` parse errors surface as `InvalidArgument` so the
/// UI can show the line/column inline; the runtime is the source of
/// truth for what counts as valid YAML.
fn seed_job_directory(repo_local_path: &str, template_yaml: &str) -> Result<(), RpcError> {
    let parsed = JobTemplate::parse_yaml(template_yaml)
        .map_err(|e| RpcError::InvalidArgument(format!("template parse: {e}")))?;

    let repo_path = std::path::PathBuf::from(repo_local_path);
    let dir = directory_path(&repo_path, &parsed.name);
    if dir.exists() {
        return Err(RpcError::Conflict(format!(
            "a job named `{}` already exists at {}; pick a different name",
            parsed.name,
            dir.display(),
        )));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| RpcError::Internal(format!("create job dir {}: {e}", dir.display())))?;

    let tpl_path = template_yaml_path(&repo_path, &parsed.name);
    std::fs::write(&tpl_path, template_yaml)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", tpl_path.display())))?;

    let scope_path = dir.join("SCOPE.md");
    std::fs::write(&scope_path, SCOPE_PRESET)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", scope_path.display())))?;

    let workflow_path = dir.join("WORKFLOW.md");
    std::fs::write(&workflow_path, WORKFLOW_PRESET)
        .map_err(|e| RpcError::Internal(format!("write {}: {e}", workflow_path.display())))?;

    commit_paths(
        &repo_path,
        &format!("scaffold job: {}", parsed.name),
        &[tpl_path, scope_path, workflow_path],
    )
    .map_err(git_commit_err)?;

    Ok(())
}

const SCOPE_PRESET: &str = "# Scope\n\n\
What this job is for. Replace this with what success looks like, what\n\
is out of scope, the constraints, and the deliverables.\n";

const WORKFLOW_PRESET: &str = "# Workflow\n\n\
How the agent should drive the work. Replace this with how to sequence\n\
the stages, what to verify between them, and what counts as done.\n";

/// Map host-side `FsError` to the wire `RpcError` so transports can
/// surface the right status code. Path-escape is `InvalidArgument`
/// rather than a 4xx-with-no-clue because the caller supplied a path
/// the server refused; non-utf8 is the same. IO errors map to
/// `Internal` (typically permission/disk).
fn fs_err(e: FsError) -> RpcError {
    match e {
        FsError::Escape(p) => RpcError::InvalidArgument(format!("path escapes root: {p}")),
        FsError::NotUtf8(p) => RpcError::InvalidArgument(format!("not a utf-8 text file: {p}")),
        FsError::BadRoot(p) => {
            RpcError::Internal(format!("fs root misconfigured: {}", p.display()))
        }
        FsError::Io(err) if err.kind() == std::io::ErrorKind::NotFound => {
            RpcError::NotFound(err.to_string())
        }
        FsError::Io(err) => RpcError::Internal(format!("fs io: {err}")),
    }
}

/// Translate `GitDiffError` into wire errors. Missing-ref cases map to
/// `NotFound` so the UI's files-changed tab can render a "no diff
/// available" empty state rather than an error toast — the most
/// common cause is "the job ran without a worktree provisioned" and
/// that's expected, not exceptional.
fn diff_err(e: GitDiffError) -> RpcError {
    match e {
        GitDiffError::BaseMissing(b) => RpcError::NotFound(format!("base ref {b}")),
        GitDiffError::HeadMissing(h) => RpcError::NotFound(format!("head ref {h}")),
        GitDiffError::Io(err) => RpcError::Internal(format!("git io: {err}")),
        GitDiffError::GitFailed { op, status, stderr } => {
            RpcError::Internal(format!("git {op} failed ({status}): {stderr}"))
        }
    }
}
