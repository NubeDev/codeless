//! `restart_server` RPC: enumerate running jobs, partition them into
//! resumable / killed, and (when allowed) fire the runtime's shutdown
//! signal so the host context can either re-exec the child (supervised
//! CLI), respawn the sidecar (Tauri desktop), or refuse and surface a
//! manual command (bare CLI).
//!
//! The partition rule is fixed at stage 1 of the adapter-registry job
//! (`.codeless/jobs/adapter-registry/SCOPE.md` §"Open questions" Q4):
//! a running job is *resumable* iff its runner advertises template-
//! driven replay AND its most recent persisted stage transition is
//! within the last 30s; everything else is *killed*. PTY-bound runners
//! (`claude`, `codex`, `copilot`) are never reported as resumable —
//! their child process dies on restart and the in-flight stream is
//! gone regardless of how recent the last transition was.

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AdapterError, RestartServerArgs, RestartServerResult, RpcError, RpcResult};
use codeless_types::{JobId, JobStatus};
use tokio::sync::Notify;

use super::InProcessRpc;
use crate::time::now_ms;

/// Where this server is running, which decides what `restart_server`
/// does once the partition check passes. The CLI / Tauri shell sets
/// this at boot via `InProcessRpc::with_restart_context`; tests default
/// to `Bare` so a stray `restart_server` call in a unit test cannot
/// accidentally fire the shutdown signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartContext {
    /// Running under a supervisor that re-execs on exit-75 (systemd,
    /// `init-session.sh`, or the `--respawn-on-exit` self-watcher).
    /// The RPC fires the shutdown signal so the process exits with
    /// `EX_TEMPFAIL`; the supervisor re-execs.
    SupervisedCli,
    /// Running as a Tauri sidecar. The desktop shell owns sidecar
    /// lifecycle, so the RPC fires the shutdown signal and the shell
    /// respawns the child. Process exit code is `0` — the shell does
    /// not interpret 75.
    TauriDesktop,
    /// Bare `codeless serve` with no supervisor. The RPC refuses with
    /// `AdapterError::RestartUnsupervised` and a copy-pasteable hint
    /// rather than exiting, because there is nothing to bring the
    /// process back.
    Bare,
}

/// Sentinel exit code the supervisor reads to mean "I asked for a
/// restart". Matches `<sysexits.h>` `EX_TEMPFAIL`; documented in
/// `.codeless/jobs/adapter-registry/SCOPE.md` and reused by the
/// `--respawn-on-exit` self-watcher in `codeless-adapters-host::respawn`.
pub const EX_TEMPFAIL: i32 = 75;

/// Resumable window — see the stage-1 decision in
/// `DOCS/SCOPE.md` §"Adapter registry, stage 1". A running job whose
/// most recent stage transition is older than this is reported
/// `killed` even if its runner is template-driven.
pub const RESUMABLE_WINDOW: Duration = Duration::from_secs(30);

/// Cross-task handle for the shutdown signal a successful
/// `restart_server` arms. The CLI's `run_server` selects on this Notify
/// alongside SIGINT so a successful call drops the axum listener
/// gracefully. The CLI captures the requested exit code from
/// [`RestartTrigger::desired_exit_code`] after the signal fires.
#[derive(Clone)]
pub struct RestartTrigger {
    notify: Arc<Notify>,
    state: Arc<parking_lot::Mutex<TriggerState>>,
}

#[derive(Default)]
struct TriggerState {
    fired: bool,
    /// Exit code the CLI should use after the listener drains. `None`
    /// before the trigger has fired.
    exit_code: Option<i32>,
}

impl RestartTrigger {
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            state: Arc::new(parking_lot::Mutex::new(TriggerState::default())),
        }
    }

    /// Await the next restart signal. Returns immediately when the
    /// trigger has already fired so a caller racing the RPC against a
    /// previously-fired signal still observes it.
    pub async fn wait(&self) {
        if self.state.lock().fired {
            return;
        }
        self.notify.notified().await;
    }

    /// Exit code the CLI should pass to `std::process::exit` after the
    /// listener drains. `None` when the trigger has not fired yet.
    pub fn desired_exit_code(&self) -> Option<i32> {
        self.state.lock().exit_code
    }

    fn fire(&self, exit_code: i32) {
        let mut st = self.state.lock();
        if st.fired {
            return;
        }
        st.fired = true;
        st.exit_code = Some(exit_code);
        drop(st);
        self.notify.notify_waiters();
    }
}

impl Default for RestartTrigger {
    fn default() -> Self {
        Self::new()
    }
}

/// Template-driven runners that survive a restart by replaying from
/// their last persisted stage. The PTY-bound CLI runners (`claude`,
/// `codex`, `copilot`) are deliberately excluded — their child process
/// dies on restart and any in-flight stream is gone. New runner crates
/// register the same way they do elsewhere (the runner-id is free-form
/// on the wire); adding one to this list is a one-line change paired
/// with the corresponding factory wiring.
pub(crate) fn is_resumable_runner(runner_id: &str) -> bool {
    matches!(runner_id, "mock" | "anthropic")
}

