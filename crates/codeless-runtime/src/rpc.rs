use std::sync::Arc;

use async_trait::async_trait;
use codeless_rpc::{
    AddRepoArgs, EventFilter, EventStream, GetJobArgs, ListJobsArgs, ListJobsResult,
    ListReposResult, RemoveRepoArgs, RpcError, RpcResult, RpcServer, Since, StopJobArgs,
    SubmitJobArgs,
};
use codeless_types::{CostCents, Event, Job, JobId, JobStatus, Repo, RepoId, StopReason};

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::store::MemoryStore;
use crate::time::now_ms;

/// In-process `RpcServer`. The CLI's `codeless run --once` path
/// (stage 10) talks to this directly without serialising over a wire;
/// the same struct is what the hosted server (Phase 3) will hand to
/// `axum` handlers.
///
/// `since` replay is not implemented yet — the event bus is in-memory
/// only, so `Some(_)` cursors return `Conflict` until stage 4 wires the
/// SQLite event log. Live subscriptions work today.
pub struct InProcessRpc {
    store: Arc<MemoryStore>,
    bus: Arc<EventBus>,
}

impl InProcessRpc {
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn with_capacity(event_buffer: usize) -> Self {
        Self {
            store: Arc::new(MemoryStore::new()),
            bus: Arc::new(EventBus::new(event_buffer)),
        }
    }

    pub fn store(&self) -> &Arc<MemoryStore> {
        &self.store
    }

    pub fn bus(&self) -> &Arc<EventBus> {
        &self.bus
    }
}

impl Default for InProcessRpc {
    fn default() -> Self {
        Self::new()
    }
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
        self.store.insert_repo(repo.clone());
        self.bus
            .publish(None, None, None, Event::RepoAdded { repo_id: repo.id }, now);
        Ok(repo)
    }

    async fn remove_repo(&self, args: RemoveRepoArgs) -> RpcResult<()> {
        if !self.store.remove_repo(args.repo_id) {
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
            repos: self.store.list_repos(),
        })
    }

    async fn submit_job(&self, args: SubmitJobArgs) -> RpcResult<Job> {
        if self.store.get_repo(args.repo_id).is_none() {
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
        self.store.insert_job(job.clone());
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
            .ok_or_else(|| RpcError::NotFound(format!("job {}", args.job_id)))
    }

    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult> {
        Ok(ListJobsResult {
            jobs: self.store.list_jobs(args.repo_id),
        })
    }

    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()> {
        let Some(mut job) = self.store.get_job(args.job_id) else {
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
        self.store.update_job(job.clone());
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
