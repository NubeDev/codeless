use codeless_types::{JobStatus, StageStatus, TaskStatus};
use thiserror::Error;

/// Refused transition. The driver treats this as a programming error
/// (the state machine guarded against an illegal move), not a user
/// error — surface paths convert it to `RpcError::Conflict` when the
/// runtime is asked to act on a job that is already terminal.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("illegal transition: {kind} {from:?} -> {to:?}")]
pub struct TransitionError {
    pub kind: &'static str,
    pub from: String,
    pub to: String,
}

fn deny<F: std::fmt::Debug, T: std::fmt::Debug>(
    kind: &'static str,
    from: F,
    to: T,
) -> TransitionError {
    TransitionError {
        kind,
        from: format!("{from:?}"),
        to: format!("{to:?}"),
    }
}

/// Job lifecycle guard. Allowed edges:
/// - `Queued -> Running | Stopped`
/// - `Draft -> Queued | Stopped`
/// - `Queued -> Running | Stopped`
/// - `Running -> AwaitingReview | Completed | Failed | Stopped`
/// - `AwaitingReview -> Running | Completed | Stopped`
///
/// `Draft` is the editable pre-run holding state — the user has
/// submitted the job but not asked the driver to run it yet.
/// Terminal states (`Completed`, `Failed`, `Stopped`) accept no further
/// transitions; the runtime reports `Conflict` when asked. This matches
/// the lifecycle described in SCOPE.md "Runtime / scheduler".
pub fn transition_job(from: JobStatus, to: JobStatus) -> Result<(), TransitionError> {
    use JobStatus::*;
    let ok = matches!(
        (from, to),
        (Draft, Queued)
            | (Draft, Stopped)
            | (Queued, Running)
            | (Queued, Stopped)
            | (Running, AwaitingReview)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Stopped)
            | (AwaitingReview, Running)
            | (AwaitingReview, Completed)
            | (AwaitingReview, Stopped),
    );
    if ok {
        Ok(())
    } else {
        Err(deny("job", from, to))
    }
}

/// Stage lifecycle guard. Mirrors `transition_job` for the per-stage
/// state column. `AwaitingReview` is the verify-gated pause; a human (or
/// auto-approver) sends it to `Passed` or back to `Running`.
pub fn transition_stage(from: StageStatus, to: StageStatus) -> Result<(), TransitionError> {
    use StageStatus::*;
    let ok = matches!(
        (from, to),
        (Pending, Running)
            | (Running, AwaitingReview)
            | (Running, Passed)
            | (Running, Failed)
            | (AwaitingReview, Running)
            | (AwaitingReview, Passed)
            | (AwaitingReview, Failed),
    );
    if ok {
        Ok(())
    } else {
        Err(deny("stage", from, to))
    }
}

/// Task lifecycle guard. `Enqueued -> Running -> Completed | Failed`,
/// with `Cancelled` reachable from any non-terminal state when the job
/// is stopped mid-flight.
pub fn transition_task(from: TaskStatus, to: TaskStatus) -> Result<(), TransitionError> {
    use TaskStatus::*;
    let ok = matches!(
        (from, to),
        (Enqueued, Running)
            | (Enqueued, Cancelled)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Cancelled),
    );
    if ok {
        Ok(())
    } else {
        Err(deny("task", from, to))
    }
}

pub fn is_terminal_job(s: JobStatus) -> bool {
    matches!(
        s,
        JobStatus::Completed | JobStatus::Failed | JobStatus::Stopped
    )
}
