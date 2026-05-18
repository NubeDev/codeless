# Plugin Substrate — Module-federated UI

Status: draft
Owner: ap@nube-io.com
Created: 2026-05-18

Companion to [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md). This doc
fills in item 10 — UI contributions from plugins, served as Module
Federation remotes from the codeless server and mounted by the
single `ui/codeless-ui/` host at declared slots.

If anything below contradicts [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md),
[`../UI-ARCHITECTURE.md`](../UI-ARCHITECTURE.md), or
[`../SCOPE.md`](../SCOPE.md), those win.

## One-line summary

A plugin may ship a Module Federation remote (`remoteEntry.js` +
chunks). The codeless server serves the bundle from
`/plugins/<id>/ui/*` via `ServeDir`; the host page registers the
remote at boot, negotiates shared singletons (React, RpcClient,
zustand, tanstack-query), and mounts the plugin's exposed modules at
declared slots in the existing `ui/codeless-ui/` shell. The author-
facing SDK is `@codeless/plugin-ui-sdk`, an in-tree fork of rubix's
`extension-ui-sdk`.

## Why MF (and not iframes, web components, or a thicker plugin
   surface)

Three real choices:

- **iframes.** Strongest isolation; worst integration. Shared theme,
  shared focus management, drag-and-drop into a chat composer — none
  of it works without postMessage choreography we'd have to design
  per slot. The Assistant action-card surface (PLUGIN-SUBSTRATE item
  7) crosses the chat/plugin boundary on every confirmation; iframes
  would force every plugin to reinvent that wire.
- **Web components.** Better integration than iframes, no module
  resolution story. Codeless's UI is React 19 + zustand +
  tanstack-query + shadcn primitives; a web component cannot reuse
  any of that without bringing its own copy. Bundle size explodes
  per plugin.
- **Module Federation.** React-shared, theme-shared, RpcClient-
  shared. Plugin code runs in the host's React tree as a normal
  component subtree; theming, focus, drag-and-drop, action cards all
  Just Work. Cost paid: a small amount of build-time and runtime
  glue. Cost avoided: per-slot integration code on every plugin.

Pick MF. The rubix workspace has already paid the toolchain-design
cost (see [rubix EXTENSIONS.md § Frontend contract](../../../rubix-workspace/rubix-agent/docs/design/extensions/EXTENSIONS.md));
codeless reuses the contract.

## Slot vocabulary — where plugin UI can mount

A *slot* is a named region of the codeless UI where a plugin remote
can mount one or more modules. The host enumerates the slots; the
plugin manifest declares which slots its modules contribute to. No
free-form "render anywhere" — every mount point is named, so the
host can render fallbacks when a slot has no contributors and so
operators can read `codeless plugin info <id>` and see exactly where
the plugin appears.

**v0.1 slot vocabulary is locked as of 2026-05-18
(plugin-substrate-runtimes stage 1)** to the five rows in the table
below. None was dropped at stage-1 review because each removes a
class of plugin UI we know we want; growing the set requires a host-
side change per § "Adding a new slot is a host-side change" further
down this section.

v0.1 slots:

| Slot id | Where it renders | Cardinality | Typical use |
|---|---|---|---|
| `assistant-panel` | Right-hand drawer of the Assistant page, visible when the active thread's persona belongs to the plugin | 1 module per plugin per thread | Persona-specific side panel (e.g. estimator's takeoff list, notes' recent-notes list) |
| `tool-result:<tool_id>` | Inline in a chat thread, replacing the default JSON-blob renderer for that tool's results | exactly 1 module per tool id; the plugin owning the tool wins | Custom result cards (e.g. `estimate.render_quote` → PDF preview card) |
| `persona-picker:<persona_id>` | Persona-pick row of the Assistant page; replaces the default persona card | exactly 1 module per persona id | Persona icon, blurb, optional preset prompts |
| `settings-page:<plugin_id>` | Plugin-specific subpage of the Settings area (e.g. "Estimating" tab) | exactly 1 module per plugin | Plugin configuration UI (catalog uploader, prompt overrides, …) |
| `composer-attachment-action:<plugin_id>` | A button row inside the chat composer when the active thread's persona belongs to the plugin | unbounded per plugin | One-shot actions tied to the composer (e.g. "Add property photo") |

