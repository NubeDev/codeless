## Done

- Created `plugins/notes/ui/` plugin subtree: `package.json`, `tsconfig.json`, `rsbuild.config.ts` (imports `@codeless/plugin-ui-sdk/rsbuild-shared` as the shared-singleton source of truth), `eslint.config.js` (re-exports the R6 flat-config), and `src/AssistantPanel.tsx` (calls `rpc.call("tools_call", { tool: "notes.list_recent", args: { limit } })` and renders the recent-notes list).
- Added `ui/codeless-ui/src/lib/plugin-host/plugin_ui_e2e.test.tsx` with the three stage-12 tests:
- `host_loads_plugin_remote_and_mounts_assistant_panel` — installs the host with a `notes` plugin row, fakes the MF runtime to return the real `AssistantPanel` from `plugins/notes/ui/src/AssistantPanel.tsx`, asserts `<PluginSlot id="assistant-panel"/>` mounts it and the recent-notes `<ul>` renders with the canned RPC payload.
- `mismatched_react_fails_loudly` — MF runtime rejects `loadRemote` with a "React expected ^19, plugin pinned ^18" message; the SDK's `PluginErrorBoundary` renders the structured error card (`[data-codeless-plugin-error]`) inside the slot, host stays alive.
- `r6_eslint_rejects_forbidden_imports` — drives a lightweight flat-config interpreter over `@codeless/plugin-ui-sdk/eslint-config` against a fixture source string that imports `@tauri-apps/api/core` + `react` + calls `fetch()` / `window.fetch()` / `new XMLHttpRequest()`; asserts every R6 message fires and the allowed-only control source lints clean.
- `pnpm -C ui/codeless-ui test` → 25 files / 126 tests pass. `pnpm -C ui/codeless-ui typecheck` clean. `pnpm -C ui/codeless-ui/packages/plugin-ui-sdk test` → 6 files / 56 pass.
- Committed as `aca4547` on `codeless/plugin-substrate-runtimes` (not pushed; no `mani.yaml` present in the worktree to drive a push).

## Next

- Stage 13 (codeless-server manifest parser: `[[runtimes]]` kinds + `[runtimes.capabilities]` + `[runtimes.policy]` + two-phase scan + `plugin_substrate_e2e::process_runtime_declared_today_loads_failed_with_structured_reason`). Gated on REVIEW M-UI per `WORKFLOW.md`.

## What you need to know

- `AssistantPanel.tsx` carries a top-of-file `// @ts-nocheck` with a comment explaining why: the plugin is not (yet) a pnpm-workspace member, so the host's `pnpm typecheck` would otherwise fail to resolve `react`/`react/jsx-runtime` when it follows the test's relative import. The plugin's own `tsc -p plugins/notes/ui/tsconfig.json` does the real type-checking once `pnpm install` has run in the plugin directory.
- The R6 e2e test does NOT pull `eslint` into the host's test runtime (the SDK + host both stay eslint-free, by design — see `eslint-config.test.ts`'s preamble). It interprets the R6 flat-config directly: `no-restricted-imports.patterns[].group` via `glob → regex` (treating `*` as "any chars" to match ESLint's `ignore`-lib semantics on `@tauri-apps/*` matching `@tauri-apps/api/core`), plus literal `selector` recognition for the two `no-restricted-syntax` rules and `name` lookup for the `no-restricted-globals` rule.
- `rsbuild.config.ts` declares `pluginModuleFederation({ name: "notes", filename: "mf-manifest.json", exposes: { "./AssistantPanel": "./src/AssistantPanel.tsx" }, shared: sharedSingletons })` so the host's `mf-manifest.json` URL pattern (`/plugins/notes/ui/mf-manifest.json`, stage 11) and `remote_name = "notes"` (stage 10) line up without any per-plugin overrides.
- The plugin's `package.json` declares `@codeless/plugin-ui-sdk: "workspace:*"` + an `eslint ^9` devDep; neither is currently installed (no `pnpm-workspace.yaml` entry yet). Stage-13/14 work that needs to actually `pnpm install` or `rsbuild build` the plugin will need to add `../../plugins/*/ui` to `ui/codeless-ui/pnpm-workspace.yaml` (or hoist via a workspace root).

## Open questions

- Whether the next stage should promote `plugins/notes/ui` to a pnpm-workspace member so the real `rsbuild build` + `eslint` lint runs on it in CI, or keep the plugin out-of-tree from pnpm and wire the build through a separate per-plugin install step.
