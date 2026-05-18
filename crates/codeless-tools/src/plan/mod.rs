//! In-memory Plan engine (P1) — pure data only at this stage.
//!
//! Mirrors the `schedule/` and `email/` layout: `spec` is data with
//! no I/O, later sub-modules will own the engine, job-spawner trait,
//! and event-bus wiring. See `DOCS/JOB-WORKFLOW.md` "Job chaining"
//! for the P1 → P3 sequencing.

pub mod dispatch;
pub mod engine;
pub mod spec;

pub use dispatch::{LogJobSpawner, StartPlanRunAction, START_PLAN_RUN_KIND};
pub use engine::{
    JobSpawner, Outcome, PlanEngine, PlanEngineError, PlanId, PlanRunId, PlanRunState,
    PlanRunStatus, SpawnError,
};
pub use spec::{PlanSpec, PlanSpecError, PlanStep, StepId, Transition};
