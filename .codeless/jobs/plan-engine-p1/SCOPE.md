# Scope — plan-engine-p1

## Goal

Land Phase **P1** of the Plan engine described in
[`DOCS/JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md) under "Job
chaining — the next job after this one loop". P1 is the **reusable
in-memory library + tool surface, no UI, no SQLite**. Success is:

- A `codeless-tools::plan` module with pure-data types and an
  in-memory engine, mirroring the shape of the existing
  `codeless-tools::schedule` and `codeless-tools::email` modules.
- `codeless.plan.{create,start,list,cancel}` tools the LLM (or a
  script, or a scheduled Action) can call.
- `codeless-runtime` constructs the engine at boot, the engine
  subscribes to the runtime's terminal Job events
  (`JobFinished` / `JobFailed` / `JobStopped`), and a fired
  `Schedule` whose `Action` says "start Plan X" turns into a
  one-line call into `PlanEngine::start_run(plan_id)`.

That last bullet is the **proof the boundary is right**, per
JOB-WORKFLOW.md (P1) — the composition `Schedule → Action →
PlanEngine` must be trivial.

## In scope

- `codeless-tools/src/plan/spec.rs` — `PlanSpec`, `PlanStep`,
  `Transition` (target = step id, `stop`, or omitted = stop), serde
  + validation. Mirrors `codeless-tools::schedule::spec`.
- `codeless-tools/src/plan/engine.rs` — `PlanEngine`, with an
  injected `JobSpawner` trait so the engine has zero coupling to
  `codeless-runtime`. The engine subscribes to terminal Job events
  (via the same `EventSource` abstraction `codeless-bot-core`
  already uses) and walks each PlanRun's state machine.
- `codeless-tools/src/plan/mod.rs` — public re-exports, doc comment
  framed the same way `schedule/mod.rs` is.
- `codeless.plan.create` / `start` / `list` / `cancel` tool
  registrations, matching the shape of the existing schedule tool
  wrappers.
- `codeless-runtime` boot wiring: construct one `PlanEngine` per
  runtime, give it a `JobSpawner` impl that talks to the existing
  job submission path, subscribe it to the event bus.
- A new `Action` variant (or equivalent dispatch entry) in the
  schedule module so a scheduled fire can call
  `PlanEngine::start_run(plan_id)`. Whatever the cleanest fit is
  given the current `Action` / `ActionFn` / `PayloadDispatcher`
  shape — survey before deciding.

## Out of scope

- **SQLite persistence.** No `plans` / `plan_steps` / `plan_runs` /
  `plan_run_steps` tables. The engine is in-memory; a restart loses
  in-flight PlanRuns. Document this as a known limit. That work is
  **P2**.
- **UI.** No CodeMirror Plan editor, no PlanRun graph view, no
  re-run dialog. That work is **P3**.
- **DAG primitives.** No `fan_out:`, no `fan_in:`, no `when:`
  predicates. Linear chain only, with `on_success` / `on_failure`
  per step. Per JOB-WORKFLOW.md: "Resist the urge to ship a full
  DAG."
- **Plan-of-Plans.** A `PlanStep` spawns a Job, never another Plan.
- **Cross-repo coordination.** A step's Job runs in the repo the
  step names. `mani` is still how cross-repo work happens.
- **Conditional re-runs of the same step.** No loop-back; retries
  remain a per-Job concern (JOB-WORKFLOW (A)/(B)).
- **Bot adapters surfacing PlanRun events to Slack / Telegram.**
  Mentioned in JOB-WORKFLOW open-question 3; belongs in
  `codeless-bot-core::notify` later, not this job.

## Constraints

1. **R1 — crate dependency direction.** `codeless-tools` is where
   bot/tool surfaces live. No `std::process`, no `tokio::process`,
   no host-only crate imports from inside
   `codeless-tools/src/plan/`. The engine reaches the rest of the
   world only via the `JobSpawner` trait and the `EventSource` it
   subscribes to.
2. **R2 — comments explain *why*, not *what*.** No emojis. No
   "added in P1" / "TODO for P2" status comments. No restatements.
   The module-level doc comments in `schedule/mod.rs` and
   `schedule/spec.rs` are the model — follow that voice.
3. **R3 — one file, one concept.** `spec.rs` is data + validation.
   `engine.rs` is the state machine. Tool wrappers live next to
   the schedule tool wrappers, not jammed into the same file.
4. **R5 — tests live with the code.** Every `spec.rs` validation
   path has a unit test. The engine has tests that drive it with a
   mock `JobSpawner` and a hand-driven event stream, covering at
   minimum: linear chain success, mid-chain failure with
   `on_failure: stop`, mid-chain failure with an
   `on_failure: <recover-step>` handler, and cancellation while a
   step's Job is in flight.
5. **Mirror existing module shapes.** `plan/` should look like a
   sibling of `schedule/` — same file split (`mod.rs` / `spec.rs` /
   `engine.rs`), same doc-comment voice, same exported-surface
   discipline. Do not invent a new convention here.
6. **MSRV 1.78 + clippy `-D warnings` + `cargo fmt --check` all
   green** before any commit lands. Use `mani` for commit + push
   per `CLAUDE.md`, never raw git.

## Open questions

These must be resolved in stage 1 (survey) and recorded in
`handover.md` before any code stage starts:

1. **`Action` variant vs. `PayloadDispatcher` route.** The schedule
   module exposes both `Action` / `ActionFn` and a
   `PayloadDispatcher` (re-exported from `dispatch.rs`). Which is
   the right seam for "fire calls `PlanEngine::start_run`"?
   Survey `dispatch.rs` end-to-end before choosing.
2. **Where does the `EventSource` for terminal Job events come
   from?** Confirm whether `codeless-bot-core::outbound`'s
   `EventSource` is reusable as-is, or whether the engine subscribes
   directly to the runtime event bus. Pick the path with fewer new
   abstractions.
3. **Naming the cross-cutting variant.** Is it
   `Action::StartPlanRun { plan_id }`? Or does the schedule's
   `Action` stay opaque and the host wires a closure that calls
   the engine? Both are valid; the closure path keeps
   `codeless-tools::schedule` ignorant of plans, which is cleaner
   layering. Decide in the REVIEW after stage 1.
4. **What does the `JobSpawner` trait return?** A spawned Job id is
   the minimum so the engine can match terminal events back to the
   step. Confirm whether the existing submit-job path returns
   synchronously or via the event bus, and shape the trait to
   match.
5. **Cancellation semantics.** JOB-WORKFLOW open-question 1 for
   Plan says "probably stop the in-flight Job — confirm at P2." For
   P1, default to "cancellation marks the PlanRun cancelled and
   lets the in-flight Job finish naturally", and document that
   choice explicitly. Revisit at P2.

## References

- [`DOCS/JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md) — the
  authoritative design; sections "Job chaining" through "Naming
  inside the code".
- [`crates/codeless-tools/src/schedule/`](../../../crates/codeless-tools/src/schedule/)
  — the layout to mirror.
- [`crates/codeless-bot-core/src/outbound/`](../../../crates/codeless-bot-core/src/outbound/)
  — the `EventSource` pattern to reuse.
- [`DOCS/JOB-MODEL.md`](../../../DOCS/JOB-MODEL.md) — Job/Run
  vocabulary the Plan layer composes on top of.
- [`CLAUDE.md`](../../../CLAUDE.md) — R1–R5 rules.
