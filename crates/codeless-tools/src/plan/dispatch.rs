//! Schedule-side glue: a `schedule::Action` that reads `plan_id` out
//! of the payload and calls `PlanEngine::start_run`.
//!
//! Lives in `plan/` rather than `schedule/` because the dependency
//! direction is plan → schedule (the engine doesn't know schedules
//! exist, but a schedule fire wants to kick off a PlanRun). Hosts
//! register this under the dispatcher kind `start_plan_run`; the
//! payload shape is `{"kind":"start_plan_run","plan_id":"<id>"}`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::engine::{JobSpawner, PlanEngine, PlanId, PlanRunId, SpawnError};
use super::spec::StepId;
use crate::schedule::{Action, ScheduleId};
use codeless_types::id::JobId;

pub const START_PLAN_RUN_KIND: &str = "start_plan_run";

/// Boundary-proof spawner: every spawn call emits a tracing event,
/// fabricates a fresh `JobId`, and returns it. Used to bring up the
/// engine in hosts that do not yet have a real job-submission surface
/// wired (P1 of JOB-WORKFLOW). Real spawners — the runtime queue, a
/// future test harness — replace this without touching the engine.
pub struct LogJobSpawner;

#[async_trait]
impl JobSpawner for LogJobSpawner {
    async fn spawn(
        &self,
        plan_run_id: &PlanRunId,
        step_id: &StepId,
        job_template: &str,
    ) -> Result<JobId, SpawnError> {
        let job_id = JobId::new();
        tracing::info!(
            plan_run_id = plan_run_id.as_str(),
            step_id = step_id.as_str(),
            job_template,
            %job_id,
            "LogJobSpawner: would spawn job"
        );
        Ok(job_id)
    }
}

pub struct StartPlanRunAction {
    engine: Arc<PlanEngine>,
}

impl StartPlanRunAction {
    pub fn new(engine: Arc<PlanEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Action for StartPlanRunAction {
    async fn fire(&self, schedule_id: &ScheduleId, payload: &Value) {
        let Some(plan_id) = payload.get("plan_id").and_then(Value::as_str) else {
            tracing::warn!(
                schedule_id = %schedule_id.0,
                "start_plan_run payload missing 'plan_id'"
            );
            return;
        };
        let plan_id = PlanId::new(plan_id);
        match self.engine.start_run(&plan_id).await {
            Ok(run_id) => {
                tracing::info!(
                    schedule_id = %schedule_id.0,
                    plan_id = plan_id.as_str(),
                    run_id = run_id.as_str(),
                    "plan run started from schedule fire"
                );
            }
            Err(err) => {
                tracing::warn!(
                    schedule_id = %schedule_id.0,
                    plan_id = plan_id.as_str(),
                    error = %err,
                    "plan run failed to start from schedule fire"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::engine::{JobSpawner, PlanRunId, SpawnError};
    use crate::plan::spec::{PlanSpec, PlanStep, Transition};

    struct StubSpawner;

    #[async_trait]
    impl JobSpawner for StubSpawner {
        async fn spawn(&self, _: &PlanRunId, _: &StepId, _: &str) -> Result<JobId, SpawnError> {
            Ok(JobId::new())
        }
    }

    fn one_step_spec() -> PlanSpec {
        PlanSpec {
            name: "p".into(),
            steps: vec![PlanStep {
                id: StepId::new("only"),
                job_template: "t".into(),
                on_success: Transition::Stop,
                on_failure: Transition::Stop,
            }],
        }
    }

    #[tokio::test]
    async fn fire_with_plan_id_starts_a_run() {
        let spawner = Arc::new(StubSpawner);
        let engine = Arc::new(PlanEngine::new(spawner));
        engine
            .register_plan(PlanId::new("p"), one_step_spec())
            .await
            .unwrap();

        let action = StartPlanRunAction::new(engine.clone());
        action
            .fire(
                &ScheduleId::new("s"),
                &serde_json::json!({"kind": START_PLAN_RUN_KIND, "plan_id": "p"}),
            )
            .await;

        // Engine should have one run registered now.
        // We can't enumerate runs directly, so probe run-1.
        let state = engine.run_state(&PlanRunId::new("run-1")).await;
        assert!(state.is_some(), "expected a run to be created");
    }

    #[tokio::test]
    async fn fire_without_plan_id_is_a_noop() {
        let spawner = Arc::new(StubSpawner);
        let engine = Arc::new(PlanEngine::new(spawner));
        let action = StartPlanRunAction::new(engine.clone());
        action
            .fire(
                &ScheduleId::new("s"),
                &serde_json::json!({"kind": START_PLAN_RUN_KIND}),
            )
            .await;
        assert!(engine.run_state(&PlanRunId::new("run-1")).await.is_none());
    }
}
