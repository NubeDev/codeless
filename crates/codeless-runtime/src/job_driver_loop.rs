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
//!
//! ## Retry-on-error
//!
//! `drive_job` is fallible — worktree creation, a transient DB
//! failure, or a malformed runner factory call can all return Err
//! before the row ever reaches Running. The naive event-only loop
//! used to log such failures and move on, pinning the row in Queued
//! with no recovery; this is the wedged-Queued failure mode the
//! `runtime-driver-recovery` job is built around. The loop now
//! classifies each `drive_job` error:
//!
//! - **Retryable** (worktree create, IO, transient db / git): the
//!   loop re-publishes `Event::JobQueued` after a bounded backoff
//!   (`RetryPolicy::default()` = 30s / 120s / 600s). After the last
//!   backoff is consumed the row is transitioned to `Failed` with
//!   `stop_reason = RunnerCrash` and `Event::JobFailed` is
//!   published.
//! - **Non-retryable** (runner not enabled at dispatch time,
//!   template parse / argument shape errors surfaced via
//!   `RpcError::Conflict` / `NotFound` / `InvalidArgument`): the
//!   row moves to `Failed` immediately with the same
//!   `stop_reason`.
//!
//! Retry state is per-`JobId` and lives in an in-memory map. SQLite
//! reflects only the outcome — adding a `retry_count` column would
//! over-fit MVP. If the server restarts mid-backoff the counter
//! resets on backlog replay; this is the documented accepted
//! trade-off (see `SCOPE.md` "Constraints" §R4).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use codeless_adapters_host::WorktreeManager;
use codeless_rpc::RpcError;
use codeless_types::{Event, Job, JobId, JobStatus, RepoId, StageId, StageStatus, StopReason};
use futures_util::StreamExt;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::driver::drive_job;
use crate::event_bus::SubscribeFilter;
use crate::rpc::InProcessRpc;
use crate::runner::Runner;
use crate::state_machine::{is_terminal_job, transition_job};
use crate::time::now_ms;

/// Resolves a queued `Job` to a concrete `Runner` implementation.
/// The factory sees the whole row so it can read `job.runner`,
/// `job.prompt`, and (eventually) other per-job knobs without an
/// extra DB round trip. Returning `None` means "this runner isn't
/// enabled on this core"; the loop treats this as a non-retryable
/// failure and transitions the job straight to `Failed`.
///
/// `pending_operator_comment` is the value the driver loop already
/// took-and-cleared from `jobs.pending_operator_comment` for this
/// build. Threading it through the factory keeps the take atomic
/// (one DB statement in the driver, the factory just plumbs the
/// value into runner construction) so a runner rebuild without a
/// fresh resume call sees `None` rather than re-applying stale
/// guidance.
pub trait RunnerFactory: Send + Sync + 'static {
    fn build(&self, job: &Job, pending_operator_comment: Option<String>)
        -> Option<Arc<dyn Runner>>;
}

/// Bounded retry-with-backoff policy for `drive_job` failures.
///
/// `backoff[n]` is the delay applied before the `(n+1)`-th attempt.
/// When `backoff` is exhausted the loop gives up: the job moves to
/// `Failed` with `stop_reason = RunnerCrash` so the user can
/// recover via `reset_job`. Production wiring uses
/// `RetryPolicy::default()` (30s / 120s / 600s); the test harness
/// passes shorter durations so unit tests do not sleep for real.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    backoff: Vec<Duration>,
}

impl RetryPolicy {
    pub fn new(backoff: Vec<Duration>) -> Self {
        Self { backoff }
    }

    /// Backoff applied to retryable failures, in order. The slice
    /// length doubles as the retry budget — `len()` retries before
    /// the job is transitioned to `Failed`.
    pub fn backoff(&self) -> &[Duration] {
        &self.backoff
    }

