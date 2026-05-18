## Done

- Added `ui/codeless-ui/src/lib/shell/path-picker.ts` defining `PathPicker` interface plus `browserPathPicker` injector (uses `window.showDirectoryPicker()` when available, falls back to a typed-input prompt otherwise; returns null on cancel; never canonicalises — caller must hand result to `validate_workspace_path`).
- Added `ui/codeless-ui/src/shells/desktop/path-picker.ts` (`tauriPathPicker`) backed by `@tauri-apps/plugin-dialog`'s `open({ directory: true })`.
- Wired the new capability through `ShellProvider`: added `pathPicker` to ShellValue / ProviderProps, default `browserPathPicker`, and `usePathPicker()` hook; re-exported from `shell/index.ts`.
- Desktop shell `main.tsx` now passes `pathPicker={tauriPathPicker}`; browser shell uses the default automatically.
- Added `@tauri-apps/plugin-dialog ~2.4.0` to `ui/codeless-ui/package.json` (resolved to 2.4.2 in lockfile via pnpm install).
- `pnpm typecheck` and `pnpm test` (18 files, 89 tests) both green.
- Committed as `a1689f2` with message starting with the stage title.

## Next

- Stage 4 (per WORKSPACE-ATTACH.md M4 phasing): start the Settings → Workspaces tab UI that consumes the four new RpcClient methods plus the `usePathPicker()` hook.

## What you need to know

- Mani is not available inside this isolated worktree (mani.yaml lives in the outer codeless-workspace), so this commit used plain `git commit`. If the workspace rule requires mani for push, the next session will need to run it from the outer workspace, not inside the worktree.
- Rust-side Tauri plugin-dialog registration is NOT wired in `crates/codeless-tauri-desktop` — the JS import compiles, but `tauriPathPicker.pickDirectory` will fail at runtime in desktop until the Rust plugin is added (`tauri-plugin-dialog` crate + `.plugin(tauri_plugin_dialog::init())` in main.rs + capabilities entry). The doc's stage scope was UI-only, so this is intentionally deferred.
- `browserPathPicker` keeps the contract weak — even when `showDirectoryPicker()` succeeds it still prompts for the absolute server-side path because the File System Access API never exposes the OS path. UI callers must always round-trip through `validate_workspace_path`.

## Open questions

- Should the Rust-side Tauri plugin-dialog registration land in M3c as well, or is it scoped to a later stage? Treated here as deferred per the stage description ("the Tauri-desktop-shell injector").