A slot id of the form `foo:<arg>` is *parameterised*; the host
passes `<arg>` as a prop to the mounted component. The host
*chooses* which contributor to render when cardinality is "exactly
1" — usually by matching `<arg>` against ownership (the plugin
owning the persona / tool / settings page wins; conflicts are a
manifest parse error at load time).

**Adding a new slot is a host-side change.** Plugins cannot invent
slot ids. This is the only way we keep a stable UI contract — every
slot has a host renderer that knows what to do when no plugin claims
it.

## Manifest extension (item 6 addendum)

`plugin.toml` gains a `[contributes.ui]` block:

```toml
[contributes.ui]
entry  = "ui/remoteEntry.js"          # path under the plugin dir
mf_manifest = "ui/mf-manifest.json"   # sits next to remoteEntry.js;
                                      # host fetches THIS as the entry url

[[contributes.ui.exposes]]
name   = "AssistantPanel"
module = "./AssistantPanel"
slot   = "assistant-panel"

[[contributes.ui.exposes]]
name   = "QuoteResultCard"
module = "./QuoteResultCard"
slot   = "tool-result:estimate.render_quote"

[[contributes.ui.exposes]]
name   = "EstimatingPersonaCard"
module = "./EstimatingPersonaCard"
slot   = "persona-picker:estimating"

[[contributes.ui.exposes]]
name   = "EstimatingSettings"
module = "./EstimatingSettings"
slot   = "settings-page:estimating"
```

Strict-validate at load time:

- Every `slot` matches the host's slot vocabulary or its
  parameterised form. Unknown slot id → manifest parse error.
- For "exactly 1" slots, the `<arg>` in the slot id is owned by this
  plugin (the persona/tool/plugin id matches). A contributor for
  `persona-picker:other-plugins-persona` is rejected.
- The `entry` and `mf_manifest` files exist under the plugin dir at
  scan time. Missing → `Failed` with reason.

## Host wiring — what `ui/codeless-ui/` does

At boot, the codeless UI:

1. `rpc.plugins.list()` returns enabled plugins with their
   `contributes.ui` blocks.
