## Done

- added `crates/codeless-runtime/src/diff_verify.rs` (path extraction from handover `Done` bullets, verification against a diff-paths list, fail-reason rendering) with 12 unit tests
- added `crates/codeless-adapters-host/src/git_changed.rs` (`changed_files(worktree, base)` shells out to `git diff --name-only base...HEAD` + `git status --porcelain`; falls back to `git log` when base is missing) with 3 git-backed unit tests; R1 honoured — process spawning stays in adapters-host
- wired diff-verify pre-check into `crates/codeless-runtime/src/template_runner.rs`: `prev_stage_id` tracked across the stage loop, `run_diff_verify_precheck` runs at the top of every REVIEW iteration before the inner adapter, miss publishes `StageCompleted { Failed }` and returns `RunnerOutcome::Failed` with no model invoked
- exposed `changed_files` + `GitChangedError` from `codeless-adapters-host::lib.rs`; registered new `diff_verify` module in `codeless-runtime::lib.rs`
- added 4 integration-style tests for `run_diff_verify_precheck` (pass / fail / absent-handover / no-paths skips) using real on-disk git repos
- committed as `stage 2: diff-verify pre-check ...` on `codeless/session-mutable-scope`

## Next

- stage 3: stand up the predicate-runner crate (xtask-shaped, host-only per R1) seeded with 3-5 hand-written probes (e.g. no `tokio::process` outside `codeless-adapters-host`, no-emoji-in-source, handover-four-sections-present); decisions file Q4 pins the exact crate name and Cargo member entry — read it before naming anything

## What you need to know

- `prev_stage_id` is updated only on the Passed exit path; a Failed prior stage short-circuits the loop and never reaches the assignment, so the next REVIEW (which would not run anyway) cannot read a stale id
- diff-verify is intentionally lenient about prior-handover absence: missing / unparseable / no-path-tokens → `PreCheckOutcome::Skipped` (info log, no fail). The contract is "verify what was claimed," not "demand a claim exists." Mock-runner mode and "REVIEW as the first stage" both depend on this
- the path-extractor's slash rule requires at least one lowercase ASCII letter so `PASS/FAIL` (which appears literally in the REVIEW prompt and will show up in `Done` bullets that quote it) does not get treated as a path. Every real codebase path satisfies the lowercase requirement
- the suffix-match rule is one-directional: a bare-leaf claim matches a deep diff entry, but a deep claim does not match a bare-leaf diff entry — that prevents `runtime/src/template_runner.rs` from being satisfied by a stray `template_runner.rs` in some unrelated diff
- the git call is wrapped in `tokio::task::spawn_blocking`; a JoinError collapses to `Skipped` rather than `Fail` (deliberate: better to miss a verification than to fail a stage on a thread-pool bug)
- base ref is hardcoded to `main`; `changed_files` falls back to `git log` when the ref does not resolve, so a worktree whose `main` was pruned still works. If a non-`main` default branch appears later, plumb it through `RunnerContext` rather than guessing
- pre-existing test failure on `rpc_in_process::job_filtered_subscription_drops_unrelated_events` reproduces on the parent commit (`13f7480`); unrelated to this stage
- per CLAUDE.md commit-and-push rule the work must be pushed via `mani` from the workspace root; this headless session committed but did not push — the outer JOB-LOOP harness handles the push

## Open questions

- (none)
