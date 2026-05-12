use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codeless_adapters_host::WorktreeManager;
use codeless_rpc::{RpcError, RpcResult};
use codeless_types::{Event, JobId, JobStatus, StopReason};
use futures_core::Stream;
use tokio_util::sync::CancellationToken;

use crate::event_bus::{EventBus, SubscribeFilter};
use crate::rpc::InProcessRpc;
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::state_machine::{is_terminal_job, transition_job};
use crate::store::SqliteStore;
use crate::time::now_ms;

fn db_err(e: sqlx::Error) -> RpcError {
    RpcError::Internal(format!("db: {e}"))
}

/// Drive a queued job to a terminal state. Owns the surrounding
/// `Job` row transitions, the framing events, and (when a
/// `WorktreeManager` is supplied) the per-job `git worktree`
/// lifecycle. The runner is responsible only for whatever stage/task
/// /AI events its run actually produces.
///
/// State machine, in one place so the next reader does not have to
/// hunt it down:
///
/// 1. Look up the job. It must exist and be `Queued`. A repeat call
///    on a non-`Queued` job returns `Conflict` — drivers do not retry
///    in-place; the surrounding scheduler resubmits.
/// 2. If `worktrees` is supplied, look up the repo, create a fresh
///    `git worktree` at `<base>/job-<id>`, and persist its path on
///    the job row so a reaper after a crash has somewhere to look.
/// 3. Move `Queued -> Running`, stamp `started_at`, emit `job-started`.
/// 4. Invoke `runner.run(ctx)` with the worktree path threaded into
///    the context. Whatever the runner publishes lands on the bus
///    as-is. The runner does **not** transition the job row itself.
/// 5. Translate `RunnerOutcome` to `Running -> Completed | Failed`,
///    stamp `ended_at`, emit the terminal event.
/// 6. If a worktree was created, remove it. Removal is best-effort —
///    a `tracing::warn!` records failures so a leaked tree is visible
///    in logs but does not poison the job's terminal status.
///
/// `Stopped` is not reachable from here — that path is the explicit
/// `stop_job` RPC, which races this driver via the store. If the job
/// became `Stopped` while the runner was working, the post-run
/// transition guard refuses the move and the driver silently exits;
/// the `stop_job` event has already been published. The worktree is
/// still removed in that case so the stop path matches the completed
/// and failed paths.
///
/// `worktrees` is `Option<Arc<_>>` so the test harness can drive
/// jobs without provisioning a real repo on disk. Production wiring
/// always passes `Some(_)`.
#[tracing::instrument(
    name = "drive_job",
    skip_all,
    fields(job_id = %job_id),
)]
pub async fn drive_job(
    rpc: &InProcessRpc,
    job_id: JobId,
    runner: Arc<dyn Runner>,
    worktrees: Option<Arc<WorktreeManager>>,
) -> RpcResult<()> {
    let store: &Arc<SqliteStore> = rpc.store();
    let bus: &Arc<EventBus> = rpc.bus();

    let mut job = store
        .get_job(job_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("job {job_id}")))?;
    transition_job(job.status, JobStatus::Running)
        .map_err(|e| RpcError::Conflict(e.to_string()))?;

    let provisioned = match worktrees.as_ref() {
        Some(mgr) => Some(provision_worktree(mgr, store, &mut job).await?),
        None => None,
    };

    let started = now_ms();
    job.status = JobStatus::Running;
    job.started_at = Some(started);
    store.update_job(&job).await.map_err(db_err)?;
    tracing::info!(status = "running", "job started");
    bus.publish(
        Some(job.id),
        None,
        None,
        Event::JobStarted { job_id: job.id },
        started,
    )
    .await
    .map_err(db_err)?;

    let cancel = CancellationToken::new();
    let cap_watcher = spawn_cap_watcher(
        Arc::clone(store),
        Arc::clone(bus),
        job_id,
        job.cost_cap_cents.0,
        job.wall_clock_cap_ms,
        cancel.clone(),
    )
    .await
    .map_err(db_err)?;

    let outcome = runner
        .run(RunnerContext {
            job_id,
            bus: Arc::clone(bus),
            worktree_path: provisioned.as_ref().map(|p| p.worktree.clone()),
            cancel: cancel.clone(),
        })
        .await;
    cap_watcher.abort();

    let Some(current) = store.get_job(job_id).await.map_err(db_err)? else {
        return Err(RpcError::NotFound(format!("job {job_id}")));
    };
    if is_terminal_job(current.status) {
        tracing::info!(status = ?current.status, "runner returned after stop");
        if let Some(p) = provisioned.as_ref() {
            release_worktree(p);
        }
        return Ok(());
    }

    let (next_status, event) = match outcome {
        RunnerOutcome::Completed => (JobStatus::Completed, Event::JobCompleted { job_id: job.id }),
        RunnerOutcome::Failed { reason: _ } => {
            (JobStatus::Failed, Event::JobFailed { job_id: job.id })
        }
    };
    transition_job(current.status, next_status).map_err(|e| RpcError::Conflict(e.to_string()))?;

    let ended = now_ms();
    let mut updated = current;
    updated.status = next_status;
    updated.ended_at = Some(ended);
    store.update_job(&updated).await.map_err(db_err)?;
    tracing::info!(status = ?next_status, "job terminal");
    bus.publish(Some(job_id), None, None, event, ended)
        .await
        .map_err(db_err)?;
    if let Some(p) = provisioned.as_ref() {
        release_worktree(p);
    }
    Ok(())
}

