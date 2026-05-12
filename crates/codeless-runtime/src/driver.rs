use std::sync::Arc;

use codeless_rpc::{RpcError, RpcResult};
use codeless_types::{Event, JobId, JobStatus};

use crate::event_bus::EventBus;
use crate::rpc::InProcessRpc;
use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::state_machine::{is_terminal_job, transition_job};
use crate::store::MemoryStore;
use crate::time::now_ms;

/// Drive a queued job to a terminal state. Owns the surrounding
/// `Job` row transitions and the framing events; the runner is
/// responsible only for whatever stage/task/AI events its run
/// actually produces.
///
/// State machine, in one place so the next reader does not have to
/// hunt it down:
///
/// 1. Look up the job. It must exist and be `Queued`. A repeat call
///    on a non-`Queued` job returns `Conflict` — drivers do not retry
///    in-place; the surrounding scheduler resubmits.
/// 2. Move `Queued -> Running`, stamp `started_at`, emit `job-started`.
/// 3. Invoke `runner.run(ctx)`. Whatever the runner publishes lands on
///    the bus as-is. The runner does **not** transition the job row
///    itself.
/// 4. Translate `RunnerOutcome` to `Running -> Completed | Failed`,
///    stamp `ended_at`, emit the terminal event.
///
/// `Stopped` is not reachable from here — that path is the explicit
/// `stop_job` RPC, which races this driver via the store. If the job
/// became `Stopped` while the runner was working, the post-run
/// transition guard refuses the move and the driver silently exits;
/// the `stop_job` event has already been published.
pub async fn drive_job(
    rpc: &InProcessRpc,
    job_id: JobId,
    runner: Arc<dyn Runner>,
) -> RpcResult<()> {
    let store: &Arc<MemoryStore> = rpc.store();
    let bus: &Arc<EventBus> = rpc.bus();

    let mut job = store
        .get_job(job_id)
        .ok_or_else(|| RpcError::NotFound(format!("job {job_id}")))?;
    transition_job(job.status, JobStatus::Running)
        .map_err(|e| RpcError::Conflict(e.to_string()))?;

    let started = now_ms();
    job.status = JobStatus::Running;
    job.started_at = Some(started);
    store.update_job(job.clone());
    bus.publish(
        Some(job.id),
        None,
        None,
        Event::JobStarted { job_id: job.id },
        started,
    );

    let outcome = runner
        .run(RunnerContext {
            job_id,
            bus: Arc::clone(bus),
        })
        .await;

    let Some(current) = store.get_job(job_id) else {
        return Err(RpcError::NotFound(format!("job {job_id}")));
    };
    if is_terminal_job(current.status) {
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
    store.update_job(updated);
    bus.publish(Some(job_id), None, None, event, ended);
    Ok(())
}
