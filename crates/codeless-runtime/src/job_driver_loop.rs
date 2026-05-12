//! Background loop that drives queued jobs to completion. The CLI's
//! local-mode path calls `drive_job` inline on a single job; the
//! hosted server has no such single-shot invocation, so a long-lived
//! task here subscribes to the event bus and dispatches `drive_job`
//! for every `JobQueued` event.
//!
//! Scope is deliberately minimal so the demo works without
//! introducing a heavyweight scheduler:
//!
//! - At startup the loop replays the `Queued` jobs already in the
//!   DB (re-queued by the lease reaper at runtime construction).
//! - It then subscribes live for new `JobQueued` events.
//! - Each job is run in its own spawned task; concurrency is bounded
//!   by `concurrency` (a tokio `Semaphore`).
//! - Worktree provisioning is left to the future; the loop passes
//!   `None` to `drive_job`, matching the CLI's `codeless run` path
//!   today. SCOPE.md's "Worktrees" deliverable is a separate phase.
//!
//! Runner selection goes through a `RunnerFactory` trait so the
//! server binary can choose which adapters to wire in without
//! depending on every implementation transitively.

use std::sync::Arc;

use codeless_adapters_host::WorktreeManager;
use codeless_rpc::RpcError;
use codeless_types::{Event, Job, JobStatus};
use futures_util::StreamExt;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::driver::drive_job;
use crate::event_bus::SubscribeFilter;
use crate::rpc::InProcessRpc;
use crate::runner::Runner;

/// Resolves a queued `Job` to a concrete `Runner` implementation.
/// The factory sees the whole row so it can read `job.runner`,
/// `job.prompt`, and (eventually) other per-job knobs without an
/// extra DB round trip. Returning `None` means "this runner isn't
/// enabled on this core"; the driver loop logs and the job remains
/// `Queued` for an operator to fix (re-submit with a different
/// runner, or restart the server with the runner enabled).
pub trait RunnerFactory: Send + Sync + 'static {
    fn build(&self, job: &Job) -> Option<Arc<dyn Runner>>;
}

/// Handle to the running driver loop. Drop semantics: the loop runs
/// until the underlying event-bus subscription closes or until
/// `cancel()` is called; the join handle resolves shortly after
/// either trigger.
pub struct DriverLoopHandle {
    cancel: CancellationToken,
    join: JoinHandle<()>,
}

impl DriverLoopHandle {
    /// Politely ask the loop to stop. The in-flight jobs each have
    /// their own driver-owned cancellation token (the cap watcher's);
    /// stopping the loop does not abort them. Use `join` afterwards
    /// to wait for the subscription drain.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        self.join.await
    }
}

/// Spawn the driver. Replays existing `Queued` jobs once, then tails
/// `JobQueued` events live until cancelled or the bus shuts down.
///
/// `concurrency` caps the number of in-flight `drive_job` tasks. A
/// small bound is fine in MVP — each `drive_job` is a long-lived
/// future that owns its own task, so 4 in-flight is plenty until the
/// server gets a real scheduler.
pub async fn spawn_job_driver_loop<F: RunnerFactory>(
    rpc: Arc<InProcessRpc>,
    factory: Arc<F>,
    worktrees: Option<Arc<WorktreeManager>>,
    concurrency: usize,
) -> Result<DriverLoopHandle, RpcError> {
    let cancel = CancellationToken::new();
    let token_for_task = cancel.clone();
    let bus = rpc.bus().clone();

    // Live subscription opens before backlog scan so any job queued
    // *during* the scan is picked up by the live tail without races.
    let mut stream = bus
        .subscribe_since(SubscribeFilter::All, None)
        .await
        .map_err(|e| RpcError::Internal(format!("driver subscribe: {e}")))?;

    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));

    let join = tokio::spawn(
        async move {
            // Drive whatever is already `Queued` on disk. The runtime's
            // startup lease reaper has already converted abandoned
            // `Running` rows back to `Queued`, so a single pass here
            // covers crashes.
            replay_backlog(&rpc, &factory, &worktrees, &semaphore).await;

            // Live tail. `subscribe_since(All, None)` is live-only,
            // which is what we want — backlog was just handled above.
            loop {
                tokio::select! {
                    _ = token_for_task.cancelled() => break,
                    item = stream.next() => {
                        let env = match item {
                            Some(Ok(env)) => env,
                            Some(Err(e)) => {
                                tracing::warn!(error = %e, "driver loop stream error");
                                continue;
                            }
                            None => break,
                        };
                        if let Event::JobQueued { job_id, .. } = env.event {
                            dispatch(
                                rpc.clone(),
                                factory.clone(),
                                worktrees.clone(),
                                semaphore.clone(),
                                job_id,
                            )
                            .await;
                        }
                    }
                }
            }
        }
        .instrument(tracing::info_span!("job_driver_loop")),
    );

    Ok(DriverLoopHandle { cancel, join })
}

async fn replay_backlog<F: RunnerFactory>(
    rpc: &Arc<InProcessRpc>,
    factory: &Arc<F>,
    worktrees: &Option<Arc<WorktreeManager>>,
    semaphore: &Arc<Semaphore>,
) {
    let jobs = match rpc.store().list_jobs(None).await {
        Ok(jobs) => jobs,
        Err(e) => {
            tracing::warn!(error = %e, "driver backlog scan failed");
            return;
        }
    };
    for job in jobs.into_iter().filter(|j| j.status == JobStatus::Queued) {
        dispatch(
            rpc.clone(),
            factory.clone(),
            worktrees.clone(),
            semaphore.clone(),
            job.id,
        )
        .await;
    }
}

async fn dispatch<F: RunnerFactory>(
    rpc: Arc<InProcessRpc>,
    factory: Arc<F>,
    worktrees: Option<Arc<WorktreeManager>>,
    semaphore: Arc<Semaphore>,
    job_id: codeless_types::JobId,
) {
    let job = match rpc.store().get_job(job_id).await {
        Ok(Some(job)) => job,
        Ok(None) => {
            tracing::warn!(%job_id, "driver: queued job not found");
            return;
        }
        Err(e) => {
            tracing::warn!(%job_id, error = %e, "driver: get_job failed");
            return;
        }
    };
    if job.status != JobStatus::Queued {
        // Already picked up by another path (CLI's `codeless run`,
        // a previous tick of this loop, etc.). The state machine
        // would reject the transition anyway; bail early so the
        // semaphore isn't pointlessly held.
        return;
    }
    let runner = match factory.build(&job) {
        Some(r) => r,
        None => {
            tracing::warn!(
                %job_id,
                runner = %job.runner,
                "driver: runner not enabled — job stays queued",
            );
            return;
        }
    };
    // Each in-flight drive is its own task so the subscription loop
    // never blocks. The semaphore caps how many run at once; the
    // permit is held for the lifetime of the spawned task.
    let permit = match semaphore.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => {
            tracing::warn!(%job_id, "driver: semaphore closed");
            return;
        }
    };
    tokio::spawn(async move {
        let _permit = permit;
        if let Err(e) = drive_job(&rpc, job_id, runner, worktrees).await {
            tracing::warn!(%job_id, error = %e, "drive_job returned error");
        }
    });
}
