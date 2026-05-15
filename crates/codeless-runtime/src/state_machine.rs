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
            // Driver give-up edge. Used by `job_driver_loop` when
            // `drive_job` keeps erroring before the row ever reaches
            // Running — the retry budget is exhausted (retryable
            // failures) or the error is unrecoverable (runner not
            // enabled, template parse). `stop_reason = RunnerCrash`
            // is recorded on the row so the UI distinguishes this
            // from a clean user-driven Failed state.
            | (Queued, Failed)
            | (Running, AwaitingReview)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, Stopped)
            | (AwaitingReview, Running)
            | (AwaitingReview, Completed)
            | (AwaitingReview, Stopped)
            // Resume paths (A0 — intra-stage session continuation per
            // SCOPE.md hard rule #1). A user-resumed job re-enters the
            // queue with its captured `Stage.session_id` intact; the
            // claude adapter passes that id via `CliCfg::resume_id` so
            // the agent picks up the same conversation rather than
            // re-deriving from scratch. `Failed` is included so a
            // cost-cap that fired mid-tool-call (sometimes recorded as
            // failed rather than stopped depending on cancel ordering)
            // is also resumable.
            | (Stopped, Queued)
            | (Failed, Queued)
            // Pause paths (user-initiated `pause_job`, or cap-watcher
            // pausing instead of stopping when the stage has a
            // captured `Stage.session_id`). A paused job is expected
            // to be resumed; it is NOT a terminal state. Resume goes
            // back through `Queued` so the driver re-picks it up via
            // the same loop as a fresh submit.
            | (Running, Paused)
            | (AwaitingReview, Paused)
            | (Paused, Queued)
            | (Paused, Stopped)
            // Reset paths (user-driven `reset_job` recovery hatch).
            // Stuck states (`Queued` the driver could not move,
            // `Failed`, `Stopped`) collapse back to `Draft` so the
            // operator can edit the spec or re-`start_job` without
            // the resume cap dance. `Running`, `Paused`, and
            // `AwaitingReview` are deliberately excluded — those are
            // not stuck-states; they go through `stop_job` /
            // `pause_job` first.
            | (Queued, Draft)
            | (Failed, Draft)
            | (Stopped, Draft),
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

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::JobStatus;

    #[test]
    fn reset_allows_queued_to_draft() {
        transition_job(JobStatus::Queued, JobStatus::Draft).expect("queued -> draft is allowed");
    }

    #[test]
    fn reset_allows_failed_to_draft() {
        transition_job(JobStatus::Failed, JobStatus::Draft).expect("failed -> draft is allowed");
    }

    #[test]
    fn reset_allows_stopped_to_draft() {
        transition_job(JobStatus::Stopped, JobStatus::Draft).expect("stopped -> draft is allowed");
    }

    #[test]
    fn reset_refuses_running_to_draft() {
        let err = transition_job(JobStatus::Running, JobStatus::Draft)
            .expect_err("running -> draft must be refused");
        assert_eq!(err.kind, "job");
    }

    #[test]
    fn reset_refuses_paused_and_awaiting_review_to_draft() {
        assert!(transition_job(JobStatus::Paused, JobStatus::Draft).is_err());
        assert!(transition_job(JobStatus::AwaitingReview, JobStatus::Draft).is_err());
        assert!(transition_job(JobStatus::Completed, JobStatus::Draft).is_err());
    }
}
