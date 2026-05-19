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

## Open questions (resolved in stage 1)

1. **Is `worktree_root_effective` already in scope at the point of
   the new `add_root` call in `serve.rs`?** Yes — and the
   `add_root` call has already landed on this branch.
   `worktree_root_effective` is bound at
   [`crates/codeless-cli/src/serve.rs:368`](../../../crates/codeless-cli/src/serve.rs#L368)
   (via `effective_worktree_root(&args)`); the matching
   `create_dir_all` + `host_fs.add_root(wt)` block is already
   present at lines 413–422 of the same file, sitting between the
   `attached_workspaces` rehydration loop (399–412) and the
   `runtime.with_fs(Arc::new(host_fs))` wiring at line 429. *Why:*
   the CLI half of the fix is in; the recon only confirms it is
   not a regression target for stage 2 (it stays as-is, but the
   regression test still covers it).
2. **Where does the integration test live?** The chat-side reject
   path is
   [`crates/codeless-runtime/src/rpc/chat.rs`](../../../crates/codeless-runtime/src/rpc/chat.rs)
   (`"agent_chat cwd is outside the configured fs roots"` at
   line 76). The existing `HostFs` unit tests live in-file at
   [`crates/codeless-adapters-host/src/fs.rs`](../../../crates/codeless-adapters-host/src/fs.rs)
   from line 377 onward (`add_root_makes_outside_path_resolvable`
   at 487, `add_root_canonicalises_so_duplicates_collapse` at 544
   — `tempfile::tempdir` based, no DB). The `HostFs` regression
   test belongs there. The boot-time end-to-end test for
   `codeless-tauri-desktop::boot()` belongs in
   [`crates/codeless-tauri-desktop/src/boot.rs`](../../../crates/codeless-tauri-desktop/src/boot.rs)'s
   `#[cfg(test)] mod tests` (line 255+) which already runs
   workspace-slug tests against `PathBuf`; that module gains the
   "worktree base is in HostFs roots after boot" assertion. The
   `codeless-cli serve` path has no per-test harness today
   (`serve.rs` is one long function with no `#[cfg(test)]`
   module); the `HostFs` unit test plus the desktop-boot test
   together pin both halves without inventing a new harness.
   *Why:* match the SCOPE constraint "do not invent a new
   harness".
3. **Does `HostFs::add_root` already handle the case where the
   directory does not yet exist?** No — `add_root` calls
   `canonicalise_root` at
   [`crates/codeless-adapters-host/src/fs.rs:87`](../../../crates/codeless-adapters-host/src/fs.rs#L87),
   which `std::fs::canonicalize`s the path and returns
   `FsError::BadRoot` if it doesn't resolve or isn't a directory
   (369–375). The CLI side already preserves the ordering
   (`std::fs::create_dir_all(wt).ok()` at line 417 immediately
   before `host_fs.add_root(wt)` at 418). For
   `codeless-tauri-desktop`, `boot.rs` already runs
   `std::fs::create_dir_all(&paths.worktree_base).ok()` at line
   129 — long before the new `add_root` call would land between
   lines 160 and 163. *Why:* the precondition is met in both
   hosts without re-ordering anything in stage 2.

## Stage-1 reproduction notes

- The reject string fires from
  [`crates/codeless-runtime/src/rpc/chat.rs:76`](../../../crates/codeless-runtime/src/rpc/chat.rs#L76):
  `RpcError::InvalidArgument("agent_chat cwd is outside the
  configured fs roots: {p}")`. The chat-panel client passes the
  job's worktree path as `args.cwd`; `HostFs::is_path_allowed`
  rejects it unless the worktree root (or an ancestor) is in the
  registered allowed-roots set.
- **`codeless-cli serve` — already patched on this branch.** The
  worktree-root `add_root` block lives at
  [`crates/codeless-cli/src/serve.rs:413-422`](../../../crates/codeless-cli/src/serve.rs#L413-L422),
  with `worktree_root_effective` bound at line 368. Stage 2 has
  nothing to edit here, but the regression test must still cover
  the contract so a future refactor cannot silently drop it.
- **`codeless-tauri-desktop::boot()` — NOT patched.** No
  `host_fs.add_root(&paths.worktree_base)` call exists between
  the rehydration loop (ending
  [`crates/codeless-tauri-desktop/src/boot.rs:160`](../../../crates/codeless-tauri-desktop/src/boot.rs#L160))
  and the `runtime.with_fs(Arc::new(host_fs))` wiring at
  [line 163](../../../crates/codeless-tauri-desktop/src/boot.rs#L163).
  `paths.worktree_base` is in scope from line 134 and
  `create_dir_all`'d at line 129; the insertion point is between
  lines 160 and 162 (after the rehydration `match`, before the
  fluent chain `let runtime = runtime.with_fs(...)...`). Surface
  failures through `BootError::FsRoot(format!("{}: {e}",
  paths.worktree_base.display()))` to match the surrounding
  style at line 148.
- **Reproduction not run live.** Stage 1 runs in an isolated
  worktree without a booted server; the static evidence above
  (chat.rs reject site + boot.rs missing registration) is the
  reproduction. A live smoke (boot desktop shell, open chat
  panel) is the stage-4 deliverable per SCOPE §Deliverables #4.
- **Doc gap to address in stage 4.** SCOPE.md §References points
  at `BROWSER-LAUNCHER.md` §"Known issues — Worktree root is not
  in the fs jail", but that section does **not** exist in
  [`crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md`](../../../crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md)
  on this branch (grep returns zero hits in that file). Stage 4
  must either add the section (and immediately flip it to
  `Status: fixed in <sha>` with the actual line ranges), or
  delete the SCOPE.md cross-reference. The deliverable in
  SCOPE.md #5 assumes the section exists; the section does not.

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
