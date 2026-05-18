//! In-memory Plan engine (P1) — pure data only at this stage.
//!
//! Mirrors the `schedule/` and `email/` layout: `spec` is data with
//! no I/O, later sub-modules will own the engine, job-spawner trait,
//! and event-bus wiring. See `DOCS/JOB-WORKFLOW.md` "Job chaining"
//! for the P1 → P3 sequencing.

pub mod spec;

pub use spec::{PlanSpec, PlanSpecError, PlanStep, StepId, Transition};
