## Done

- reviewed the engine boundary in crates/codeless-tools/src/plan/engine.rs against the doc's claim and the Layer-1 invariants
- confirmed PlanEngine holds only Arc<dyn JobSpawner> + in-memory maps; host wires envelopes in via handle_event
- confirmed codeless-tools dep set (codeless-types only on the engine path) keeps R1 intact and no process spawning leaks in
- verified schedule -> plan composition reduces to engine.start_run(&plan_id) at the Action call site

## Next

- (none) — stage 5 is a review gate; stage 6 picks up in a fresh session

## What you need to know

- PASS: JobSpawner is the only host coupling, engine takes no tokio runtime handle, and a fired schedule Action calling PlanEngine::start_run(plan_id) is the one-liner the doc promised.
- engine.rs does run `JobSpawner::spawn` under released lock (re-acquires Mutex after await) — correct, worth preserving in stage-6 wiring
- handle_event removes the join-index entry on first sight of the JobId, so replayed terminal envelopes are intentionally no-ops; the late-event test pins this

## Open questions

- (none)
