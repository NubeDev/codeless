## Done

- Surfaced `reset_job` via the typed RpcClient boundary: regenerated `ui/codeless-ui/src/lib/rpc/generated/wire.ts` (registered `ResetJobArgs` in `crates/codeless-rpc/examples/wire_ts.rs`), added `reset_job` to `RpcMethodMap` and a hand-mirrored `ResetJobArgs` in `methods.ts`, taught `MockRpcClient` to honour the `Queued|Failed|Stopped → Draft` edge and emit `job-reset`.
- Added the Reset button to `JobPage` header, gated on `canReset = queued | failed | stopped`. Exported `PageHeader` for unit tests.
- New `ui/codeless-ui/src/modules/jobs/JobPage.reset.test.tsx` — 8 cases covering visibility across all `JobStatus` values; load-bearing case is the hide-in-`running` assertion. All 8 pass.
- Committed as `stage 6: UI affordance for reset_job` on `codeless/runtime-driver-recovery`.

## Next

- Stage 7: workspace liveness audit. Confirm `workspace_liveness.rs` never writes to the `jobs` table; add a fixture test asserting that invariant. Stage 8 is the final REVIEW gate (R1/R5 grep, regression test green, full check suite).

## What you need to know

- R2 respected: `JobPage.tsx` calls `rpc.call("reset_job", …)` only; no new `@tauri-apps/*` imports.
- R1 untouched: no process-spawn calls moved.
- `pnpm typecheck` clean, `pnpm test` 9/9 green, `cargo fmt --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Pre-existing test failure on HEAD before this stage: `codeless-runtime --test rpc_in_process job_filtered_subscription_drops_unrelated_events` (in-repo-mode conflict on the second `submit_job` in the fixture). Confirmed unrelated by stashing this stage's diff and re-running. Worth flagging to a future stage but out-of-scope here.
- Push step was skipped — `bin/mani` isn't reachable from inside the worktree at `/home/user/.codeless/worktrees/job-…`. Previous stage commits in the log also appear to be plain `git commit`, so the runtime is expected to push.

## Open questions

- Should the Reset button live in a confirmation flow (like Delete's two-step) given it best-effort reaps the worktree? Current implementation is single-click to mirror Stop/Resume; revisit if user-test feedback says otherwise.