2. For each, register an MF remote against the entry url
   `/plugins/<id>/ui/mf-manifest.json` (the rsbuild MF manifest sits
   next to `remoteEntry.js` inside the plugin's `ui/` dir).
3. MF negotiates **shared singletons** — versions pinned both in
   the host's `rsbuild.config.ts` and every plugin's
   `rsbuild.config.ts`:

   ```ts
   shared: {
     react:                { singleton: true, requiredVersion: "^19" },
     "react-dom":          { singleton: true, requiredVersion: "^19" },
     zustand:              { singleton: true, requiredVersion: "^5"  },
     "@tanstack/react-query": { singleton: true, requiredVersion: "^5" },
     "@codeless/plugin-ui-sdk": { singleton: true, requiredVersion: "^0.1" },
     "@codeless/rpc":      { singleton: true, requiredVersion: "^0.1" },
   }
   ```

4. The host renders `<PluginSlot id="assistant-panel" />` (or
   `<PluginSlot id="tool-result:estimate.render_quote" />`) at every
   declared slot site. `PluginSlot` looks up contributors from the
   registry built in step 1, lazy-imports the exposed module, and
   mounts it inside an error boundary.

The plugin remote runs **inside the host's React tree**. Theming,
suspense, focus, tanstack-query cache, and the RpcClient are all
the host's. The plugin's bundle ships only its own components.

## Server wiring — `codeless-server`

Three additions, all small:

- `GET /plugins` — returns `[{ id, version, contributes_ui: bool,
  slots: [...] }]`. Reflects the plugin registry; the UI calls this
  from the existing `RpcClient`, not a one-off fetch.
- `GET /plugins/<id>/ui/*` — `ServeDir` rooted at the plugin dir's
  `ui/` subdirectory. Static files only, no auth on this route (the
  whole UI is behind the bearer token at the load step; the plugin
  bundle is no more sensitive than the host bundle).
- CORS is already permissive in development; production mode locks
  the origin to the codeless server itself.

No new RPC methods for the *bundle* itself. Plugin code that wants
to talk to the codeless server calls `RpcClient` methods exactly
like host code does — including methods the plugin's own backend
exports through `register_tool(...)`, which appear as a normal tool
in the agent surface. Plugin UI talking *directly* to plugin tools
without going through the agent is allowed (e.g. the estimator's
settings page reading the catalog list) and uses
`rpc.tools.call(tool_id, args)` — the same path the agent uses,
under the same persona allow-list.

## R6 — the plugin-side rule

Add to [`../../CLAUDE.md`](../../CLAUDE.md) under "Hard rules":

> **R6 — Plugin MF remotes import only shared scope.** A plugin's
> remote bundle must not import `@tauri-apps/*`, must not `fetch`
> the codeless server directly, must not bundle its own copy of
> React or zustand. The only allowed network egress is through
> `RpcClient`, sourced from the MF shared scope. The host shell
> decides which `RpcClient` impl is injected (browser, desktop,
> mobile); the plugin never knows the difference.

Same reasoning as R2, applied one layer out. The shape of the
guarantee is identical: one codebase, four shells, even for plugins.

Enforcement: an ESLint rule in `@codeless/plugin-ui-sdk/eslint-config`
that plugin `rsbuild.config.ts` extends. Importing a forbidden module
is a build-time error.

## The authoring SDK — `@codeless/plugin-ui-sdk`

In-tree under `ui/codeless-ui/packages/plugin-ui-sdk/`. Forked from
rubix's `extension-ui-sdk` at SHA `<TBD-at-port-time>`. The fork
keeps:

| Rubix asset | Codeless rename | Notes |
|---|---|---|
| `mf.ts` | `mf.ts` | MF remote registration helpers; rubix's slot vocabulary stripped, replaced with codeless slots above |
| `registration.ts` | `registration.ts` | Slot contribution wiring; rewritten against codeless slot ids |
| `components/` (editable-collection, etc.) | `components/` | Kept; shadcn-derived primitives line up with codeless theme |
| `hooks/` | `hooks/` | Kept; usage of `@rubix/agent-client` → `@codeless/rpc` |
| `editable-collection.test.tsx` + vitest config | Kept | Real test coverage for the MF wiring; vitest config inherits codeless-ui's |
| `@rubix/agent-client` dep | `@codeless/rpc` | The RpcClient seam |
| `@rubix/ui-core` dep | `@codeless/ui-core` (existing in `ui/codeless-ui/`) | shadcn primitives |

Every ported file gets a `// codeless-ported-from: rubix-workspace/
extension-ui-sdk/<path>@<sha>` header. No upstream tracking; if a
rubix fix matters, it's a re-port decision.

Author-facing API surface (the load-bearing shape):

```ts
// plugin's src/AssistantPanel.tsx
import { useRpc, useThread, useAttachments } from "@codeless/plugin-ui-sdk";

export default function AssistantPanel({ threadId }: { threadId: string }) {
  const rpc = useRpc();                    // injected by host
  const thread = useThread(threadId);      // shared tanstack-query cache
  const recent = useQuery(
    ["notes.recent", thread.persona_id],
    () => rpc.tools.call("notes.list_recent", { limit: 10 }),
  );
  return <PanelShell>{...}</PanelShell>;
}
```

A plugin author never imports `@tanstack/react-query` directly even
though they call `useQuery` — the SDK re-exports it from shared
scope, so the host's cache is the plugin's cache. This is how a
plugin's mutation invalidates a query the host registered.

## Build pipeline — what a plugin's `rsbuild.config.ts` looks like

```ts
import { defineConfig } from "@rsbuild/core";
import { pluginModuleFederation } from "@module-federation/rsbuild-plugin";
import codelessShared from "@codeless/plugin-ui-sdk/rsbuild-shared";

export default defineConfig({
  plugins: [
    pluginModuleFederation({
      name: "notes",
      filename: "remoteEntry.js",
      exposes: {
        "./AssistantPanel": "./src/AssistantPanel.tsx",
      },
      shared: codelessShared,            // pinned singletons from the SDK
    }),
  ],
  output: {
    distPath: { root: "../ui" },         // → plugins/notes/ui/
  },
});
```

`codelessShared` is the single source of truth for shared-singleton
versions. A plugin that pins a different React version fails MF's
version check at runtime and renders a clear error in the slot ("MF
version conflict: react ^18 (plugin) vs ^19 (host)") rather than a
white screen.

## Across shells — browser, desktop, iOS, Android

The whole point of R6 is that plugin UI works identically on every
shell because the only thing it does is render React + call
`RpcClient`. Concretely:

- **Browser** — `<script type="module">` loads `mf-manifest.json`,
  bundles arrive over HTTP from `/plugins/<id>/ui/*`. Production
  uses the codeless server's own origin; CORS is not an issue.
- **Tauri desktop** — same wire; the Tauri webview fetches the bundle
  over HTTP from the embedded codeless server. No `tauri://` magic.
  The `RpcClient` impl is `TauriIpcClient`, injected before the host
  mounts; the plugin sees `RpcClient` and doesn't know the
  difference.
- **iOS / Android (Tauri 2 mobile)** — webview fetches the bundle
  over HTTP from the *remote hosted codeless server* (mobile is a
  thin client, per `SCOPE.md`). The RpcClient impl is
  `HttpSseClient`; same plugin code.

Bundle caching is the host shell's problem, not the plugin's: the
shell tells the MF host to use a stable cache key (plugin id +
version + hash). When a plugin is upgraded, the version bumps and
the cache key changes; clients re-fetch.

## Acceptance

The MF UI flavour is done when:

1. The `notes` plugin ships a `ui/` directory and an `AssistantPanel`
   remote, declared in `plugin.toml`. Loading the Assistant with the
   `notes` persona active renders the panel from the plugin's
   bundle, with React/RpcClient/zustand singletons unified.
2. A second plugin (`echo`, written purely as an integration smoke
   test in-tree) renders a `tool-result:echo.shout` card replacing
   the default JSON renderer for that one tool, proving the
   parameterised-slot match works.
3. An MF version-mismatch test
   `plugin_ui_e2e::mismatched_react_fails_loudly` proves a plugin
   pinning the wrong React version fails with a structured error in
   the slot, not a host crash.
4. The R6 ESLint rule rejects a plugin source file that imports
   `@tauri-apps/api/core` at build time.
5. The same plugin bundle renders identically when the Assistant
   page is loaded from a Tauri desktop shell (verified by hand;
   automated browser-shell coverage is enough for CI).

## Open questions

- **OQ-UI-1. Resolved 2026-05-18 (plugin-substrate-runtimes stage
  1): the `@codeless/plugin-ui-sdk` semver pin is the slot
  vocabulary contract.** The host reads each plugin's declared SDK
  version from its `package.json` (`@codeless/plugin-ui-sdk` in
  `dependencies`), refuses to mount a plugin whose declared SDK
  major.minor is newer than the host's, and degrades to "no slot
  mounted, structured error in the slot's error boundary" rather
  than crashing the host. A separate "slot vocabulary version"
  field would drift from the SDK version it claims to pin; the
  package version is already the contract every plugin declares.
- **OQ-UI-2.** Server-side render / SSG. Codeless UI is SPA today;
  plugin bundles are dynamic chunks. If/when we add SSR, plugin
  components need to declare whether they're SSR-safe. **Lean: SSR
  is out of scope; punt the design until a real ask appears.**
- **OQ-UI-3.** Sandbox per-remote (CSP / iframe-of-last-resort).
  R5 is single-tenant; in MVP plugin UI runs at host trust. Once we
  ship a third-party plugin registry (Phase 7+), per-remote CSP or
  an opt-in iframe wrapper might be needed. **Lean: not now,
  reserved for Phase 7 alongside OIDC and a plugin marketplace.**
- **OQ-UI-4.** Plugin theming. Plugins inherit the host's tailwind
  theme tokens by importing from `@codeless/ui-core`. Do we allow a
  plugin to ship its own theme overrides? **Lean: no.** A plugin
  that doesn't fit the host theme should change its layout, not its
  colour palette. If a real use case appears, it's a CSS-scoped
  contribution, not arbitrary `:root` rewrites.

## Decisions locked

1. **Module Federation, not iframes or web components.** Theming,
   focus, drag-and-drop, action cards work in-tree.
2. **Slot vocabulary is host-defined and finite.** Plugins cannot
   invent slots; adding a slot is a host-side change.
3. **Shared singletons are pinned in `@codeless/plugin-ui-sdk`.**
   Plugin and host build configs both import the same shared map.
4. **R6 — plugins import only from the shared scope** —
   `RpcClient`, React, shadcn primitives, SDK helpers. No
   `@tauri-apps/*`, no direct `fetch` to the codeless server.
5. **`@codeless/plugin-ui-sdk` is an in-tree fork of rubix's
   `extension-ui-sdk`.** Ported files carry `// codeless-ported-
   from:` headers. No upstream tracking, no patch log.
6. **One UI framework, forever (R3 unchanged).** A plugin that
   wants UI ships React/TS; a plugin that genuinely cannot ship
   React falls back to no UI (its tool results render with the
   default JSON renderer).
