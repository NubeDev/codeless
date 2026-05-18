## Done

- Added `useWorkspacesStore` zustand singleton (`ui/codeless-ui/src/modules/workspaces/store.ts`) mirroring the server `attached_workspaces` set with reducer-style `setWorkspaces` / `applyAttached` / `applyDetached` / `setActive` / `setStatus` actions. Detach of the active workspace falls back to the most-recently-attached survivor per the doc.
- Added `useWorkspacesSync` hook (`useWorkspacesSync.ts`) — hydrates via `list_workspaces` on mount and subscribes to `{scope:"all"}` event stream, dispatching `workspace-attached` / `workspace-detached` payloads through `reconcileFromEvent` into the store.
- Added `EmptyWorkspacesState` component with the doc's blank-state copy and a CTA button (disabled when no callback supplied).
- Extended `MockRpcClient` with the four workspace methods + `seedAttachedWorkspaces` test seam; empty-by-default so the empty-state path is the default in tests.
- Unit tests: `store.test.ts` covers hydrate / upsert / detach-fallback / active preservation / unknown-event ignore; `EmptyWorkspacesState.test.tsx` covers render + click.
- `pnpm typecheck` and `pnpm test --run` both green (20 files, 101 tests). Commit: `42f0728`.

## Next

- Stage 6 (M4b): Settings → Workspaces tab — list table with active dot + open/attach/detach buttons; attach modal driving validate_workspace_path debounced live (~5/s server cap); confirm + add_repo + attach_workspace round-trip. The store + sync hook + empty state are ready for the table to consume via `useWorkspacesStore()` and the page to call `useWorkspacesSync()` once.

## What you need to know

- The Rust event union does NOT yet include `workspace-attached` / `workspace-detached` variants (only `workspace-unhealthy` / `workspace-recovered` from the liveness sweep). The sync hook string-matches on the event tag so the dispatch is forward-compatible; the M4b/M4c modals should also call `useWorkspacesStore.getState().applyAttached(...)` / `.applyDetached(...)` directly after the RPC resolves so the store reconciles immediately without waiting for an event the server isn't emitting.
- `seedAttachedWorkspaces` on `MockRpcClient` is the test seam to pre-populate the roster.
- `pnpm lint` is a no-op script in this repo (`echo 'lint: no eslint configured yet'`); typecheck is the real gate.
- No `@tauri-apps/*` imports added outside the desktop shell; no `.web/.desktop` filename forks.

## Open questions

- (none)
