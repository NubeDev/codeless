## Done

- Rewrote `DOCS/JOB-WORKFLOW.md` §"(P1) — Reusable Plan library + tool, no UI." as a checked-off checklist that names the actual landed modules: `codeless-tools/src/plan/{spec,engine,dispatch}.rs`, `codeless-tools/src/tools/plan_tool.rs` + `tools/mod.rs` registration, and the `codeless-mcp/src/main.rs` boot wiring.
- Recorded the in-memory-only / restart-loses-runs caveat, the MCP-process scope caveat (engine not wired into the runtime event bus yet — that moves to P2), and the linear-chains-only caveat under a "Known limits" subsection.
- Listed the four `codeless.plan.*` tools (create / start / list / cancel) inline next to the tool-surface bullet — there is no separate tool-index doc in the repo, so JOB-WORKFLOW.md's P1 section is the index.
- Committed as `dc80537` on `codeless/plan-engine-p1` with the stage-7 title prefix.

## Next

- (none) — final stage of the job.

## What you need to know

- There is no standalone "tool index" doc in `DOCS/` (no TOOLS.md). Existing tools like `ScheduleCreateTool` are not catalogued anywhere either. The JOB-WORKFLOW.md (P1) bullet is the only place that names the four `codeless.plan.*` tools; if a future stage wants a real index, it should be a new file.
- The runtime-side `EventSource` subscription mentioned in earlier handovers was not landed in P1 — only the MCP-process engine in `codeless-mcp/src/main.rs` exists. That gap is now documented as a known limit, not as a missing P1 deliverable.
- Repo-relative link convention in `DOCS/` is `../crates/...` (not `../codeless/crates/...`); I matched the existing style after an initial overshoot.

## Open questions

- (none)
