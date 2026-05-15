# Chat for this job

## user @ 2026-05-15T04:29:42Z

let me know rough idea of the scope

## assistant @ 2026-05-15T04:29:55Z

Here's the rough scope:

**What this job is fixing:** A regression where opening two job-detail tabs simultaneously leaves one blank. The design intent of `JobDetailStack` is to mount all job-detail `JobPage` instances in parallel and toggle visibility — so switching is instant. Something is breaking that.

**Investigation targets (3 likely culprits):**
1. SSE/event stream singleton keyed by connection instead of `(connection, jobId)` — second mount swaps the server-side scope
2. Zustand chat/session store with a "current job" slice not keyed by `jobId` — second mount clobbers first's pointer
3. `useJob(jobId)` cache with a shared in-flight-request map not keyed by `jobId`

**Size of the job:** Small-to-medium. All changes are in `codeless/ui/codeless-ui/src/` — no Rust, no backend. The constraint list is tight: no remount workaround, no collapsing to one active `JobPage`, no `key={jobId}` trick.

**Deliverables:**
- SCOPE.md §Reproduction + §Root cause filled in
- One failing RTL test (mounts two `JobPage` instances against different `jobId`s with `MockRpcClient`)
- One fix commit per distinct root cause (could be one, could be two or three)
- SCOPE.md §Manual verification filled in
- PR to `master`

**Two REVIEW gates** — one before the fix lands (to validate the root cause + failing test), one before the PR merges.

Rough effort: 2–4 hours of agent time spread across 7 stages, assuming the root cause is a single shared singleton. If two or three independent sites are involved, add a fix commit per site but the investigation cost is the same.
