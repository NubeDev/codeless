## Done

- Wrote the survey findings into `handover.md` at the worktree root and committed them as `stage 1: survey existing surfaces for plan engine P1`. No source files were modified — this stage is read-only investigation by design (see SCOPE.md stage 1).
- Wrote this stage-summary `handover.md` so the next stage opens with the Done/Next/Know contract the runtime expects.
- The survey itself (read-only) covered: DOCS/JOB-WORKFLOW.md §"Job chaining" (P1 bullet list + linear transition vocabulary), the existing schedule + email module layouts in codeless-tools, the runtime event bus and `codeless-types::Event` terminal variants, and codeless-bot-core's outbound EventSource + run-loop pattern. Full notes are in the worktree-root handover.md; path-level references live under "What you need to know" below, not here, because nothing under those paths was written.

## Next

- Stage 2 should start on `crates/codeless-tools/src/plan/spec.rs` (`PlanSpec`, `PlanStep`, `Transition`) per the layout fixed in handover.md.

## What you need to know

Paths in this section are read-as-reference for stage 2+, not work produced by stage 1.

- Mirror target for the new `plan/` module: the existing `codeless-tools` `schedule/` layout (mod / spec / scheduler / dispatch) and the `email/` layout (message / mailer / one transport impl). Same file split, same doc-comment voice.
- DOCS/JOB-WORKFLOW.md says "JobFinished" but the real variant in `codeless-types::Event` is `JobCompleted` (wire label `job-completed`). The three terminal events for the plan engine are `JobCompleted`, `JobFailed`, `JobStopped`. `JobPaused` is not terminal.
- `codeless-tools` cannot depend on `codeless-bot-core` or `codeless-runtime`. The `EventSource` trait shape from outbound.rs is three lines — re-declare it inside the plan module rather than reach across crates.
- `JobSpawner` should keep `codeless-runtime` types out — use `codeless-types::JobId` (mobile-safe) and a string `job_template`.
- The scheduled-plan boundary is the schedule module's `PayloadDispatcher` + a host-side `StartPlanAction` whose payload carries `{"kind":"start_plan","plan_id":"..."}`; the library does not own this wiring (matches `LogAction` precedent).

## Open questions

- Stage 2 must decide whether `Transition` is `enum Transition { Step(StepId), Stop }` or `Option<StepId>` (spec says "omitted = stop", which leans toward `Option`). Noted in handover.md.
- Stage 3+ must decide whether `JobSpawner` returns just `JobId` (engine owns one bus subscription, dispatches by `job_id`) or a per-spawn `oneshot`. Outbound's pattern argues for the former; recorded in handover.md.
