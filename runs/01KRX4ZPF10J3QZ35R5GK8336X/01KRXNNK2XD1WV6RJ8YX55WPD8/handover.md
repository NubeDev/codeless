## Done

- Added `list_plugins` RPC seam to `wire.ts` + `methods.ts` (forward-declared `PluginListEntry` / `PluginUiContribution` / `ListPluginsResult`; Rust side lands with stage 13's `[[runtimes]]` + `[contributes.ui]` manifest parsing).
- `MockRpcClient` now answers `list_plugins` with `{ plugins: [] }`.
- New `ui/codeless-ui/src/lib/plugin-host/` with `installPluginUiHost(rpc, options?)`: idempotent install, swallows `RpcError` from `list_plugins` (server may not implement it yet), installs an `MfRuntime` on the SDK (placeholder by default; shell can inject a real adapter), and registers each enabled plugin's `[contributes.ui.exposes]` rows through `@codeless/plugin-ui-sdk`'s registration table.
- Re-exports `PluginSlot` from `@/lib/plugin-host` so host slot sites don't have to learn the SDK package name.
- Boot hook in `App.tsx` calls `installPluginUiHost(rpc)` once via `useEffect`.
- First slot site: `<PluginSlot id="assistant-panel" threadId={selected.id} fallback={null} />` mounted to the right of the Assistant chat pane. With zero contributors, the fallback renders and the host UI is byte-identical to pre-stage.
- Added `@codeless/plugin-ui-sdk` as a workspace dep on the host (`pnpm add -w "@codeless/plugin-ui-sdk@workspace:*"`).
- Vitest coverage in `installPluginUiHost.test.ts`: empty list path, `list_plugins`-unavailable path, contribution registration, idempotency, placeholder-runtime structured rejection.
- `pnpm typecheck` clean; `pnpm vitest run` 123/123 green; SDK `pnpm test` 56/56 green.
- Committed as `047293a` on `codeless/plugin-substrate-runtimes`.

## Next

- Stage 13: server-side plugin manifest parsing for `[[runtimes]]` + `[contributes.ui]`, plus the Rust `list_plugins` RPC handler that backs `rpc.call("list_plugins", {})`. Wire shapes are already pinned in `ui/codeless-ui/src/lib/rpc/wire.ts` (forward-declared block at the bottom); when the Rust side emits these via specta, delete the forward declarations and re-export from `./generated/wire`.

## What you need to know

- The host installs the SDK's *placeholder* `MfRuntime` by default. It accepts `registerRemote` so the registration call site stays symmetric but rejects every `loadRemote` with a structured error (surfaced through `PluginSlot`'s per-contributor error boundary, never thrown synchronously). The browser / Tauri shell entry will eventually push a real adapter (`@module-federation/enhanced/runtime`) via `installPluginUiHost(rpc, { mfRuntime })`. Adding that dep is left for the stage that actually needs to load a plugin chunk end-to-end.
- Only one slot site (`assistant-panel`) is mounted in this stage. The other four slots in the v0.1 vocabulary (`tool-result:<tool_id>`, `persona-picker:<persona_id>`, `settings-page:<plugin_id>`, `composer-attachment-action:<plugin_id>`) need broader UI work to identify their canonical render sites — picked up in later stages alongside the notes-plugin `AssistantPanel` remote (which is what proves the wiring end-to-end).
- `installPluginUiHost` is idempotent — React StrictMode's double-mount and HMR re-runs both resolve to the cached descriptor without re-fetching `list_plugins`. Tests reset it through `resetPluginUiHostForTesting()`.
- The `RpcError` catch path is intentional: until Rust ships `list_plugins`, every host boot logs one `console.debug` and the registry stays empty. Slots fall back gracefully.

## Open questions

- (none)