/// Records the per-run state the driver needs in order to release a
/// worktree at the end. Held only for the duration of a single
/// `drive_job` call; not exposed on the public API.
struct ProvisionedWorktree {
    manager: Arc<WorktreeManager>,
    repo_path: PathBuf,
    worktree: PathBuf,
}

async fn provision_worktree(
    manager: &Arc<WorktreeManager>,
    store: &Arc<SqliteStore>,
    job: &mut codeless_types::Job,
) -> RpcResult<ProvisionedWorktree> {
    let repo = store
        .get_repo(job.repo_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
    let repo_path = PathBuf::from(&repo.local_path);
    let handle = manager
        .create(&repo_path, &job.id.to_string())
        .map_err(|e| RpcError::Internal(format!("worktree create: {e}")))?;
    job.worktree_path = Some(handle.path.to_string_lossy().into_owned());
    store.update_job(job).await.map_err(db_err)?;
    Ok(ProvisionedWorktree {
        manager: Arc::clone(manager),
        repo_path,
        worktree: handle.path,
    })
}

fn release_worktree(p: &ProvisionedWorktree) {
    if let Err(e) = p.manager.remove(&p.repo_path, &p.worktree) {
        tracing::warn!(
            error = %e,
            worktree = %p.worktree.display(),
            "failed to remove worktree on terminal status; leaked on disk",
        );
    }
}

/// Concurrent watcher that races the runner against the per-job cost
/// cap and wall-clock cap. Wakes on every `AiMessageComplete` (cost
/// is rolled up by `EventBus::publish` first, so the job row is
/// already up-to-date by the time we observe the event) and on the
/// wall-clock deadline. Firing either cap moves the job to `Stopped`
/// with the appropriate `StopReason`, publishes `JobStopped`, and
/// triggers `cancel.cancel()` so the runner tears down. A cap value
/// of `0` is treated as "unlimited" — the watcher loops past it
/// without firing, which matches the existing `submit_job` test
/// callers that pass `cost_cap_cents: 0` to mean "don't enforce".
async fn spawn_cap_watcher(
    store: Arc<SqliteStore>,
    bus: Arc<EventBus>,
    job_id: JobId,
    cost_cap: i64,
    wall_clock_ms: i64,
    cancel: CancellationToken,
) -> sqlx::Result<tokio::task::JoinHandle<()>> {
    let stream = bus
        .subscribe_since(SubscribeFilter::Job(job_id), None)
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("subscribe: {e}")))?;
    let handle = tokio::spawn(watch_caps(
        store,
        bus,
        job_id,
        cost_cap,
        wall_clock_ms,
        cancel,
        stream,
    ));
    Ok(handle)
}

async fn watch_caps(
    store: Arc<SqliteStore>,
    bus: Arc<EventBus>,
    job_id: JobId,
    cost_cap: i64,
    wall_clock_ms: i64,
    cancel: CancellationToken,
    mut stream: std::pin::Pin<
        Box<dyn Stream<Item = Result<codeless_types::EventEnvelope, RpcError>> + Send>,
    >,
) {
    use tokio_stream::StreamExt;

    let wall_clock_sleep = if wall_clock_ms > 0 {
        Some(tokio::time::sleep(Duration::from_millis(
            wall_clock_ms as u64,
        )))
    } else {
        None
    };
    tokio::pin!(wall_clock_sleep);

    loop {
        let next_item: futures_core::future::BoxFuture<'_, _> = Box::pin(stream.next());
        let stop_reason = tokio::select! {
            biased;
            _ = async {
                match wall_clock_sleep.as_mut().as_pin_mut() {
                    Some(s) => s.await,
                    None => std::future::pending::<()>().await,
                }
            } => Some(StopReason::WallClock),
            item = next_item => {
                match item {
                    Some(Ok(env)) if matches!(env.event, Event::AiMessageComplete { .. }) && cost_cap > 0 => {
                        match store.get_job(job_id).await {
                            Ok(Some(j)) if j.cost_cents.0 >= cost_cap => Some(StopReason::CostCap),
                            _ => None,
                        }
                    }
                    Some(_) => None,
                    None => return,
                }
            }
        };
        if let Some(reason) = stop_reason {
            fire_stop(&store, &bus, job_id, reason, &cancel).await;
            return;
        }
    }
}

async fn fire_stop(
    store: &Arc<SqliteStore>,
    bus: &Arc<EventBus>,
    job_id: JobId,
    reason: StopReason,
    cancel: &CancellationToken,
) {
    let Ok(Some(mut job)) = store.get_job(job_id).await else {
        cancel.cancel();
        return;
    };
    if is_terminal_job(job.status) {
        cancel.cancel();
        return;
    }
    let ended = now_ms();
    job.status = JobStatus::Stopped;
    job.stop_reason = Some(reason);
    job.ended_at = Some(ended);
    if let Err(e) = store.update_job(&job).await {
        tracing::warn!(error = %e, "cap watcher: update_job failed");
    }
    if let Err(e) = bus
        .publish(
            Some(job_id),
            None,
            None,
            Event::JobStopped { job_id, reason },
            ended,
        )
        .await
    {
        tracing::warn!(error = %e, "cap watcher: publish JobStopped failed");
    }
    cancel.cancel();
}
