## Done

- Added `MockRpcClient.seedRunningJobsForWorkspace` seam in `ui/codeless-ui/src/lib/rpc/mock-client.ts` that makes `detach_workspace { on_running_jobs: "refuse" }` throw a `WorkspaceError::RunningJobs`-shaped `RpcError("conflict", …)` payload, and is cleared by `stop` / `leave-running`.
- New `ui/codeless-ui/src/modules/workspaces/DetachWorkspaceDialog.test.tsx` covering both modal shapes: no-running-jobs one-line confirm (refuse → close), and running-jobs (refuse → inline jobs list + radio → stop submit → detach).
- `pnpm test` green: 22 files / 108 tests. `pnpm lint` is a stub but green.
- Committed as `54af09a` on `codeless/workspace-attach-ui` with message starting `stage 7: M4c: …`.

## Next

- Stage 8 (final REVIEW gate per WORKFLOW.md): paste test tail, Playwright report summary, and 3-state Workspaces tab screenshots; flip M3/M4 boxes in `DOCS/WORKSPACE-ATTACH.md` per SCOPE Deliverables item 5.

## What you need to know

- Attach happy-path is already covered in `WorkspacesSection.test.tsx` ("round-trips a typed path…"); SCOPE/stage wording asks for "Playwright/RTL" — no Playwright runner is installed in `ui/codeless-ui` (`grep -i playwright package.json` empty); all UI tests run under Vitest + RTL, which matches every other M4 test in tree.
- `mani` is not available inside the isolated worktree (`/home/user/code/...` paths only); I committed with raw `git` per harness reality. If the review process requires a mani-driven commit, the stage 8 session running outside the worktree should redo it.
- Detach dialog's `parseRunningJobs` regex matches the literal substring `running-jobs` and a `jobs":[...]` array; the new mock seam serialises exactly that shape via `JSON.stringify({ "running-jobs": { jobs } })`, so any future change to the parser must keep that substring contract.

## Open questions

- (none)
