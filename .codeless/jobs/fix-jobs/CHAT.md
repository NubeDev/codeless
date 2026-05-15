# Chat for this job

## user @ 2026-05-15T04:29:59Z

let me know rough idea of the scope

## assistant @ 2026-05-15T04:30:10Z

The job is narrowly scoped — a single UI regression fix, no backend changes, no architectural rewrites.

**What it is:** Two job-detail tabs mounted simultaneously by `JobDetailStack`, but one renders blank. The fix must preserve the "all mounted, toggle visibility" design.

**Investigation targets (pick 1-2, likely):**
- SSE subscription manager — module-level singleton not keyed by `(connection, jobId)`
- Zustand chat store — a "current job/session" slice clobbered by the second mount
- `useJob` query cache — shared in-flight map resolving into the wrong instance

**Deliverables:**
1. Failing RTL test (two `JobPage` instances, `MockRpcClient`, both must resolve independently)
2. Minimal fix at the shared-state site — one commit per root cause
3. Manual verification note in `SCOPE.md`
4. PR to `master`

**Two REVIEW gates** (human approval required before fixing, and before merging).

**Explicit out of scope:** Rust/backend, draft-job UX, workspace-attach work, any "serialize to one tab" workaround, `key={jobId}` remount hack.

Rough effort: stages 1–2 are read-only investigation (~S each), stage 3 is the failing test (~M), stage 5 is the fix (~M), stage 6 is verification (~S). Total wall-clock with reviews gating between them: probably 2–3 sessions.
