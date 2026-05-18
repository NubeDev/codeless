## Done

- reviewed stage 0+1 survey commits (d040f96, 3e295b3) — handover.md only, no source changes
- confirmed R1 (codeless-tools host-only, JobSpawner kept free of codeless-runtime types), single EventBus consumer, in-memory P1, no wire-format edits
- confirmed transition vocabulary on_success/on_failure/stop and absence of Workflow* naming

## Next

- (none) — fresh session picks up Stage 3 (define PlanSpec/PlanStep/Transition in codeless-tools/src/plan/spec.rs)

## What you need to know

- PASS: Layer-1 invariants hold — survey is docs-only, names schedule/email as mirror layout, keeps JobSpawner trait local to plan module, and treats the plan engine as a second EventBus consumer (no new transport).
- Real terminal variants are JobCompleted/JobFailed/JobStopped (not JobFinished as JOB-WORKFLOW.md says); Stage 3 should use the real names.
- Survey leans toward PlanSpec naming (mirroring Schedule); job goal lists Plan/PlanStep/PlanRun/PlanRunStep — Stage 3 reviewer should resolve before code lands.

## Open questions

- Plan vs PlanSpec for the pure-data root type (survey says PlanSpec for symmetry with schedule/email; job spec says Plan) — decide in Stage 3 before spec.rs is written.
