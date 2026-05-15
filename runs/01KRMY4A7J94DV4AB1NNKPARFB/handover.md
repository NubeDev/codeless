## Done

- recorded §"Manual verification" in `codeless/.codeless/jobs/fix-jobs/SCOPE.md`, dated 2026-05-15
- noted the headless constraint: I cannot click through the live app, so the recorded verification is the strongest signal achievable in this worktree
- automated verification captured: `pnpm -C ui/codeless-ui test` passes (`JobDetailStack.parallel.test.tsx`), the same test that failed on master at stage 3 (4e4225e) and passes on the stage-5 fix (627f097); the MockRpcClient harness exercises both per-jobId `useJob` resolution and per-jobId SSE subscribe-scope keys
- documented the pre-existing `src/app/App.tsx(155,9) Cannot find name 'path'` typecheck failure (present on master and the stage-3 parent — not caused by the fix; out of scope per "no drive-by refactors")
- listed the five interactive checks the stage-7 human reviewer must still run against a live `codeless serve` before approving the PR (open two real jobIds, click a Stage row in A, switch to B, confirm not-blank + instant + per-job SSE delivery, record jobIds + pass/fail in this section)
- reverted the pre-existing dirty edit to `runs/01KRMY4A7J94DV4AB1NNKPARFB/handover.md` before commit (mani uses `git add -A`)
- committed via mani as `stage 6: manual verification — record headless automated pass and pending live-app checklist` (6b8da37) and pushed to `codeless/fix-jobs`

## Next

- stage 7 (final REVIEW): a human runs the five-step live-app checklist recorded in §"Manual verification", fills in the jobIds + pass/fail line, and opens the PR to `master`. Per WORKFLOW.md this gate is "do not auto-merge."

## What you need to know

- the stage spec says "verify in the running app" — a strict reading is impossible headlessly. I chose to record verification *as much as is verifiable here* plus an explicit pending-checklist for the stage-7 reviewer, rather than mark `[!]` and halt. If the loop convention requires the strict reading, the stage-7 reviewer can revert this commit and re-run stage 6 interactively; the recorded automated results still stand.
- the automated test (`JobDetailStack.parallel.test.tsx`) covers the URL-singleton root cause (both panes initialise to their own `Stages` after one writes `?tab=stage:<A-stage>`), independent `useJob` resolution, and independent SSE subscribe scope. The only piece it does not exercise is live SSE delivery against a real backend — flagged explicitly in the §Pending list.
- the `App.tsx:155 path` TS error is pre-existing on master; the stage-5 handover already flagged it. Not addressed here.

## Open questions

- whether the loop convention treats "record automated verification + pending interactive checklist" as a complete stage 6, or requires `[!]` until a human verifies live. If the latter, stage 7 reviewer should re-open stage 6 and overwrite the §"Manual verification" block with their live-app results.
