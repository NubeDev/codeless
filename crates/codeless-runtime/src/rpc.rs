use std::sync::Arc;

use async_trait::async_trait;
use codeless_adapters_host::{diff_against, FsError, GitDiffError, HostFs};
use codeless_rpc::{
    AddRepoArgs, ApproveReviewArgs, CommentReviewArgs, EventFilter, EventStream, FsCwdResult,
    FsReadDirArgs, FsReadDirResult, FsReadFileArgs, FsReadFileResult, FsStatArgs, FsStatResult,
    FsWriteFileArgs, GetJobArgs, JobDiffArgs, JobDiffFile, JobDiffResult, ListJobsArgs,
    ListJobsResult, ListReposResult, ListReviewsArgs, ListReviewsResult, RemoveRepoArgs, RpcError,
    RpcResult, RpcServer, Since, StopJobArgs, StopReviewArgs, SubmitJobArgs,
};
use codeless_types::{
    CostCents, Event, Job, JobId, JobStatus, Repo, RepoId, Review, ReviewStatus, StopReason,
};
use sqlx::SqlitePool;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::migrations::MIGRATOR;
use crate::store::SqliteStore;
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
        })
    }

    /// Attach a filesystem adapter so the `fs_*` RPC surface becomes
    /// live. Without this call those methods return `Internal` so
    /// transports get a typed failure rather than a panic.
    pub fn with_fs(mut self, fs: Arc<HostFs>) -> Self {
        self.fs = Some(fs);
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
        if self
            .store
            .get_repo(args.repo_id)
            .await
            .map_err(db_err)?
            .is_none()
        {
            return Err(RpcError::NotFound(format!("repo {}", args.repo_id)));
        }
        let now = now_ms();
        let job = Job {
            id: JobId::new(),
            repo_id: args.repo_id,
            status: JobStatus::Queued,
            stop_reason: None,
            template_yaml: args.template_yaml,
            prompt: args.prompt,
            runner: args.runner,
            branch: args.branch,
            worktree_path: None,
            cost_cap_cents: CostCents(args.cost_cap_cents),
            wall_clock_cap_ms: args.wall_clock_cap_ms,
            cost_cents: CostCents::ZERO,
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
}

fn fs_not_configured() -> RpcError {
    RpcError::Internal("fs.* not available: runtime has no filesystem root configured".to_owned())
}

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
