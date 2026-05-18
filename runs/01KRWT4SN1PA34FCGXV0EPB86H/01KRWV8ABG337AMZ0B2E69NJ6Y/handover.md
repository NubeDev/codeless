## Done

- Added `crates/codeless-tools/src/plan/{mod.rs,spec.rs}` with `PlanSpec`, `PlanStep`, `Transition` (+ `StepId`, `PlanSpecError`), serde, and `PlanSpec::validate()` enforcing unique step ids and resolvable transition targets.
- Wired `pub mod plan;` into `codeless-tools/src/lib.rs`.
- 5 unit tests pass (linear release-prep chain, duplicate id, unknown target, serde round-trip for `"stop"`/step, omitted-field default = `Stop`) — verified standalone since the worktree workspace cannot resolve `../ai-runner` and `ai-ui-core`.
- Committed as `303f183` on `codeless/plan-engine-p1` with the stage-title prefix.

## Next

- Stage 4: introduce the `JobSpawner` trait and an in-memory `PlanEngine` skeleton (no event-bus wiring yet), following the handover's note that the engine itself owns the bus subscription and dispatches by `JobId`.

## What you need to know

- Workspace edition is 2021, so let-chains are not available — `check_target` uses a nested `if let` instead.
- Transition is modelled as an explicit `enum { Step(StepId), Stop }` (not `Option<StepId>`); `Default` is `Stop` so `#[serde(default)]` on the `PlanStep` fields gives the "omitted = stop" semantic from JOB-WORKFLOW.md.
- `cargo test -p codeless-tools` fails in the worktree because the workspace's `../ai-runner` and `ai-ui-core` paths are not present here; this is environmental, not a code issue. Standalone build of `spec.rs` against serde/serde_json/thiserror compiles cleanly and all 5 tests pass.
- No mani / push performed (worktree is isolated; ran raw git commit only because mani wasn't available here).

## Open questions

- Should `StepId` get a stricter parser (forbid the literal `"stop"` as a step id) so a step named `stop` can't shadow the reserved transition target? Today `PlanSpec { steps: [{ id: "stop", ... }] }` would validate but its `on_success: "stop"` would mean "terminate," never "go to step stop." Defer to Stage 4 once the engine semantics are pinned.
