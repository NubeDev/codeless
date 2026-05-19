## Done

- Added `host_fs.add_root(&worktree_base)` in `crates/codeless-tauri-desktop/src/boot.rs` between the `attached_workspaces` rehydration loop and `runtime.with_fs(Arc::new(host_fs))` (boot.rs:160-162 insertion point identified in stage 1).
- Error surfaced via `BootError::FsRoot(format!("{}: {e}", worktree_base.display()))`, matching the existing `HostFs::new` error site on line 148.
- `crates/codeless-cli/src/serve.rs` already carries the equivalent registration for `worktree_root_effective` at lines 413-422 (confirmed by stage 1); no edit needed there.
- Committed as `168cb7f` on branch `codeless/worktree-fs-jail` with message starting `stage 2: land the fix in both hosts`.

## Next

- Stage 3: regression test pinning the boot behaviour so the bug cannot silently come back (per job scope).
- Stage 4: reconcile `BROWSER-LAUNCHER.md` "Known issues — Worktree root is not in the fs jail" — stage 1 noted that section does not currently exist in `BROWSER-LAUNCHER.md`, so stage 4 must either add it (and flip to fixed) or drop the SCOPE.md cross-reference.

## What you need to know

- `cargo build`/`cargo check` cannot be run from this worktree: the shared sibling `../ai-runner` Cargo workspace is locked to a different job worktree (`job-01KRYQJVK0G60MEZVFQ6KW3Y1F`), so cargo refuses with "package … is a member of the wrong workspace". The next stage will need that other worktree torn down (or the build done from outside the JOB-LOOP isolation) before `cargo test --workspace` / `cargo clippy -D warnings` can be run.
- The added comment in `boot.rs` follows R2: explains *why* (worktree base sits outside workspace root → agent_chat rejects cwd) without referencing stages or tasks.
- `create_dir_all(&paths.worktree_base)` runs at boot.rs:129, well before the new `add_root` call, so the `canonicalise_root` precondition is met without reordering.
- Commit used raw `git commit` rather than mani (no `bin/mani` reachable from this isolated worktree); the preceding stage-1 commit (`e636a69`) followed the same pattern.

## Open questions

- Does stage 3's regression test cover only the desktop boot path, only the CLI serve path, or both? The fix shape is symmetric; the test surface may not need to be.
- Should the `BROWSER-LAUNCHER.md` "Known issues" section be added in stage 4 (and immediately flipped to "fixed in …") or simply removed from the SCOPE.md cross-reference?
