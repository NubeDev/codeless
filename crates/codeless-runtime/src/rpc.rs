use std::sync::Arc;

use async_trait::async_trait;
use codeless_adapters_host::{
    commit_paths, diff_against, FsError, GitCommitError, GitDiffError, HostFs, WorktreeManager,
};
use codeless_rpc::{
    AddRepoArgs, AgentChatArgs, AgentChatResult, ApproveReviewArgs, CommentReviewArgs,
    DeleteJobFileArgs, EventFilter, EventStream, FsCwdResult, FsReadDirArgs, FsReadDirResult,
    FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult, FsWriteFileArgs, GcWorktreeEntry,
    GcWorktreesArgs, GcWorktreesResult, GetJobArgs, JobDiffArgs, JobDiffFile, JobDiffResult,
    JobFileEntry, ListJobFilesArgs, ListJobFilesResult, ListJobsArgs, ListJobsResult,
    ListReposResult, ListReviewsArgs, ListReviewsResult, ListStagesArgs, ListStagesResult,
    ReadJobFileArgs, ReadJobFileResult, RemoveRepoArgs, RerunJobArgs, RpcError, RpcResult,
    RpcServer, Since, StartJobArgs, StopJobArgs, StopReviewArgs, SubmitJobArgs,
    UpdateJobTemplateArgs, UpdateJobTemplateResult, UploadChatAttachmentArgs,
    UploadChatAttachmentResult, WriteHandoverArgs, WriteHandoverResult, WriteJobFileArgs,
    WriteJobFileResult,
};
use codeless_types::{
    CostCents, Event, Job, JobId, JobStatus, Repo, RepoId, Review, ReviewStatus, StopReason,
};
use sqlx::SqlitePool;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::job_dir::{
    self, directory_path, flat_yaml_path, sanitise_filename, template_yaml_path, FilenameError,
    JobLayout,
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
}

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
            status: JobStatus::Queued,
            stop_reason: None,
            template_yaml: source.template_yaml,
            prompt: source.prompt,
            runner: source.runner,
            branch: String::new(),
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
                let allowed = self
                    .fs
                    .as_ref()
                    .map(|fs| fs.is_path_allowed(&canon))
                    .unwrap_or(false);
                if !allowed {
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
        let prompt = build_chat_prompt(args.context.as_ref(), &args.prompt);

        // Detached: the call returns once the runner has been spawned;
        // its tokens / tool-calls / completion event flow back through
        // the bus, keyed by `session_id` so the caller's subscribe
        // filter matches them. A panicked task only kills the chat
        // turn — log it and let other turns continue.
        tokio::spawn(async move {
            let publish = move |event: codeless_types::Event| {
                let bus = Arc::clone(&bus);
                async move {
                    bus.publish(Some(session_id), None, Some(task_id), event, now_ms())
                        .await
                        .map(|_| ())
                }
            };
            let cancel = tokio_util::sync::CancellationToken::new();
            if let Err(e) = codeless_adapters_host::run_chat(
                registry, provider, prompt, cwd, task_id, publish, cancel,
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
                base64::engine::general_purpose::STANDARD_NO_PAD
                    .decode(args.content_b64.as_bytes())
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
fn build_chat_prompt(ctx: Option<&codeless_rpc::ChatContext>, prompt: &str) -> String {
    let Some(ctx) = ctx else {
        return prompt.to_owned();
    };
    let has_any = ctx.ui_location.is_some()
        || ctx.selection.is_some()
        || !ctx.attachments.is_empty()
        || !ctx.user_prompts.is_empty();
    if !has_any {
        return prompt.to_owned();
    }

    let mut out = String::new();
    out.push_str("# Context\n\n");
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
    /// `template.name`. The job-file surface is template-only: a raw
    /// `prompt`-only job has no directory to read from, so it gets
    /// `InvalidArgument` rather than an empty list. `NotFound` covers
    /// unknown job or repo ids.
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
        let yaml = job.template_yaml.as_ref().ok_or_else(|| {
            RpcError::InvalidArgument(format!(
                "job {job_id} has no template; file surface is template-only"
            ))
        })?;
        let template = JobTemplate::parse_yaml(yaml)
            .map_err(|e| RpcError::InvalidArgument(format!("job {job_id} template parse: {e}")))?;
        let repo = self
            .store
            .get_repo(job.repo_id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
        Ok((std::path::PathBuf::from(repo.local_path), template.name))
    }
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
