# UI storage audit — per-tab vs library-level

This audit lists every UI call site that talks to `localStorage`,
`sessionStorage`, or a zustand `persist` middleware, and classifies
each one as **library-level** (shared across tabs is the correct
behaviour) or **tab-level** (must move to `sessionStorage` or
in-memory so two tabs viewing two different workspaces do not bleed
state into each other).

This is the storage half of the workspace-scoping job
(`.codeless/jobs/workspace-scoping/`). The wire half (`EventFilter`,
`fs.*` `repo_id` argument) and the deep-link half (`?workspace=…`
URL param) only buy two-tab isolation if no surface silently shares
state through a key that both tabs read and write.

## Method

```
grep -rn 'localStorage\|sessionStorage' ui/codeless-ui/src/
grep -rn 'persist(' ui/codeless-ui/src/
```

A hit is **library-level** when the value is naturally global to the
user / install (theme, app-wide preferences, RPC base URL override,
last-update-check timestamp, user-authored snippets) — two tabs
seeing the same value is the desired UX.

A hit is **tab-level** when the value answers the question "what is
*this* tab looking at?" (active workspace, open job tabs, currently
focused assistant thread) — two tabs should answer that question
independently.

`persist()` from `zustand/middleware` does not appear in the tree —
every persistent store goes either through `localStorage` directly or
through `getStore(name)` in `lib/shell/kv-store.ts`.

## Classification

### Library-level — keep in `localStorage`

| Site | Key(s) | Rationale |
| --- | --- | --- |
| `modules/theme/ThemeProvider.tsx` | `codeless-theme-fast-path` | User-wide theme preference, used only for first-paint fidelity; server is the source of truth. |
| `modules/updater/useUpdater.ts` | `codeless-last-update-check` | About the installed binary, not about what the tab is viewing. |
| `lib/rpc/config.ts` | `codeless-rpc-base-url`, `codeless-rpc-bearer-token` | Build-time override for which server this UI talks to. Identical across tabs by construction. |
| `modules/jobs/JobPage.tsx` | `codeless-pinned-tabs:${jobId}` | Already partitioned by `jobId`; a job belongs to exactly one workspace, so two tabs viewing different workspaces address different keys. Pinning a stage tab in one tab and seeing it in another tab viewing the same job is desired. |
| `lib/shell/kv-store.ts` (browser/mobile `KVStoreAdapter` backend) | `codeless-<name>:*` prefix | Library-level abstraction. Per-consumer judgement below. |
| `modules/settings/store.ts` via `getStore("preferences")` | `keyboard-shortcuts`, others | User preferences, shared across tabs. |
| `modules/ai/lib/agents.ts` via `getStore("ai-agents")` | `custom-agents`, `active-agent-id` | Authored agents are user-wide; active-agent selection is a user preference, not a per-tab view. |
| `modules/ai/lib/sessions.ts` via `getStore("ai-sessions")` | `sessions`, `active-session-id`, `messages:*` | The chat history belongs to the user across tabs. (The `/assistant` rail is itself a singleton tab — see `useTabs` below — so cross-tab leakage of the *active* session is not a regression on top of today's behaviour.) |
| `modules/ai/lib/snippets.ts` via `getStore("ai-snippets")` | `snippets` | User-authored, shared across tabs. |
| `modules/ai/lib/todos.ts` via `getStore("ai-todos")` | `todos:<sessionId>` | Partitioned by AI session id; not addressable cross-tab anyway. |

### Tab-level — moved in this stage

| Site | Old key (localStorage) | New key (sessionStorage) | Why |
| --- | --- | --- | --- |
| `modules/assistant/focusStore.ts` | `codeless.assistant.currentThreadId` | `codeless.assistant.currentThreadId.v2` | "What assistant thread is *this* tab's footer / rail focused on" is a per-tab projection over the (library-level) thread list. Sharing it across tabs caused the symptom: open tab A, focus thread X, open tab B, the footer in tab B silently jumps to X. |
| `modules/tabs/lib/useTabs.ts` | `codeless-open-job-tabs-v3` | `codeless-open-job-tabs.v4` | Open job-detail / jobs / patches tabs are the user's *workbench* for one tab — they are how the user is laying out one workspace's work. Opening a `JobDetailTab` against workspace A's job in tab A must not appear in tab B's tab bar when tab B is viewing workspace B. |

`sessionStorage` is the right surface for both:

- It survives reloads (so `?workspace=<repo_id>` deep-link reload
  rehydrates the same tab's workbench) — the property that justified
  picking `localStorage` originally.
- It is **per browsing-context** — a second tab gets a fresh empty
  store, even on the same origin. Two browser tabs on the same
  Codeless server therefore see independent assistant focus and
  independent open-job-tab lists.
- On the Tauri desktop path each window already has its own
  `sessionStorage`, so the desktop shell behaviour is unchanged.

The key versions are bumped (`.v2`, `.v4`) so a user upgrading from
the localStorage layout starts with an empty tab-level store rather
than inheriting a snapshot that the old code wrote into
`localStorage`. The old `localStorage` rows become dead weight; they
do not need active cleanup because `localStorage`'s typical quota
makes the leak negligible and a future `clear-stale-storage` task
can sweep them.

## Out of scope

- Host allowlist, CORS, random session prefix. Tracked separately
  per `crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md §Security`.
- A general "per-tab namespacing" helper. Two call sites do not
  justify the abstraction yet (R4 — three similar lines is better
  than a premature one).
- Migrating any of the library-level `localStorage` keys to the
  `KVStoreAdapter` abstraction. Orthogonal cleanup.
