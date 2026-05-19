# Scope — worktree-fs-jail

Fix the "agent_chat cwd is outside the configured fs roots" rejection
that fires for every per-job chat panel. The bug, root cause, and
exact fix are documented at
[`crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md`](../../../crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md)
§"Known issues — Worktree root is not in the fs jail" and that section
is authoritative; this brief is the trimmed per-job scope.

## Goal

After this job, opening the chat panel for any running job — whether
the host is `codeless-cli serve` or `codeless-tauri-desktop` — does
not return the `invalid_argument: agent_chat cwd is outside the
configured fs roots` error. The worktree root is registered with
`HostFs` at boot in both hosts; a regression test pins the
behaviour.

## In scope

- Two-line edit in
  [`crates/codeless-cli/src/serve.rs`](../../../crates/codeless-cli/src/serve.rs)
  (~line 413, after the `attached_workspaces` rehydration block) —
  call `host_fs.add_root(&worktree_root_effective)` once the
  directory has been created on disk.
- Two-line edit in
  [`crates/codeless-tauri-desktop/src/boot.rs`](../../../crates/codeless-tauri-desktop/src/boot.rs)
  inside `boot()`, after the `attached_workspaces` rehydration loop
  and before `runtime.with_fs(Arc::new(host_fs))` — call
  `host_fs.add_root(&paths.worktree_base)`. Surface failures via
  `BootError::FsRoot` to match the surrounding style.
- A unit test on `HostFs` that proves `add_root` of a worktree path
  makes `is_path_allowed` return `true` for descendants and `false`
  for siblings outside the registered roots.
- An integration test that boots a runtime (in-memory SQLite +
  tmp-dir worktree root), creates a worktree path, calls
  `agent_chat` with `cwd` set to that path, and asserts the call
  does not return the `fs roots` `InvalidArgument`. The harness
  used here should be the existing one in `codeless-cli` or
  `codeless-tauri-desktop` — do not invent a new one.
- Status flip in `BROWSER-LAUNCHER.md` §"Known issues — Worktree
  root is not in the fs jail" from `Status: open` to `Status: fixed
  in <commit-sha>` with the actual line ranges that landed.

## Out of scope

- Refactoring `HostFs`, the allowed-roots list, or the `is_path_allowed`
  predicate. The fix is two `add_root` calls; do not generalise.
- Touching the `attached_workspaces` rehydration logic. The
  worktree-root registration sits next to it, but is a separate
  concern.
- Exposing the worktree root to the user as a configurable fs root.
  The doc is explicit: the worktree is an internal directory; the
  runtime owns it and registers it.
- Any change to `WorktreeManager`, `agent_chat`'s validation logic,
  or the chat panel UI. The bug is at boot, the fix is at boot.
- Fixing the wider `--fs-root` UX, the per-workspace data-dir
  rollback, or any other BROWSER-LAUNCHER milestone. Those are
  separate jobs.
- Adding the worktree root to `ServerInfo` or any RPC payload. It
  stays an internal jail entry; clients have no business knowing.

## Constraints

- **R1** — host-only crates only. The fix lives in
  `codeless-cli` and `codeless-tauri-desktop` (and possibly a unit
  test in `codeless-adapters-host`). No mobile-safe crate is
  touched.
- **R4** — no behavioural change to what SQLite holds. The fix is
  an in-memory `HostFs` registration that already runs for
  `attached_workspaces` rows; the worktree-root entry is the same
  kind of registration with no persistence implication.
- **R5** — bearer-token authorisation unchanged. The fix sits
  below the RPC layer.
- **MSRV / lint gates**: `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --check` all green before each stage commits.

## Deliverables (what "done" looks like)

1. `codeless/worktree-fs-jail` branch with one commit per stage,
   pushed via mani.
2. `cargo test --workspace` green; the new unit and integration
   tests pass.
3. `cargo clippy --workspace --all-targets -- -D warnings` green.
4. Manual smoke (record in the stage-4 handover): start the server,
   submit a small job, open its chat panel, send one message, see no
   `fs roots` rejection in the server log or the panel.
5. `BROWSER-LAUNCHER.md` §"Known issues" entry shows `Status: fixed
   in <commit-sha>` with the actual line ranges of the edits.

## Open questions (resolve in stage 1, before any code)

1. **Is `worktree_root_effective` already in scope at the point of
   the new `add_root` call in `serve.rs`?** The BROWSER-LAUNCHER doc
   says yes (line ~413, with the `create_dir_all` for it just
   above). Confirm against the current branch and record the exact
   line number in the handover.
2. **Where does the integration test live?** Pick the existing
   harness in `codeless-cli` or `codeless-tauri-desktop` that
   already boots a runtime against tmp dirs; do not create a new
   harness file. Record the chosen path in the handover.
3. **Does `HostFs::add_root` already handle the case where the
   directory does not yet exist?** Read the function before the
   stage 2 edit; if it requires the path to exist, ensure the
   `create_dir_all` ordering is preserved. (The doc claims the
   `create_dir_all` line "just above" already guarantees this.)

Record the chosen answer plus a one-line *why* under each in this
file during stage 1; no production code in stage 1.

## References

- Bug doc (authoritative):
  [`crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md`](../../../crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md)
  §"Known issues — Worktree root is not in the fs jail"
- Reject path:
  [`crates/codeless-runtime/src/rpc/chat.rs`](../../../crates/codeless-runtime/src/rpc/chat.rs)
- HostFs predicate:
  [`crates/codeless-adapters-host/src/fs.rs`](../../../crates/codeless-adapters-host/src/fs.rs)
- CLI boot:
  [`crates/codeless-cli/src/serve.rs`](../../../crates/codeless-cli/src/serve.rs)
- Desktop boot:
  [`crates/codeless-tauri-desktop/src/boot.rs`](../../../crates/codeless-tauri-desktop/src/boot.rs)
- Agent rules: [`CLAUDE.md`](../../../CLAUDE.md)
