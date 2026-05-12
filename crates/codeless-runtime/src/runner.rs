use std::sync::Arc;

use async_trait::async_trait;
use codeless_types::JobId;

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
    pub bus: Arc<EventBus>,
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
