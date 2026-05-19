## Done

- Added `set_assistant_thread_mode` to the `RpcMethodMap` in `ui/codeless-ui/src/lib/rpc/methods.ts` with `SetAssistantThreadModeArgs` / `SetAssistantThreadModeResult` imports.
- Created `ui/codeless-ui/src/modules/assistant/ThreadModeDropdown.tsx`: a shadcn `Select` over the three `AssistantThreadMode` variants. Displayed value derives from the `AssistantThread` row's server-side `mode` (defaulting to `read-only` if absent); on change it calls `set_assistant_thread_mode` then invokes `onChanged` so the caller refreshes the row — no client-owned authoritative state (R4). Server rejects (NotFound, decode error) surface inline.
- Wired the dropdown into `AssistantPage.tsx`'s right rail: a new 16rem `<aside>` mounted only when a thread is selected, hosting the dropdown above the existing `assistant-panel` PluginSlot. `onChanged` re-runs `refresh(selected.id)` so the next render reads the updated thread row from `list_assistant_threads`.
- `pnpm typecheck` is green. Committed as `640ba7f`.

## Next

- (none — final stage of the job)

## What you need to know

- `AssistantThread.mode` is optional on the wire; the dropdown falls back to `read-only` for any pre-migration row that round-trips without the column populated.
- The dropdown does not subscribe to `assistant-thread-touched`; refreshes hang off the existing rail-level `useEventStream` plus the explicit `onChanged` invocation, since the server intentionally does not bump `updated_at` on a mode flip (stage-3 invariant).
- Worktree had no `node_modules`; ran `pnpm install --frozen-lockfile` once to get `tsc` on PATH for the typecheck.
- No `mani` binary is reachable from this worktree; used `git commit` directly per the standard fallback.

## Open questions

- (none)
