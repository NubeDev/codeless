use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codeless_adapters_host::WorktreeManager;
use codeless_rpc::{RpcError, RpcResult};
use codeless_types::{Event, JobId, JobStatus, StopReason, WorkspaceMode};
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

    let provisioned = match job.workspace_mode {
        WorkspaceMode::InRepo => {
            // In-repo mode: edits land in the user's existing clone.
            // Create the branch but skip `git worktree add`.
            let repo = store
                .get_repo(job.repo_id)
                .await
                .map_err(db_err)?
                .ok_or_else(|| RpcError::NotFound(format!("repo {}", job.repo_id)))?;
            let repo_path = PathBuf::from(&repo.local_path);
            provision_in_repo(&repo_path, store, &mut job).await?;
            None
        }
        WorkspaceMode::Worktree => match worktrees.as_ref() {
            Some(mgr) => Some(provision_worktree(mgr, store, &mut job).await?),
            None => None,
        },
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

    // Per-Run supervisor (JOB-CHAT.md (C2)). Spawned here — after the
    // row reached `Running` and the `JobStarted` envelope is in the
    // persisted events table — so the supervisor's own
    // `subscribe_since` replay sees the start event for the Run it
    // owns. The handle is intentionally not joined: the supervisor
    // self-terminates when it observes a Run terminal event
    // (`JobCompleted` / `JobFailed` / `JobStopped`) on the bus,
    // which the rest of `drive_job` is going to publish below. A
    // fresh Run (rerun / resume → new `drive_job` invocation) spawns
    // a fresh supervisor; the previous Run's task has already exited
    // by the time we get here because its terminal event already
    // fired.
    let _supervisor = crate::supervisor::spawn_supervisor(Arc::clone(bus), job_id);

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
            stage_id: None,
            bus: Arc::clone(bus),
            worktree_path: match job.workspace_mode {
                WorkspaceMode::InRepo => {
                    // In in_repo mode the repo's local_path *is* the
                    // working directory. We already stored it on the job
                    // row during provision_in_repo.
                    job.worktree_path.as_ref().map(PathBuf::from)
                }
                WorkspaceMode::Worktree => provisioned.as_ref().map(|p| p.worktree.clone()),
            },
            cancel: cancel.clone(),
        })
        .await;
    cap_watcher.abort();

    let Some(current) = store.get_job(job_id).await.map_err(db_err)? else {
        return Err(RpcError::NotFound(format!("job {job_id}")));
    };
    if is_terminal_job(current.status) || current.status == JobStatus::Paused {
        tracing::info!(status = ?current.status, "runner returned after stop or pause");
        // Worktree is intentionally left on disk. SCOPE.md "Crash
        // recovery": "Worktrees: a job whose task crashed leaves its
        // worktree on disk. The reaper either preserves it (default —
        // user can inspect / re-run from where it was) or removes it
        // (configurable). It does not silently delete user-visible
        // work." The user-driven cleanup path (a future
        // `gc_worktrees` RPC + UI button) reaps; the driver never
        // does. `worktree_path` on the job row still points at it.
        //
        // `Paused` lands here when the cap-watcher paused the job
        // mid-stage (resumable via `resume_job`); the row + branch +
        // captured `Stage.session_id` all survive for the resume
        // path.
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
    // SCOPE-ASSISTANT-PARITY W3d: a non-cap stage failure under `None`
    // policy surfaces a `set_policy` recommendation in the most-recent
    // assistant thread. The helper is best-effort and silently no-ops
    // when there is nothing to render against, so the terminal-state
    // path here does not branch on whether a card was actually written.
    if next_status == JobStatus::Failed {
        crate::auto_bypass_failure_card::maybe_emit_failure_set_policy_card(rpc, job_id).await;
    }
    // Same preservation rule as the early-terminal branch above:
    // the worktree stays on disk so the user can inspect or re-run
    // from where it left off. `release_worktree` is kept around for
    // an upcoming user-driven `gc_worktrees` RPC, not auto-fired.
    //
    // The handover is now per-stage (`runs/<job_id>/<stage_id>/...`,
    // JOB-MODEL.md H1), so the driver no longer synthesises a
    // job-level fallback here — there is no stage frame at this
    // call site. Per-stage runners (TemplateRunner) drop their own
    // handover inside `runner.run`; runners that bypass the stage
    // frame leave no handover, which is the correct outcome for the
    // keyed-discovery contract (H3).
    if let Some(worktree) = provisioned.as_ref().map(|p| p.worktree.clone()) {
        // Append the session block to runs/<job_id>/log.md. Always
        // runs (the handover is overwritten each session; the log
        // never is — JOB-MODEL.md "one block per session, never
        // rewritten"). The block carries the three load-bearing
        // fields the doc spells out: what got done, how much it cost,
        // why the run ended. Today's per-ULID layout matches the
        // handover; a future migration to `<repo>/runs/<name>/` moves
        // both files together.
        let end = crate::session_log::EndReason::from_status(next_status);
        match crate::session_log::append_session_block(&worktree, &updated, end).await {
            Ok(path) => tracing::info!(log = %path.display(), "session log appended"),
            Err(err) => tracing::warn!(?err, "failed to append session log; ignoring"),
        }
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
        .create(&repo_path, &job.id.to_string(), Some(&job.branch))
        .map_err(|e| RpcError::Internal(format!("worktree create: {e}")))?;
    job.worktree_path = Some(handle.path.to_string_lossy().into_owned());
    job.branch = handle.branch.clone();
    store.update_job(job).await.map_err(db_err)?;
    Ok(ProvisionedWorktree {
        manager: Arc::clone(manager),
        repo_path,
        worktree: handle.path,
    })
}

/// In-repo mode: create a branch in the user's local clone and record
/// the repo path as the working directory. No `git worktree add`.
async fn provision_in_repo(
    repo_path: &PathBuf,
    store: &Arc<SqliteStore>,
    job: &mut codeless_types::Job,
) -> RpcResult<()> {
    // Only attempt `git checkout -B` when the repo path is a real git
    // checkout. Test repos and placeholder paths skip the branch
    // creation — the mock runner doesn't need a working git tree.
    if repo_path.join(".git").exists() {
        // An empty branch (set by rerun_job) gets a canonical name so
        // `git checkout -B ""` never runs.
        if job.branch.is_empty() {
            job.branch = format!("codeless/job-{}", job.id);
        }
        let branch = &job.branch;
        let output = std::process::Command::new("git")
            .args(["checkout", "-B", branch])
            .current_dir(repo_path)
            .output()
            .map_err(|e| RpcError::Internal(format!("git checkout -B: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RpcError::Internal(format!(
                "git checkout -B {branch}: {stderr}"
            )));
        }
    } else {
        tracing::warn!(
            repo = %repo_path.display(),
            "in_repo mode: path is not a git repo, skipping branch creation",
        );
    }
    job.worktree_path = Some(repo_path.to_string_lossy().into_owned());
    store.update_job(job).await.map_err(db_err)?;
    Ok(())
}