    /// Sub-second backoff for unit tests. Pinned in code so the
    /// production default and the test harness cannot drift apart
    /// silently.
    pub fn test_fast() -> Self {
        Self {
            backoff: vec![
                Duration::from_millis(5),
                Duration::from_millis(5),
                Duration::from_millis(5),
            ],
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            backoff: vec![
                Duration::from_secs(30),
                Duration::from_secs(120),
                Duration::from_secs(600),
            ],
        }
    }
}

/// Shared retry-counter map. Keyed by `JobId`; the value is the
/// number of `drive_job` attempts that have already failed. Cleared
/// on success and on terminal give-up.
type RetryAttempts = Arc<Mutex<HashMap<JobId, usize>>>;

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

/// Spawn the driver with the default retry policy. Thin wrapper
/// over `spawn_job_driver_loop_with_retry` so existing call sites
/// stay unchanged.
pub async fn spawn_job_driver_loop<F: RunnerFactory>(
    rpc: Arc<InProcessRpc>,
    factory: Arc<F>,
    worktrees: Option<Arc<WorktreeManager>>,
    concurrency: usize,
) -> Result<DriverLoopHandle, RpcError> {
    spawn_job_driver_loop_with_retry(rpc, factory, worktrees, concurrency, RetryPolicy::default())
        .await
}

