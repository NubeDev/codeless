## Done

- Added `crates/codeless-tools/src/plan/engine.rs`: `PlanEngine`, `JobSpawner` async trait, `PlanId`/`PlanRunId`/`PlanRunState`/`PlanRunStatus`, `PlanEngineError`, internal `Outcome`. State held in a single `Mutex<Inner>` with `plans`, `runs`, `job_index`, and a monotonic run-id counter.
- `register_plan` validates the spec and stores it. `start_run` spawns the first step and indexes the returned `JobId`. `handle_event` drains terminal `Event::JobCompleted|JobFailed|JobStopped` envelopes, advances the state machine (success/failure transition), spawns the next step, and records a `(StepId, Outcome)` history trace. Non-terminal and unrelated-job envelopes are no-ops.
- 8 tokio tests using a `MockSpawner` (records calls + replays a queue of `Result<JobId,_>`) and hand-built `EventEnvelope`s. All 13 plan tests pass; `cargo clippy -p codeless-tools --all-targets -- -D warnings` is clean.
- Drive-by fix to `plan/spec.rs`: replaced manual `Default for Transition` with `#[derive(Default)]` + `#[default]` to clear a pre-existing clippy::derivable_impls error that was blocking the gate.
- Re-exported the new types from `plan/mod.rs`.
- Committed as `94551fa` on branch `codeless/plan-engine-p1`.

## Next

- Stage 5: wire `PlanEngine` to an actual `EventEnvelope` stream — define the minimal `EventSource` trait inside the plan module (per stage-1 survey), add `PlanEngine::spawn(source) -> JoinHandle + shutdown oneshot` mirroring `codeless-bot-core::outbound::OutboundPublisher`. The hand-driven test stays valid; add one extra test that feeds events through a `tokio::sync::mpsc` to exercise the loop.
- Stage 6: `codeless.plan.create|start|list|cancel` tool in `crates/codeless-tools/src/tools/`, mirroring `tools/schedule_create.rs` (single tool, `action:` enum arg).
- Stage 7: integration glue showing a fired `Schedule` `Action` calling `PlanEngine::start_run` (a `StartPlanAction` in the host wiring layer, per stage-1 survey §"How a fired Schedule becomes start_run").

## What you need to know

- The plan engine's event hookup intentionally stops at `handle_event(&EventEnvelope)`. The host is expected to wire a runtime `EventBus::subscribe_since(All, None)` stream into a loop that calls `handle_event` per envelope. This stage proved the state machine in isolation; stage 5 adds the loop.
- `JobSpawner::spawn` is given `(PlanRunId, StepId, &str job_template)`. The crate stays runtime-agnostic — no `codeless-runtime` types crossed into `codeless-tools`. The eventual host impl will translate `job_template` into the runtime's job-spawn surface.
- `Outcome::Stopped` walks the `on_failure` edge (binary success/failure vocabulary, matching the JOB-WORKFLOW P1 spec). If a future stage needs a third edge, add `on_stopped`.
- Late terminal events after `Done`/`Failed` are silently dropped because the `job_index` entry is removed on first observation; replays are no-ops.
- The `Inner { plans, runs, job_index, .. } = &mut *g` split-borrow pattern in `handle_event` is load-bearing — combining `plans.get` (immutable) with `runs.get_mut` (mutable) through `g` directly trips borrowck.
- Workspace gotcha for any agent compiling here: `../ai-runner/Cargo.toml` has a hard `workspace = "../job-…"` pointer that other worktrees keep flipping to their own path; you have to temporarily flip it to your worktree id to run `cargo {test,clippy,fmt}`. I restored the pointer after each invocation; do the same.
- Pre-existing fmt drift exists in `crates/codeless-tools/src/schedule/dispatch.rs` and `plan/spec.rs` (the `assert_eq!(serde_json::to_string(&Transition::Stop)…)` line). Not introduced here; left alone per R4. A future stage should run `cargo fmt -p codeless-tools` as a dedicated cleanup commit.

## Open questions

- Should `register_plan` reject re-registration of an existing `PlanId` while runs are in flight? Currently it overwrites; the doc-comment flags this as undefined. Worth pinning down before the `codeless.plan.create` tool lands (stage 6).
- `PlanRunId` is currently `"run-<u64>"` from a per-engine counter. If the host needs to correlate across processes, swap to a ulid (matching the rest of the type-id surface). Easy to change; decided to keep it deterministic for tests this stage.