// Kept for the upcoming user-driven `gc_worktrees` RPC. The driver no
// longer auto-reaps on terminal status (SCOPE.md "Crash recovery"
// makes preservation the default); cleanup will land as an explicit
// user action.
#[allow(dead_code)]
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

    // The select! body distinguishes two outcomes per loop turn:
    //
    //   - WatcherAction::FireCap(reason): a cap WE detected. The
    //     watcher owns writing the row + publishing the event +
    //     firing cancel via `fire_pause_or_stop`. This is the cost
    //     / wall-clock path.
    //
    //   - WatcherAction::ExternalTerminal: an external path (the
    //     `stop_job` / `pause_job` RPC) already wrote a terminal /
    //     paused row and published its event. The watcher just
    //     fires cancel so the in-flight runner exits, then returns.
    //     This is the load-bearing fix that makes mid-stage stop /
    //     pause actually interrupt the runner; without it `stop_job`
    //     was advisory until the next cap or natural completion.
    enum WatcherAction {
        FireCap(StopReason),
        ExternalTerminal,
    }
    loop {
        let next_item: futures_core::future::BoxFuture<'_, _> = Box::pin(stream.next());
        let action = tokio::select! {
            biased;
            _ = async {
                match wall_clock_sleep.as_mut().as_pin_mut() {
                    Some(s) => s.await,
                    None => std::future::pending::<()>().await,
                }
            } => Some(WatcherAction::FireCap(StopReason::WallClock)),
            item = next_item => {
                match item {
                    Some(Ok(env)) => match &env.event {
                        Event::AiMessageComplete { .. } if cost_cap > 0 => {
                            match store.get_job(job_id).await {
                                Ok(Some(j)) if j.cost_cents.0 >= cost_cap => {
                                    Some(WatcherAction::FireCap(StopReason::CostCap))
                                }
                                _ => None,
                            }
                        }
                        Event::JobStopped { .. }
                        | Event::JobPaused { .. }
                        | Event::JobFailed { .. } => {
                            Some(WatcherAction::ExternalTerminal)
                        }
                        _ => None,
                    },
                    Some(Err(_)) => None,
                    None => return,
                }
            }
        };
        match action {
            Some(WatcherAction::FireCap(reason)) => {
                fire_pause_or_stop(&store, &bus, job_id, reason, &cancel).await;
                return;
            }
            Some(WatcherAction::ExternalTerminal) => {
                cancel.cancel();
                return;
            }
            None => {}
        }
    }
}