/// Spawn the driver loop with an explicit `RetryPolicy`. Tests
/// pass `RetryPolicy::test_fast()` so the backoff path is
/// exercised without parking the runtime for ten minutes.
pub async fn spawn_job_driver_loop_with_retry<F: RunnerFactory>(
    rpc: Arc<InProcessRpc>,
    factory: Arc<F>,
    worktrees: Option<Arc<WorktreeManager>>,
    concurrency: usize,
    retry: RetryPolicy,
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
    let retries: RetryAttempts = Arc::new(Mutex::new(HashMap::new()));

    let join = tokio::spawn(
        async move {
            // Drive whatever is already `Queued` on disk. The runtime's
            // startup lease reaper has already converted abandoned
            // `Running` rows back to `Queued`, so a single pass here
            // covers crashes.
            replay_backlog(&rpc, &factory, &worktrees, &semaphore, &retries, &retry).await;

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
                        let job_id = match env.event {
                            Event::JobQueued { job_id, .. }
                            | Event::JobPromoted { job_id }
                            | Event::JobResumed { job_id, .. } => Some(job_id),
                            _ => None,
                        };
                        if let Some(job_id) = job_id {
                            dispatch(
                                rpc.clone(),
                                factory.clone(),
                                worktrees.clone(),
                                semaphore.clone(),
                                retries.clone(),
                                retry.clone(),
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
    retries: &RetryAttempts,
    retry: &RetryPolicy,
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
            retries.clone(),
            retry.clone(),
            job.id,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch<F: RunnerFactory>(
    rpc: Arc<InProcessRpc>,
    factory: Arc<F>,
    worktrees: Option<Arc<WorktreeManager>>,
    semaphore: Arc<Semaphore>,
    retries: RetryAttempts,
    retry: RetryPolicy,
    job_id: JobId,
) {
    let mut job = match rpc.store().get_job(job_id).await {
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
    let repo_id = job.repo_id;

    // Prepend the prior session's handover and the user-authored job
    // docs (SCOPE.md / WORKFLOW.md / extras) to the prompt the runner
    // sees, so the next session inherits the inter-session contract
    // (JOB-MODEL.md "the handover is the only contract between
    // sessions") and the job-level intent (JOB-DIR.md "How the agent
    // reads the docs"). The augmented prompt only flows into the
    // factory local-variable; the job row in SQLite still carries the
    // original prompt the user submitted, so this prefixing stays
    // invisible at the wire level.
    //
    // Order (per JOB-DIR.md): handover → job docs → original. Notes
    // sit in `runs/<job_id>/notes/` and reach the prompt through the
    // existing per-run handover, not through this loop.
    if let Ok(Some(repo)) = rpc.store().get_repo(job.repo_id).await {
        let repo_path = std::path::PathBuf::from(&repo.local_path);
        // Keyed handover discovery (JOB-MODEL.md H3): pick the prior
        // handover by `(job_id, stage_id)`, not by mtime. The mtime
        // ranking that previously lived here straddled unrelated jobs
        // — it would prefix the next job's prompt with whatever
        // worktree happened to be newest on disk. The correct prior
        // is the most-recently-terminated stage of *this* job; if no
        // such stage exists (fresh JobQueued, no resume context),
        // no prefix is added.
        let mut handover_prefix = String::new();
        if let Some(stage_id) = latest_terminal_stage(&rpc, job_id).await {
            if let Some((path, prior)) =
                crate::handover::find_handover(&repo_path, job_id, stage_id).await
            {
                handover_prefix = crate::handover::prompt_prefix_for(&path, &prior);
                tracing::info!(handover = %path.display(), "prepended prior handover to prompt");
            }
        }

        let job_docs = job
            .template_yaml
            .as_deref()
            .and_then(|yaml| crate::template::JobTemplate::parse_yaml(yaml).ok())
            .map(|tpl| match tpl.docs.as_deref() {
                Some(list) => crate::job_dir::read_docs_ordered(&repo_path, &tpl.name, list),
                None => crate::job_dir::read_docs_for_prompt(&repo_path, &tpl.name),
            })
            .filter(|s| !s.is_empty())
            .map(|body| format!("{body}\n"))
            .unwrap_or_default();

        if !handover_prefix.is_empty() || !job_docs.is_empty() {
            let original = job.prompt.clone().unwrap_or_default();
            job.prompt = Some(format!("{handover_prefix}{job_docs}{original}"));
        }
    }

    // Take-and-clear the operator's pending comment for this job.
    // The slot is consumed exactly once per runner build: a
    // subsequent driver restart or second resume without a fresh
    // `next_stage_comment` sees `None` rather than re-threading
    // stale guidance into the wrong stage. A DB error here is not
    // fatal to the job — the comment is best-effort context, not
    // correctness — but is logged so a regression is visible.
    let pending_operator_comment = match rpc.store.take_pending_operator_comment(job.id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                %job_id,
                error = %e,
                "driver: failed to take pending_operator_comment; runner builds without it",
            );
            None
        }
    };

    let runner = match factory.build(&job, pending_operator_comment) {
        Some(r) => r,
        None => {
            // Runner not enabled on this core. Non-retryable —
            // re-publishing JobQueued would loop forever against
            // the same missing adapter. Move the row straight to
            // Failed so the user sees the failure and can either
            // re-submit with a different runner or restart the
            // server with the runner wired in.
            tracing::warn!(
                %job_id,
                runner = %job.runner,
                kind = "runner-not-enabled",
                "driver: runner not enabled; failing job",
            );
            retries.lock().unwrap().remove(&job_id);
            mark_job_failed(&rpc, job_id).await;
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
    let rpc_for_task = rpc.clone();
    tokio::spawn(async move {
        let _permit = permit;
        match drive_job(&rpc_for_task, job_id, runner, worktrees).await {
            Ok(()) => {
                // Clear the retry counter on success so a future
                // resubmit of the same JobId (rerun_job / resume)
                // starts with a fresh budget.
                retries.lock().unwrap().remove(&job_id);
            }
            Err(err) => {
                handle_drive_error(&rpc_for_task, job_id, repo_id, &err, &retries, &retry).await;
            }
        }
    });
}

/// Find the `StageId` whose handover should prefix the next prompt
/// for `job_id`. Selection rule: highest-ordinal stage that has
/// already reached a terminal status (Passed, Failed, or
/// AwaitingReview — the last because a paused-at-review stage's
/// handover is the contract the next session resumes against).
/// Returns `None` when the job has no such stage yet — the prompt-
/// prefix path then runs without a prior-handover preamble, which is
/// correct for a fresh job and for the very first stage of a resumed
/// one.
async fn latest_terminal_stage(rpc: &Arc<InProcessRpc>, job_id: JobId) -> Option<StageId> {
    let stages = rpc.store().list_stages_for_job(job_id).await.ok()?;
    stages
        .into_iter()
        .filter(|s| {
            matches!(
                s.stage.status,
                StageStatus::Passed | StageStatus::Failed | StageStatus::AwaitingReview
            )
        })
        .map(|s| (s.stage.ordinal, s.stage.id))
        .max_by_key(|(ordinal, _)| *ordinal)
        .map(|(_, id)| id)
}

/// Distinguishes retryable transient errors from terminal failures.
/// The classifier reads error *shape* only — no process spawn, no
/// IO — so it stays clean against R1 (process-spawn confinement) and
/// can run in a hot path. Anything the runtime hasn't proved is
/// retryable is treated as terminal so a wedged row is the loud
/// failure mode rather than an infinite-retry loop.
fn is_retryable(err: &RpcError) -> bool {
    match err {
        // The only error kind drive_job manufactures from a
        // worktree / git / sqlx failure is RpcError::Internal with
        // a `<kind>:` prefix the driver itself stamps in. Match on
        // the prefix so the classifier is grep-able from the
        // source of the failure.
        RpcError::Internal(msg) => {
            msg.starts_with("worktree ")
                || msg.starts_with("db: ")
                || msg.starts_with("git ")
                || msg.starts_with("io: ")
        }
        // Conflict means the row moved out from under us (already
        // Running, already terminal). Retrying re-races the same
        // wall, so this is non-retryable by design.
        RpcError::Conflict(_) => false,
        // NotFound / InvalidArgument / Workspace are config or
        // shape errors. Retry is futile.
        RpcError::NotFound(_) | RpcError::InvalidArgument(_) | RpcError::Workspace(_) => false,
    }
}

fn error_kind_label(err: &RpcError) -> &'static str {
    match err {
        RpcError::Internal(msg) if msg.starts_with("worktree ") => "worktree",
        RpcError::Internal(msg) if msg.starts_with("db: ") => "db",
        RpcError::Internal(msg) if msg.starts_with("git ") => "git",
        RpcError::Internal(msg) if msg.starts_with("io: ") => "io",
        RpcError::Internal(_) => "internal",
        RpcError::Conflict(_) => "conflict",
        RpcError::NotFound(_) => "not-found",
        RpcError::InvalidArgument(_) => "invalid-argument",
        RpcError::Workspace(_) => "workspace",
    }
}

async fn handle_drive_error(
    rpc: &Arc<InProcessRpc>,
    job_id: JobId,
    repo_id: RepoId,
    err: &RpcError,
    retries: &RetryAttempts,
    retry: &RetryPolicy,
) {
    let kind = error_kind_label(err);
    if !is_retryable(err) {
        tracing::warn!(%job_id, %kind, error = %err, "drive_job non-retryable; failing job");
        retries.lock().unwrap().remove(&job_id);
        mark_job_failed(rpc, job_id).await;
        return;
    }

    // Take the current attempt index and bump the counter under
    // one short-held lock. Past-the-budget attempts give up;
    // otherwise the delay is `backoff[attempt]`.
    let (attempt, delay) = {
        let mut map = retries.lock().unwrap();
        let n = map.entry(job_id).or_insert(0);
        let attempt = *n;
        if attempt >= retry.backoff().len() {
            map.remove(&job_id);
            (attempt, None)
        } else {
            let d = retry.backoff()[attempt];
            *n = attempt + 1;
            (attempt, Some(d))
        }
    };

    let Some(delay) = delay else {
        tracing::warn!(
            %job_id,
            %kind,
            attempt,
            error = %err,
            "drive_job retry budget exhausted; failing job",
        );
        mark_job_failed(rpc, job_id).await;
        return;
    };

    tracing::warn!(
        %job_id,
        %kind,
        attempt,
        delay_ms = delay.as_millis() as u64,
        error = %err,
        "drive_job retryable; scheduling re-publish",
    );

    let rpc_for_retry = rpc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        // The job may have been reset / deleted / completed during
        // the backoff. Re-publishing JobQueued is harmless — the
        // dispatch read of the row will skip non-Queued status.
        let now = now_ms();
        if let Err(e) = rpc_for_retry
            .bus()
            .publish(
                Some(job_id),
                None,
                None,
                Event::JobQueued { job_id, repo_id },
                now,
            )
            .await
        {
            tracing::warn!(%job_id, error = %e, "retry re-publish JobQueued failed");
        }
    });
}

/// Move a non-terminal job row to `Failed` and publish
/// `JobFailed`. Used by both the non-retryable error path and the
/// retry-budget-exhausted path. `stop_reason = RunnerCrash`
/// distinguishes a driver give-up from a clean user-driven
/// terminal state.
async fn mark_job_failed(rpc: &Arc<InProcessRpc>, job_id: JobId) {
    let store = rpc.store();
    let bus = rpc.bus();
    let mut job = match store.get_job(job_id).await {
        Ok(Some(j)) => j,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(%job_id, error = %e, "mark_job_failed: get_job");
            return;
        }
    };
    if is_terminal_job(job.status) || job.status == JobStatus::Paused {
        return;
    }
    if let Err(e) = transition_job(job.status, JobStatus::Failed) {
        tracing::warn!(%job_id, from = ?job.status, error = %e, "mark_job_failed: refused");
        return;
    }
    let ended = now_ms();
    job.status = JobStatus::Failed;
    // Preserve any wire-level stop reason the runner already stamped
    // on the row (e.g. `ReviewPreCheck` from the diff-verify gate).
    // `RunnerCrash` is the fallback for the genuine "the runner
    // panicked / exited without setting a reason" case; overwriting
    // a more specific reason here would erase the signal the UI uses
    // to surface the right recovery affordance.
    if job.stop_reason.is_none() {
        job.stop_reason = Some(StopReason::RunnerCrash);
    }
    job.ended_at = Some(ended);
    if let Err(e) = store.update_job(&job).await {
        tracing::warn!(%job_id, error = %e, "mark_job_failed: update_job");
        return;
    }
    if let Err(e) = bus
        .publish(Some(job_id), None, None, Event::JobFailed { job_id }, ended)
        .await
    {
        tracing::warn!(%job_id, error = %e, "mark_job_failed: publish");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_retryable_errors() {
        assert!(is_retryable(&RpcError::Internal(
            "worktree create: boom".into()
        )));
        assert!(is_retryable(&RpcError::Internal("db: deadlock".into())));
        assert!(is_retryable(&RpcError::Internal(
            "git checkout -B: lock".into()
        )));
        assert!(is_retryable(&RpcError::Internal("io: stale handle".into())));
    }

    #[test]
    fn classify_non_retryable_errors() {
        assert!(!is_retryable(&RpcError::Conflict("already running".into())));
        assert!(!is_retryable(&RpcError::NotFound("job".into())));
        assert!(!is_retryable(&RpcError::InvalidArgument("shape".into())));
        // Generic Internal that doesn't match any known retryable
        // prefix is treated as terminal: better to fail loudly than
        // loop forever against an unknown class.
        assert!(!is_retryable(&RpcError::Internal(
            "template parse: unexpected key".into()
        )));
    }

    #[test]
    fn test_fast_policy_has_three_attempts() {
        // The retry-budget contract is "three retries before
        // Failed." Pin it so a future tweak to the default policy
        // doesn't silently change the test backoff.
        assert_eq!(RetryPolicy::test_fast().backoff().len(), 3);
        assert_eq!(RetryPolicy::default().backoff().len(), 3);
    }
}
