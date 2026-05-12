use std::sync::Arc;

use async_trait::async_trait;
use codeless_rpc::{
    AddRepoArgs, EventFilter, EventStream, GetJobArgs, ListJobsArgs, ListJobsResult,
    ListReposResult, RemoveRepoArgs, RpcError, RpcResult, RpcServer, Since, StopJobArgs,
    SubmitJobArgs,
};
use codeless_types::{CostCents, Event, Job, JobId, JobStatus, Repo, RepoId, StopReason};
use sqlx::SqlitePool;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::migrations::MIGRATOR;
use crate::store::SqliteStore;
use crate::time::now_ms;

/// In-process `RpcServer`. The CLI's `codeless run --once` path talks
/// to this directly without serialising over a wire; the same struct
/// is what the hosted server will hand to `axum` handlers in a later
/// phase. Repo and job rows live in SQLite (`SqliteStore`); the event
/// bus is still in-memory broadcast — `since`-cursor replay against
/// the persisted event log lands in a follow-up stage.
pub struct InProcessRpc {
    store: Arc<SqliteStore>,
    bus: Arc<EventBus>,
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

    /// Build a runtime around a caller-supplied pool. Migrations are
    /// applied here so a fresh database file works the same as a
    /// pre-migrated one and the caller never has to remember to run
    /// the migrator separately. Forward-only migration semantics —
    /// see `migrations::MIGRATOR`.
    pub async fn with_db(pool: SqlitePool) -> Result<Self, sqlx::Error> {
        MIGRATOR.run(&pool).await?;
        Ok(Self {
            store: Arc::new(SqliteStore::new(pool)),
            bus: Arc::new(EventBus::new(DEFAULT_EVENT_BUFFER)),
        })
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
            .publish(None, None, None, Event::RepoAdded { repo_id: repo.id }, now);
        Ok(repo)
    }

    async fn remove_repo(&self, args: RemoveRepoArgs) -> RpcResult<()> {
        let removed = self.store.remove_repo(args.repo_id).await.map_err(db_err)?;
        if !removed {
            return Err(RpcError::NotFound(format!("repo {}", args.repo_id)));
        }
        self.bus.publish(
            None,
            None,
            None,
            Event::RepoRemoved {
                repo_id: args.repo_id,
            },
            now_ms(),
        );
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
        self.bus.publish(
            Some(job.id),
            None,
            None,
            Event::JobQueued {
                job_id: job.id,
                repo_id: job.repo_id,
            },
            now,
        );
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
        self.bus.publish(
            Some(job.id),
            None,
            None,
            Event::JobStopped {
                job_id: job.id,
                reason: StopReason::User,
            },
            now,
        );
        Ok(())
    }

    async fn subscribe(&self, filter: EventFilter, since: Since) -> RpcResult<EventStream> {
        if since.is_some() {
            return Err(RpcError::Conflict(
                "since-cursor replay is not implemented until the SQLite event log lands".into(),
            ));
        }
        let local = match filter {
            EventFilter::All => SubscribeFilter::All,
            EventFilter::Job { job_id } => SubscribeFilter::Job(job_id),
        };
        Ok(self.bus.subscribe(local))
    }
}
