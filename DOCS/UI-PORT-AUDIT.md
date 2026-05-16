# UI port audit

Reference: upstream `crynta/terax-ai` @ commit `a628d62db1bfabf44085aeca992689ff5c4c6224`,
license Apache-2.0.

This is the per-file worklist for converting the upstream-derived
Terax UI to talk to `codeless-runtime` through the `RpcClient`
interface. The architectural rationale lives in
[`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md) — read that first if
you're new to the boundary.

The full upstream tree was already copied into
`codeless/ui/codeless-ui/` (with strings renamed terax→codeless and
attribution preserved in `NOTICE.md`). The remaining work is the
conversion grind: each file that imports `@tauri-apps/*` is rewired
through `RpcClient.*` (typed RPC) or a shell-injected capability
adapter (clipboard, file picker, biometric).

## What "done" looks like per file

1. No imports from `@tauri-apps/api/*` or `@tauri-apps/plugin-*`.
2. All transport calls go through `useRpc()` (or a capability adapter
   passed in as a prop).
3. Type-check + build still pass; the surface still renders against
   `MockRpcClient`.
4. Renamed strings preserved (no `Terax`/`terax`/`TERAX` outside
   `NOTICE.md`).

## Reuse list (with concrete upstream paths)

Maps SCOPE.md's "Reuse (visual layer)" bullets to actual files at the
pinned SHA. Overlaps with the surface table in
[`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md); kept separately because
the value here is the *concrete upstream paths* — useful when
diffing against upstream during the conversion grind.

| SCOPE bullet | Upstream source |
|---|---|
| CodeMirror 6 editor + theming | `src/modules/editor/` (8 files), CodeMirror deps in `package.json` |
| xterm.js terminal component | `src/modules/terminal/` — replace `lib/pty-bridge.ts` with WS transport |
| File explorer React component | `src/modules/explorer/` — replace `lib/contextActions.ts`, `useFileTree.ts`, `ExplorerSearch.tsx` IPC paths |
| AI message / streaming markdown rendering | `src/components/ai-elements/` (9 files: `conversation`, `message`, `markdown-code`, `code-block`, `reasoning`, `shimmer`, `snippet`, `tool`, `context`) |
| Approval-card UI | `src/modules/ai/components/AiToolApproval.tsx`, `PlanDiffReview.tsx` |
| Settings shell, providers form layout | `src/settings/` — replace `ProviderKeyCard.tsx`, `ModelsSection.tsx` IPC; lift `SettingsApp.tsx` shell |
| Tailwind + shadcn/ui setup | `components.json`, `vite.config.ts`, `tsconfig.json`, `src/components/ui/` (38 shadcn primitives), `src/styles/` |
| Project-memory concept | `TERAX.md` → already mapped to `codeless/CODELESS.md` (Phase 1 stage 7) |

## Replace list (do not port — already covered by Rust core)

These exist in the upstream tree. Do not `cp` them; the Rust core
provides the equivalent over `RpcClient`.

- `src/modules/ai/lib/agent.ts` — AI SDK loop, replaced by `codeless-runtime`
- `src/modules/ai/lib/transport.ts` — replaced by `codeless-rpc`
- `src/modules/ai/lib/native.ts` — direct Tauri invokes, replaced by `RpcClient`
- `src/modules/ai/lib/keyring.ts` — replaced by `RpcSecrets` (host + hosted backends)
- `src/modules/ai/lib/sessions.ts`, `agents.ts`, `snippets.ts`,
  `todos.ts` — Zustand stores backed by Tauri store plugin; replace
  with RPC-backed equivalents
- `src/modules/ai/tools/*` (`context.ts`, `edit.ts`, `fs.ts`, `search.ts`,
  `shell.ts`, `subagent.ts`, `terminal.ts`, `todo.ts`, `tools.ts`) —
  tools execute in Rust now; TS keeps only the schema for UI rendering
- `src/modules/ai/store/agentsStore.ts`, `snippetsStore.ts` — replaced
  by RPC-subscribed stores
- `src/modules/updater/` — re-evaluate for hosted mode (SCOPE "Drop")
- `src-tauri/` — host-side Rust modules (`pty`, `shell`, `fs`) inform
  Phase 6 `codeless-adapters-host` design; do not lift verbatim

## Tauri-coupled files (R2 violation surface)

Conversion progress, current tree (counts exclude `*.md`,
`src/shells/<shell>/*` which are legitimate injection points per
[`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md), and a doc-comment
false-positive in `src/lib/rpc/client.ts:5` that mentions
`@tauri-apps/api/*` in prose):

- **Starting point: 31 files** imported `@tauri-apps/*` outside the
  shell zone (snapshot when this audit was first written).
- **After shell-injection phase: 13 files.**
- **Current: 4 files.** The `fs.*` / `secrets.*` / `shell.*` RPC
  surface has been mirrored UI-side (hand-mirrored from the Rust
  shapes that `codeless-rpc` will codegen via specta) and the
  `MockRpcClient` implements all of it in-memory. The 9 fs/secrets/
  shell callers have been routed through `useRpc()` (components and
  hooks) or through `configureNative(rpc)` (the legacy `native.ts`
  free-function surface its Zustand stores and AI tools depend on).

**The 4 remaining files:**

```
# PTY over WebSocket — reserved channel per SCOPE.md (2)
src/modules/terminal/lib/pty-bridge.ts
src/modules/terminal/lib/useTerminalSession.ts

# Legitimate Tauri-only shell entry (1)
src/settings/main.tsx                    # entry for the separate Tauri
                                         # settings window; goes away
                                         # if we ever switch desktop to
                                         # inline-only settings

# Documented exception (1)
src/lib/rpc/tauri-ipc-client.ts          # desktop RpcClient impl;
                                         # lives in src/lib/rpc/
                                         # alongside HttpSseClient and
                                         # MockRpcClient per CLAUDE.md
```

**Conversion target**: zero outside `src/shells/<shell>/` (plus the
two documented exceptions above — `tauri-ipc-client.ts` and the
Tauri-window `settings/main.tsx`). The shell entry files are
*expected* to import `@tauri-apps/*` directly — that's where every
Tauri-backed adapter is constructed.

## Phase 2 work groups

Not a contract — feeds the Phase 2 session doc. Grouped by
unblock-status rather than numbered, because order is determined by
which RPC methods exist today.

### Landed

- [x] **Upstream tree in place.** `codeless/ui/codeless-ui/` copied,
  strings renamed terax→codeless, `NOTICE.md` + `LICENSE` in place,
  `pnpm dev` builds, `pnpm exec tsc --noEmit` clean.
- [x] **RPC boundary layer.** `RpcClient` interface, `HttpSseClient`,
  `MockRpcClient`, `<RpcProvider>`, `useRpc()` / `useRepos()` /
  `useJobs()` / `useJob()` / `useEventStream()` hooks. Bearer-token
  + base-URL config. Lives in `src/lib/rpc/`.
- [x] **`ai-elements`.** `src/components/ai-elements/` lifted with no
  Tauri imports.
- [x] **`TauriIpcClient`.** Second `RpcClient` impl wrapping
  `invoke()` and Tauri 2 `Channel<T>` for `subscribe()`. Lives in
  `src/lib/rpc/` alongside `HttpSseClient`. Documents the wire
  contract (`rpc_<method>`, `rpc_subscribe`, `rpc_unsubscribe`) the
  `codeless-tauri-desktop` Rust crate will implement in Phase 5.
- [x] **Jobs dashboard mounted inside `<App />`** as a new `"jobs"`
  tab kind (singleton; `Ctrl+J`; appears in the `+` menu alongside
  Terminal/Editor/Preview). `?view=jobs` debug route deleted.
- [x] **Shell-injection capability adapters** (`src/lib/shell/`,
  10 in total). Each has a no-op / browser-equivalent default and a
  Tauri-backed impl in `src/shells/desktop/`. See
  [`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md) "Capability adapters"
  for the full list and the consumers that graduated.
- [x] **In-app settings overlay for browser/mobile.** New
  `useInlineSettingsStore` flips a Zustand singleton; `<App />`
  renders `<SettingsApp inline={…} />` as a full-screen overlay when
  open. Esc closes. Desktop is unchanged — still opens a separate
  Tauri window via the same `useSettingsWindow().open(tab)` adapter
  call. Caller code never branches on shell.

### Blocked on Rust `RpcServer` additions

These are the only remaining UI conversions. Each waits on a
specific Rust RPC method landing in `codeless-rpc::RpcServer` /
`codeless-runtime::rpc::InProcessRpc`.

- [S] **Settings → provider keys** (`ai/lib/keyring.ts`,
  `composer.tsx`, `native.ts`) — needs
  `RpcServer::secrets_{set,get,list,rm}` plus `fs_read_file` for
  `composer.tsx`'s attach-by-path. The keychain impl already exists
  in `codeless-adapters-host::secrets`; only the trait method +
  transport binding is missing.
- [M] **File explorer** (`useFileTree.ts`, `contextActions.ts`,
  `ExplorerSearch.tsx`) — needs `RpcServer::fs_{read_dir,search,move,delete}`.
- [M] **Editor** (`useDocument.ts`, `NewEditorDialog.tsx`) — needs
  `RpcServer::fs_{read_file,write_file}`.
- [M] **Terminal** (`pty-bridge.ts`, `useTerminalSession.ts`) — needs
  WebSocket-backed `pty_*` (or its own `PtyClient` surface — open
  question per SCOPE.md). PTY is the only RPC that doesn't fit the
  request/reply shape; it earns its own transport.
- [S] **Status bar** (`CwdBreadcrumb.tsx`) — needs `RpcServer::fs_cwd`
  or a config-source equivalent.

### Job → Stage → Task decomposition (the AI chat surface)

The AI chat panel's full conversion is blocked behind the
runtime side of the **template parser** — the piece that
decomposes a Job into Stages and Tasks. The schema is there
(`stages`, `tasks`, `reviews` tables in
[`crates/codeless-runtime/migrations/0001_initial.sql`](../codeless/crates/codeless-runtime/migrations/0001_initial.sql)),
the events are wired (`task-enqueued`, `task-started`, `ai-token`,
`stage-completed` …), and `enqueue_task` / `insert_stage` are
implemented in `store.rs`. What's missing: `drive_job` currently
calls `runner.run(...)` monolithically (one provider session per
Job) instead of parsing `template_yaml` into a Stage/Task DAG and
letting the lease-based scheduler hand each Task to a runner.

Per SCOPE.md *"Task = one provider session (one CLI invocation or
one REST conversation turn-set)"* — until the parser lands, "session
per Stage" / "session per Task" is conceptual; today it's "session
per Job". Once the parser exists the UI gets:

- N sessions per Job, surfaced as Task rows in
  [`JobTimeline.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/JobTimeline.tsx)
- per-Stage verify gating (`verify_cmd` → `verify-passed/failed`
  events)
- review gates at Stage boundaries (`Review` table, already in schema)
- delete `src/modules/ai/lib/agent.ts` (the in-browser Vercel AI SDK
  loop) and the executor code under `src/modules/ai/tools/*`; keep
  only the tool *schemas* for UI rendering

### Wrap-up

- [S] Wire the specta codegen output from `codeless-types` into the UI
  build so `wire.ts` and `methods.ts` are generated, not hand-mirrored.
- [S] Audit pass: zero `@tauri-apps/*` imports outside
  `src/shells/<shell>/` (plus the two documented exceptions:
  `tauri-ipc-client.ts` and `settings/main.tsx`); zero `terax`
  strings outside `NOTICE.md`.