/// When a cap trips mid-stage, decide whether the job is resumable
/// (any stage on this job has a captured `Stage.session_id` — the
/// claude wrapper can `--continue` from it) or terminal (no session
/// captured anywhere, so a fresh session would be the only path
/// forward, which is what `Stopped` semantics already mean).
///
/// Resumable -> `Paused` + `JobPaused` event. The row stays
/// non-terminal; `resume_job` accepts it. The user's recovery is
/// "raise the cap and click resume."
///
/// Non-resumable -> today's behaviour: `Stopped` + `JobStopped`.
/// The user's recovery is "re-run from scratch."
///
/// `is_terminal_job` and the cancellation token semantics are
/// unchanged for both paths — the runner sees a cancellation and
/// exits regardless.
async fn fire_pause_or_stop(
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
    if is_terminal_job(job.status) || job.status == JobStatus::Paused {
        cancel.cancel();
        return;
    }
    let resumable = has_captured_session(store, job_id).await;
    let ended = now_ms();
    job.stop_reason = Some(reason);
    job.ended_at = Some(ended);
    if resumable {
        job.status = JobStatus::Paused;
    } else {
        job.status = JobStatus::Stopped;
    }
    if let Err(e) = store.update_job(&job).await {
        tracing::warn!(error = %e, "cap watcher: update_job failed");
    }
    let event = if resumable {
        Event::JobPaused { job_id, reason }
    } else {
        Event::JobStopped { job_id, reason }
    };
    if let Err(e) = bus.publish(Some(job_id), None, None, event, ended).await {
        tracing::warn!(
            error = %e,
            resumable,
            "cap watcher: publish JobPaused/JobStopped failed"
        );
    }
    cancel.cancel();
}

/// True when any stage on this job has captured a runner session
/// id — `Stage.session_id IS NOT NULL`. The list query is cheap
/// (a single SELECT scoped to one job's stages); avoiding a
/// targeted `WHERE session_id IS NOT NULL` keeps the SqliteStore
/// surface area smaller without hurting the cap-watcher's hot path
/// (the watcher only fires once per cap trip).
async fn has_captured_session(store: &Arc<SqliteStore>, job_id: JobId) -> bool {
    match store.list_stages_for_job(job_id).await {
        Ok(stages) => stages.iter().any(|s| s.stage.session_id.is_some()),
        Err(e) => {
            tracing::warn!(error = %e, "cap watcher: list_stages_for_job failed; falling back to stop");
            false
        }
    }
}
