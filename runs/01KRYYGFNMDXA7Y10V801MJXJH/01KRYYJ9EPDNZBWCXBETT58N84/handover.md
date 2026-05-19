## Done

- Read `.codeless/jobs/worktree-fs-jail/SCOPE.md` and the referenced `crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md`; confirmed the SCOPE-referenced §"Known issues — Worktree root is not in the fs jail" section does **not** exist in BROWSER-LAUNCHER.md on this branch (grep returns zero hits).
- Located the reject path: `crates/codeless-runtime/src/rpc/chat.rs:76` — `RpcError::InvalidArgument("agent_chat cwd is outside the configured fs roots: …")`.
- Audited both host boot paths and the `HostFs::add_root` precondition; recorded answers to the three SCOPE.md stage-1 open questions plus an explicit reproduction-notes block inside SCOPE.md.
- Committed the updated SCOPE.md as `stage 1: confirm reproduction and lock the fix shape` (commit `e636a69` on branch `codeless/worktree-fs-jail`). No production code touched.

## Next

- **Stage 2** does the actual edit in `crates/codeless-tauri-desktop/src/boot.rs`. The insertion point sits between line 160 (closing brace of the `match attached_workspaces::list_canonical_roots(...)` rehydration) and the `let runtime = runtime.with_fs(Arc::new(host_fs))` chain that currently starts at line 162. Add:
- `crates/codeless-cli/src/serve.rs` already has the equivalent `add_root` at lines 413–422 (`worktree_root_effective` bound at line 368). Do **not** re-edit it in stage 2; just keep the regression coverage in mind.

## What you need to know

- The reject string text the chat panel surfaces is produced exactly once in the workspace, at `crates/codeless-runtime/src/rpc/chat.rs:76`.
- `HostFs::add_root` canonicalises and rejects non-existent / non-directory paths (`FsError::BadRoot`). Both hosts already `create_dir_all` the worktree base before any `HostFs` work — serve.rs at line 417 (inside the `add_root` block) and boot.rs at line 129 (top of `boot()`). Preserve that ordering in stage 2.
- Test harness picks (per the SCOPE constraint "do not invent a new harness"): the unit-test belongs in `crates/codeless-adapters-host/src/fs.rs` `#[cfg(test)] mod tests` (already houses `add_root_*` tests with `tempfile::tempdir`, no DB); the boot-time regression belongs in `crates/codeless-tauri-desktop/src/boot.rs` `#[cfg(test)] mod tests` (line 255+, already tests `workspace_slug`). `codeless-cli/src/serve.rs` has no in-crate test module today; the two locations above cover both halves of the contract.
- The CLI half of the fix is **already on this branch** — half-fixed state. Stage 4's "flip BROWSER-LAUNCHER.md `Status: open` → `Status: fixed in <sha>`" deliverable lands in a section that doesn't yet exist; stage 4 must either author the section or drop the SCOPE.md cross-reference. Flagged inside SCOPE.md.
- Branch `codeless/worktree-fs-jail`; last commit `e636a69`. Workspace `CLAUDE.md` mandates `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` before every commit — stage 1 touched only markdown so they were skipped, but stage 2 must run them.

## Open questions

- (none) — the three open questions in SCOPE.md are resolved in-file.
