## Done

- Added four `codeless.plan.{create,start,list,cancel}` tools wrapping a shared `Arc<PlanEngine>` (codeless-tools/src/tools/plan_tool.rs).
- Added `StartPlanRunAction` (schedule Action) + `LogJobSpawner` in codeless-tools/src/plan/dispatch.rs; exposed `START_PLAN_RUN_KIND = "start_plan_run"`.
- Engine grew `list_plans`, `list_runs`, `cancel_run` for the tool surface; state-machine semantics unchanged.
- `codeless-runtime::plan_subscribe::spawn_plan_engine_subscriber` forwards bus envelopes into the engine; called once at boot from codeless-cli/serve.rs alongside the existing notifier/stage-recorder wiring.
- codeless-mcp/main.rs constructs its own engine, registers the four tools and the StartPlanRunAction on the PayloadDispatcher.
- Committed as c87acc8 on codeless/plan-engine-p1.

## Next

- Stage 7 (final) per JOB-WORKFLOW P1.
- Replace `LogJobSpawner` with a runtime spawner that calls `InProcessRpc::submit_job` so PlanRuns actually drive the queue (P2 territory).
- Reconcile the two-engine reality: codeless-mcp and codeless-cli/serve.rs each construct a separate in-memory engine; either fold the MCP tools onto the server's engine via RPC, or move both into one process before P2 lands persistence.

## What you need to know

- ai-runner's `Cargo.toml` `workspace = "../job-…"` points at a sibling worktree. To run cargo locally I swapped it to this worktree's id, then swapped it back before commit so it isn't part of the diff. Any future cargo invocation from this worktree needs the same patch-and-revert.
- I committed with raw git rather than mani; this is a headless job worktree and bin/mani is not present.
- Engine subscribes with `SubscribeFilter::All`, `since=None` — no replay, matching P1's in-memory restart-wipes-state contract.
- `cancel_run` only mutates engine bookkeeping; it does not actually stop the underlying spawned Job (documented inline; needs a host-side runner-cancel call to become real).

## Open questions

- The stage spec says "wire codeless-runtime so the engine is constructed once at boot." I read "boot" as the codeless-cli serve path (where InProcessRpc actually lives); confirm this is the intended boot site, or whether the engine should instead be an `InProcessRpc::with_plan_engine(...)` field so the runtime library owns it.
- `start_plan_run` dispatcher kind currently only wired in codeless-mcp's scheduler. codeless-cli/serve.rs has no scheduler today — open whether that's fine for P1.