/// Whether the stage row's most recent transition timestamp is within
/// the [`RESUMABLE_WINDOW`]. Uses `ended_at` when present (the stage
/// completed cleanly) else `started_at` (the stage is mid-run). A
/// running job with no stage rows at all is treated as having a stale
/// checkpoint — replay would have no anchor to resume from.
fn has_recent_checkpoint(stage_ts_ms: Option<i64>, now_ms_v: i64) -> bool {
    let Some(ts) = stage_ts_ms else {
        return false;
    };
    let age_ms = now_ms_v.saturating_sub(ts);
    age_ms >= 0 && age_ms <= RESUMABLE_WINDOW.as_millis() as i64
}

pub(super) async fn restart_server(
    rpc: &InProcessRpc,
    args: RestartServerArgs,
) -> RpcResult<RestartServerResult> {
    // Partition first, force second. The order matters: even
    // `force = true` callers benefit from the audit log of which jobs
    // they killed, and the partition is cheap.
    let (resumable, killed) = partition_running_jobs(rpc).await?;

    if !args.force && (!resumable.is_empty() || !killed.is_empty()) {
        return Err(RpcError::Adapter(AdapterError::RestartHasRunningJobs {
            resumable,
            killed,
        }));
    }

    match rpc.restart_context {
        RestartContext::Bare => Err(RpcError::Adapter(AdapterError::RestartUnsupervised {
            hint: "this codeless serve has no supervisor; restart manually \
                   (Ctrl-C, then re-run `codeless serve`) or relaunch with \
                   `codeless serve --respawn-on-exit` so future restarts re-exec automatically"
                .to_owned(),
        })),
        RestartContext::SupervisedCli => {
            rpc.restart_trigger.fire(EX_TEMPFAIL);
            Ok(RestartServerResult {})
        }
        RestartContext::TauriDesktop => {
            // The desktop shell observes the sidecar exit and respawns
            // it; exit code is `0` because the shell does not interpret
            // the `EX_TEMPFAIL` sentinel.
            rpc.restart_trigger.fire(0);
            Ok(RestartServerResult {})
        }
    }
}

/// Walk every `Running` job row and bucket each id into `resumable` /
/// `killed`. The store is the source of truth (R4); no in-memory view
/// is consulted. The recency check uses the latest `started_at` /
/// `ended_at` across the job's stage rows so a paused template-driven
/// run whose last stage transitioned ~5s ago counts as resumable even
/// if no stage is currently `Running`.
pub(crate) async fn partition_running_jobs(
    rpc: &InProcessRpc,
) -> RpcResult<(Vec<JobId>, Vec<JobId>)> {
    let jobs = rpc.store.list_jobs(None).await.map_err(super::db_err)?;
    let now = now_ms();
    let mut resumable = Vec::new();
    let mut killed = Vec::new();
    for job in jobs {
        if job.status != JobStatus::Running {
            continue;
        }
        // PTY-bound runners drop their child process; the in-flight
        // stream cannot be replayed. Short-circuit before the stage
        // query so a long list of PTY jobs does not stress sqlx.
        if !is_resumable_runner(&job.runner) {
            killed.push(job.id);
            continue;
        }
        let stages = rpc
            .store
            .list_stages_for_job(job.id)
            .await
            .map_err(super::db_err)?;
        let latest_ts_ms = stages
            .iter()
            .flat_map(|sw| {
                [
                    sw.stage.ended_at.map(|t| t.0),
                    sw.stage.started_at.map(|t| t.0),
                ]
            })
            .flatten()
            .max();
        if has_recent_checkpoint(latest_ts_ms, now.0) {
            resumable.push(job.id);
        } else {
            killed.push(job.id);
        }
    }
    Ok((resumable, killed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_runners_never_resumable() {
        assert!(!is_resumable_runner("claude"));
        assert!(!is_resumable_runner("codex"));
        assert!(!is_resumable_runner("copilot"));
    }

    #[test]
    fn template_runners_resumable() {
        assert!(is_resumable_runner("mock"));
        assert!(is_resumable_runner("anthropic"));
    }

    #[test]
    fn checkpoint_window_inclusive_at_boundary() {
        let now = 1_000_000i64;
        let inside = now - (RESUMABLE_WINDOW.as_millis() as i64 - 1);
        let edge = now - RESUMABLE_WINDOW.as_millis() as i64;
        let outside = now - (RESUMABLE_WINDOW.as_millis() as i64 + 1);
        assert!(has_recent_checkpoint(Some(inside), now));
        assert!(has_recent_checkpoint(Some(edge), now));
        assert!(!has_recent_checkpoint(Some(outside), now));
        assert!(!has_recent_checkpoint(None, now));
    }

    #[tokio::test]
    async fn trigger_records_exit_code_after_fire() {
        let trig = RestartTrigger::new();
        assert!(trig.desired_exit_code().is_none());
        let trig2 = trig.clone();
        let join = tokio::spawn(async move {
            trig2.wait().await;
        });
        trig.fire(EX_TEMPFAIL);
        join.await.unwrap();
        assert_eq!(trig.desired_exit_code(), Some(EX_TEMPFAIL));
        // Re-firing is a no-op so a second `restart_server` call
        // doesn't clobber the first context's exit code.
        trig.fire(0);
        assert_eq!(trig.desired_exit_code(), Some(EX_TEMPFAIL));
    }
}
