// LLM tool surface for the in-memory Plan engine (P1). Four small
// `Tool` impls share an `Arc<PlanEngine>` so a single engine instance
// holds all registered plans and in-flight runs across calls.
//
// Mirrors the shape of `schedule_create.rs` (host injects the engine,
// the tool only mutates the registry), but the schedule tool packs
// three actions behind one MCP name; here each operation gets its own
// name (`codeless.plan.create`, `.start`, `.list`, `.cancel`) because
// the JOB-WORKFLOW chaining grammar treats them as distinct LLM
// affordances.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::plan::{PlanEngine, PlanId, PlanRunId, PlanSpec};
use crate::tool::Tool;

pub struct PlanCreateTool {
    schema: Value,
    engine: Arc<PlanEngine>,
}

impl PlanCreateTool {
    pub fn new(engine: Arc<PlanEngine>) -> Self {
        Self {
            engine,
            schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Plan id. Reusing an id replaces the previous spec." },
                    "spec": {
                        "type": "object",
                        "description": "PlanSpec value: name + ordered steps with transitions.",
                        "properties": {
                            "name":  { "type": "string" },
                            "steps": { "type": "array" }
                        },
                        "required": ["name", "steps"]
                    }
                },
                "required": ["id", "spec"]
            }),
        }
    }
}

#[async_trait]
impl Tool for PlanCreateTool {
    fn name(&self) -> &str {
        "codeless.plan.create"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'id'"))?
            .to_string();
        let spec_v = args
            .get("spec")
            .ok_or_else(|| ToolError::invalid_args("missing 'spec'"))?;
        let spec: PlanSpec = serde_json::from_value(spec_v.clone())
            .map_err(|e| ToolError::invalid_args(format!("bad spec: {e}")))?;
        self.engine
            .register_plan(PlanId::new(&id), spec)
            .await
            .map_err(|e| ToolError::invalid_args(format!("register: {e}")))?;
        Ok(json!({ "created": true, "id": id }))
    }
}

pub struct PlanStartTool {
    schema: Value,
    engine: Arc<PlanEngine>,
}

impl PlanStartTool {
    pub fn new(engine: Arc<PlanEngine>) -> Self {
        Self {
            engine,
            schema: json!({
                "type": "object",
                "properties": {
                    "plan_id": { "type": "string", "description": "Id of a plan previously registered via codeless.plan.create." }
                },
                "required": ["plan_id"]
            }),
        }
    }
}

#[async_trait]
impl Tool for PlanStartTool {
    fn name(&self) -> &str {
        "codeless.plan.start"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let plan_id = args
            .get("plan_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'plan_id'"))?
            .to_string();
        let run_id = self
            .engine
            .start_run(&PlanId::new(&plan_id))
            .await
            .map_err(|e| ToolError::invalid_args(format!("start: {e}")))?;
        Ok(json!({ "started": true, "plan_id": plan_id, "run_id": run_id.as_str() }))
    }
}

pub struct PlanListTool {
    schema: Value,
    engine: Arc<PlanEngine>,
}

impl PlanListTool {
    pub fn new(engine: Arc<PlanEngine>) -> Self {
        Self {
            engine,
            schema: json!({
                "type": "object",
                "properties": {
                    "include": {
                        "type": "string",
                        "enum": ["plans", "runs", "both"],
                        "description": "What to return. Defaults to 'both'."
                    }
                }
            }),
        }
    }
}

#[async_trait]
impl Tool for PlanListTool {
    fn name(&self) -> &str {
        "codeless.plan.list"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let include = args
            .get("include")
            .and_then(Value::as_str)
            .unwrap_or("both");
        let mut out = serde_json::Map::new();
        if matches!(include, "plans" | "both") {
            let plans: Vec<Value> = self
                .engine
                .list_plans()
                .await
                .into_iter()
                .map(|(id, spec)| json!({ "id": id.as_str(), "spec": spec }))
                .collect();
            out.insert("plans".into(), Value::Array(plans));
        }
        if matches!(include, "runs" | "both") {
            let runs: Vec<Value> = self
                .engine
                .list_runs()
                .await
                .into_iter()
                .map(|(id, state)| {
                    json!({
                        "id": id.as_str(),
                        "plan_id": state.plan_id.as_str(),
                        "status": status_to_json(&state.status),
                        "history": state
                            .history
                            .iter()
                            .map(|(step, outcome)| {
                                json!({ "step": step.as_str(), "outcome": format!("{outcome:?}") })
                            })
                            .collect::<Vec<_>>(),
                    })
                })
                .collect();
            out.insert("runs".into(), Value::Array(runs));
        }
        Ok(Value::Object(out))
    }
}

