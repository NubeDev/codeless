## Done

- Surveyed DOCS/JOB-WORKFLOW.md §"Job chaining" (lines 431–600) including the (P1) bullet list and the linear transition vocabulary.
- Surveyed `codeless-tools/src/schedule/` (mod/spec/scheduler/dispatch) and `codeless-tools/src/email/` to fix the layout the new `plan/` module must mirror.
- Surveyed `codeless-runtime::event_bus::{EventBus, SubscribeFilter, EventEnvelope}` and `codeless-types::Event` terminal variants.
- Surveyed `codeless-bot-core::outbound` `EventSource` trait + `OutboundPublisher::spawn` run-loop pattern as the second-consumer template.
- Wrote `handover.md` at repo root and committed it as `stage 1: survey existing surfaces for plan engine P1`.

## Next

- Stage 2 should start on `crates/codeless-tools/src/plan/spec.rs` (`PlanSpec`, `PlanStep`, `Transition`) per the layout fixed in handover.md.

## What you need to know

- DOCS/JOB-WORKFLOW.md says "JobFinished" but the real variant in `codeless-types::Event` is `JobCompleted` (wire label `job-completed`). The three terminal events for the plan engine are `JobCompleted`, `JobFailed`, `JobStopped`. `JobPaused` is not terminal.
- `codeless-tools` cannot depend on `codeless-bot-core` or `codeless-runtime`. The `EventSource` trait shape from outbound.rs is three lines — re-declare it inside the plan module rather than reach across crates.
- `JobSpawner` should keep `codeless-runtime` types out — use `codeless-types::JobId` (mobile-safe) and a string `job_template`.
- The scheduled-plan boundary is `schedule::PayloadDispatcher` + a host-side `StartPlanAction` whose payload carries `{"kind":"start_plan","plan_id":"..."}`; the library does not own this wiring (matches `LogAction` precedent).

## Open questions

- Stage 2 must decide whether `Transition` is `enum Transition { Step(StepId), Stop }` or `Option<StepId>` (spec says "omitted = stop", which leans toward `Option`). Noted in handover.md.
- Stage 3+ must decide whether `JobSpawner` returns just `JobId` (engine owns one bus subscription, dispatches by `job_id`) or a per-spawn `oneshot`. Outbound's pattern argues for the former; recorded in handover.md.
