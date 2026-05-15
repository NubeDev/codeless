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

## user @ 2026-05-15T04:32:33Z

can you add the scope then when in the JOBS page and in the chat, when the chat is in spec mode i dosnt persist, so this is annoying, maybe its a backend issue not sure, pleasee add this to the scope

## assistant @ 2026-05-15T04:34:09Z

I'll read the current spec files and then add the new bug.I'll add the spec-mode chat persistence bug as a second tracked bug in both `SCOPE.md` and `template.yaml`.Now update the "In scope" and "Deliverables" sections, then add the new investigation sections and stages.Now add the new investigation placeholder sections and update the References section.Now add the new reference and update `template.yaml` with the new stages and goal.Now update `template.yaml` — extend the goal and add the three new stages.Done. Here's what changed:

**SCOPE.md:**
- Renamed the heading to reflect both bugs
- Added a **Bug 2** section describing the spec-mode chat persistence issue, with the three most likely root causes to check in order (backend skips persist, frontend calls wrong RPC method, SSE arrives but DB write is skipped)
- Expanded "In scope" and "Out of scope" to cover Bug 2
- Added deliverables 5–7 for Bug 2
- Added §"Spec-mode chat root cause" and §"Spec-mode chat fix verification" placeholder sections
- Updated References

**template.yaml:**
- Expanded the `goal:` to describe both bugs
- Added three new stages (8–10): trace the lifecycle, apply the minimal fix, verify persistence

The sequencing keeps Bug 1 fully closed (including its REVIEW gate) before Bug 2 stages begin, so the two investigations don't interfere.

## user @ 2026-05-15T04:34:46Z

oh wait maybe it does work, just add in to confirm it should work....

## assistant @ 2026-05-15T04:35:15Z

I'll update the spec so Bug 2 is framed as "confirm this works" rather than a confirmed regression.Now update `template.yaml` to match — the goal and stage 8 should reflect "verify, fix only if broken":Done. Stage 8 now starts with "verify first — if it already works, skip stages 9 and 10." The SCOPE.md Bug 2 section is reframed as "unconfirmed, may already work." No investigation or fix happens unless the verification actually shows messages are lost.