fn status_to_json(s: &crate::plan::PlanRunStatus) -> Value {
    use crate::plan::PlanRunStatus::*;
    match s {
        Running {
            current_step,
            current_job,
        } => json!({
            "kind": "running",
            "current_step": current_step.as_str(),
            "current_job": current_job.to_string(),
        }),
        Done { last_step } => json!({
            "kind": "done",
            "last_step": last_step.as_str(),
        }),
        Failed { at_step, error } => json!({
            "kind": "failed",
            "at_step": at_step.as_str(),
            "error": error,
        }),
    }
}

pub struct PlanCancelTool {
    schema: Value,
    engine: Arc<PlanEngine>,
}

impl PlanCancelTool {
    pub fn new(engine: Arc<PlanEngine>) -> Self {
        Self {
            engine,
            schema: json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string" },
                    "reason": {
                        "type": "string",
                        "description": "Free-text reason recorded on the run's failure status."
                    }
                },
                "required": ["run_id"]
            }),
        }
    }
}

#[async_trait]
impl Tool for PlanCancelTool {
    fn name(&self) -> &str {
        "codeless.plan.cancel"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        let run_id = args
            .get("run_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("missing 'run_id'"))?
            .to_string();
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("plan.cancel tool")
            .to_string();
        let cancelled = self
            .engine
            .cancel_run(&PlanRunId::new(&run_id), &reason)
            .await;
        Ok(json!({ "cancelled": cancelled, "run_id": run_id }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{JobSpawner, PlanRunId, SpawnError, StepId};
    use crate::testing::fake_ctx;
    use codeless_types::id::JobId;

    struct StubSpawner;
    #[async_trait]
    impl JobSpawner for StubSpawner {
        async fn spawn(&self, _: &PlanRunId, _: &StepId, _: &str) -> Result<JobId, SpawnError> {
            Ok(JobId::new())
        }
    }

    fn linear_spec_json() -> Value {
        json!({
            "name": "p",
            "steps": [
                { "id": "a", "job_template": "tpl-a", "on_success": "stop", "on_failure": "stop" }
            ]
        })
    }

    #[tokio::test]
    async fn create_start_list_cancel_round_trip() {
        let engine = Arc::new(PlanEngine::new(Arc::new(StubSpawner)));
        let create = PlanCreateTool::new(engine.clone());
        let start = PlanStartTool::new(engine.clone());
        let list = PlanListTool::new(engine.clone());
        let cancel = PlanCancelTool::new(engine.clone());

        let r = create
            .call(
                &fake_ctx().ctx,
                json!({ "id": "p", "spec": linear_spec_json() }),
            )
            .await
            .unwrap();
        assert_eq!(r["created"], json!(true));

        let r = start
            .call(&fake_ctx().ctx, json!({ "plan_id": "p" }))
            .await
            .unwrap();
        let run_id = r["run_id"].as_str().unwrap().to_string();

        let r = list.call(&fake_ctx().ctx, json!({})).await.unwrap();
        assert_eq!(r["plans"].as_array().unwrap().len(), 1);
        assert_eq!(r["runs"].as_array().unwrap().len(), 1);

        let r = cancel
            .call(&fake_ctx().ctx, json!({ "run_id": run_id }))
            .await
            .unwrap();
        assert_eq!(r["cancelled"], json!(true));
    }

    #[tokio::test]
    async fn start_on_unknown_plan_errors() {
        let engine = Arc::new(PlanEngine::new(Arc::new(StubSpawner)));
        let start = PlanStartTool::new(engine);
        let err = start
            .call(&fake_ctx().ctx, json!({ "plan_id": "missing" }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::InvalidArgs(_)),
            "expected InvalidArgs, got {err:?}"
        );
    }
}
