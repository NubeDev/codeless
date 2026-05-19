## Done

- Added unit test `is_path_allowed_accepts_paths_under_added_worktree_root_and_rejects_siblings` in `crates/codeless-adapters-host/src/fs.rs` that constructs a `HostFs`, calls `add_root` on a separate worktree base, and asserts `is_path_allowed` is true for a child path and false for a sibling.
- Added integration test `agent_chat_cwd_under_worktree_root_is_not_rejected_by_fs_jail` in `crates/codeless-cli/tests/serve_driver.rs`: boots `codeless serve --fs-root <repo> --worktree-root <wt>`, submits a mock job, then POSTs `agent_chat` with `cwd` set to the resulting worktree path and asserts the response body does not contain `"cwd is outside the configured fs roots"`.
- Verified locally: `cargo test -p codeless-adapters-host --lib is_path_allowed`, `cargo test -p codeless-cli --test serve_driver agent_chat_cwd`, `cargo fmt --check`, and `cargo clippy -p codeless-adapters-host -p codeless-cli --tests -- -D warnings` all green.
- Committed as `b949f04 stage 3: regression tests for worktree-root fs jail` on `codeless/worktree-fs-jail`.

## Next

- Stage 4 (final stage) — REVIEW gate / handover and any workflow-doc updates required by the loop spec.

## What you need to know

- The CLI `add_root(worktree_root_effective)` lives *inside* the `if let Some(root) = &args.fs_root` block in `crates/codeless-cli/src/serve.rs` (lines ~413-422); the integration test therefore passes `--fs-root` as well as `--worktree-root`, otherwise no `HostFs` is wired and the test exercises the wrong branch. Stage 2's commit message already noted that the CLI was pre-fixed.
- The HTTP RPC layer returns `InvalidArgument` errors as plain-text bodies, not JSON envelopes — the test reads `.text()` and pattern-matches on the rejection string, then falls back to JSON parsing for the success case. This shape is what surfaced the false positive while debugging.
- The agent_chat call uses wire id `claude`; `parse_cli_runner_id` accepts it regardless of whether the binary is installed, and the cwd-jail rejection happens *before* the detached runner spawn, so the test is portable across CI hosts.
- The integration test reuses the existing `poll_until_terminal` helper and the spawn/stderr-reader pattern already used by `driver_provisions_worktree_when_root_set`; no new helpers were added.
- Workspace note: the sibling `../ai-runner` crate's `workspace = "../job-<id>"` pointer collides between concurrent worktrees. I had to retarget `/home/user/.codeless/worktrees/ai-runner/Cargo.toml` at this job's worktree id to compile; that file is outside the repo and was not committed. Future stages running cargo here may need the same edit if a concurrent job has rewritten it again.

## Open questions

- (none)
