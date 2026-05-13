use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codeless_types::{JobId, StageId};
use tokio_util::sync::CancellationToken;

use crate::event_bus::EventBus;

/// Terminal outcome reported by a `Runner::run` call. Internal-only
/// today — the driver maps this to a job state transition plus an
/// outgoing `JobCompleted` / `JobFailed` event. Wire-level event types
/// stay in `codeless-types`; this enum is host-side glue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerOutcome {
    Completed,
    Failed { reason: String },
}

/// What a `Runner` needs from the runtime to publish events. Kept as a
/// concrete struct rather than a trait so the early `Runner`
/// implementations don't have to invent their own indirection — the
/// real `ai-runner` adoption (SCOPE.md "Runner layer") replaces this
/// surface with the upstream crate's `RunnerContext`.
#[derive(Clone)]
pub struct RunnerContext {
    pub job_id: JobId,
    /// The current stage's id when the runner is invoked per-stage
    /// (TemplateRunner path). `None` when no stage frame is in scope —
    /// the legacy single-runner driver path, the in-process test
    /// harnesses, and chat-runners that share `ai-runner` with the
    /// driver. Runners that emit stage-scoped events (today only the
    /// `StageSessionCaptured` capture in `ClaudeRunnerAdapter`) use it
    /// to address the right `Stage` row.
    pub stage_id: Option<StageId>,
    pub bus: Arc<EventBus>,
    /// Provisioned `git worktree` checkout for this run, when the
    /// driver has one to hand. `None` keeps the early test harness
    /// path working without a real repo on disk — the production
    /// drive_job path always populates this from `WorktreeManager`.
    pub worktree_path: Option<PathBuf>,
    /// Driver-owned cancellation token. The cap monitor in `drive_job`
    /// fires it when `job.cost_cap_cents` or `job.wall_clock_cap_ms`
    /// is reached; AI runners must watch it inside their streaming
    /// loop so the upstream HTTP request or child process tears down
    /// promptly. `MockRunner` and other test harnesses are free to
    /// ignore the token — the driver's terminal-state check still
    /// observes a `Stopped` row written by the watcher.
    pub cancel: CancellationToken,
}

/// Host-side runner contract. Asynchronously drives one job to a
/// terminal `RunnerOutcome`, publishing whatever stage/task/AI events
/// it likes through `ctx.bus`. The driver wraps the call to manage the
/// surrounding `Job` row transitions and final `job-completed` /
/// `job-failed` event — runners do **not** emit those themselves.
#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, ctx: RunnerContext) -> RunnerOutcome;
}
