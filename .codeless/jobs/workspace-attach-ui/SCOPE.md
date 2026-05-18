# Scope — workspace-attach-ui (UI grind, milestones 3 & 4)

The full design is **[`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)**
in this repo. This brief is the trimmed per-job scope; the deep
design lives there and wins on any disagreement.

The **server-side** half of WORKSPACE-ATTACH (milestones 1 & 2) is
already merged on master (PR #6, commits `0e4eeb1` … `c36d4d2`). The
backend exposes the four RPCs, persists `attached_workspaces`, serves
`fs.*` against the canonical allowed-roots list, emits liveness
events, and freezes `ServerInfo.fs_root` to the boot value. None of
this is reachable from the UI yet — that's what this job fixes.

## Goal

Land milestones 3 and 4 of WORKSPACE-ATTACH on `master` via the
`codeless/workspace-attach-ui` branch. After this job:

1. `RpcClient` exposes `attachWorkspace`, `detachWorkspace`,
   `listWorkspaces`, `validateWorkspacePath`. Both `HttpSseClient`
   (browser/mobile) and `TauriIpcClient` (desktop) implement them.
2. `PathPicker` is a shell-injected capability with browser and
   desktop implementations behind the existing
   `ui/codeless-ui/src/lib/shell/` pattern (R3 — no per-shell `.tsx`
   forks).
3. The user opens **Settings → Workspaces**, sees the live list,
   attaches a directory through a validated picker, and detaches one
   with the running-jobs policy radio rendered when needed.
4. Empty state replaces the current silent "fs_root not set" failure
   mode.

The `/workspaces` top-level route and the sidebar group are
explicitly **deferred** to a follow-up job per the doc's M4 phasing
note ("a Settings → Workspaces tab ships first as a smaller landing
surface, then the `/workspaces` route and sidebar group follow. Both
share the same components; the tab is not a parallel UI").

## In scope

- `RpcClient` interface additions in
  `ui/codeless-ui/src/lib/rpc/` (`client.ts` or wherever the
  interface lives) — four methods, typed against the existing
  generated `wire.ts` types.
- `HttpSseClient` implementations of all four methods (the wire types
  already exist in [`wire.ts`](../../../ui/codeless-ui/src/lib/rpc/generated/wire.ts)).
- `TauriIpcClient` implementations of all four methods, following the
  existing `rpc_<method>` IPC contract documented in
  [`DOCS/UI-ARCHITECTURE.md`](../../../DOCS/UI-ARCHITECTURE.md).
- `PathPicker` interface under
  `ui/codeless-ui/src/lib/shell/path-picker.ts` matching the doc's
  TypeScript signature:
  `pickDirectory(opts?: { startPath?: string }): Promise<string | null>`.
- Browser shell injector under
  `ui/codeless-ui/src/shells/browser/` — uses
  `window.showDirectoryPicker()` where available, falls back to a
  typed input (Firefox / Safari) with live
  `validate_workspace_path` feedback.
- Tauri desktop shell injector under
  `ui/codeless-ui/src/shells/desktop/` — uses
  `@tauri-apps/plugin-dialog` `open({ directory: true })`.
- Per-tab active-workspace store (zustand, mirrors the
  `useChatStore` pattern) that subscribes to `workspace_attached` /
  `workspace_detached` events through `RpcClient.subscribe()` and
  switches active to the most-recently-attached remaining workspace
  on detach.
- Settings → Workspaces tab — table per the doc §UX layout, attach
  modal, detach modal.
- Attach modal: name + path + runner; path validates live
  (debounced ~250ms, server-side capped at ~5/s); inline ✓/✗ for
  every `WorkspaceProblem`; `Attach workspace` disabled until the
  validator returns no problems; on click, runs
  `add_repo` + `attach_workspace` in one logical step (server is
  idempotent on conflict, see §Edge cases of the doc).
- Detach modal: one-line confirm when no running jobs; two-radio
  (`Stop` / `LeaveRunning`) when there are; the structured
  `WorkspaceError::RunningJobs { jobs }` is rendered inline rather
  than string-matched.
- Empty-state screen when `list_workspaces` returns `[]`.
- Typed-wire snapshot test covering all four method names + arg
  shapes (per M3 exit criterion in the doc).
- One Playwright/RTL happy-path test per modal (attach + detach) per
  the M4 exit criterion in the doc.

## Out of scope

- The `/workspaces` top-level route and the sidebar group (M4
  follow-up phasing — explicit per the doc).
- M5: job-page filter by active workspace.
- M6: health-event badges (`workspace_unhealthy` /
  `workspace_recovered` rendering). Server emits exist; UI
  subscription is a follow-up.
- M7: `codeless serve --bind` default flip + `--port-file`. The
  `bind_tcp` helper already exists. Out of scope here.
- Mobile shell UI for workspaces (the doc explicitly defers iOS /
  Android attach to clone-URL in Phase 6).
- Any drag-and-drop affordance (open question 4 — resolved "no").

## Constraints

- **R1** — UI never spawns processes. `validate_workspace_path` and
  the live `git rev-parse` inside it run server-side already; the UI
  only renders the result.
- **R2** — only `RpcClient`. The browser shell injector may import
  the Web `showDirectoryPicker` API; the desktop shell injector may
  import `@tauri-apps/plugin-dialog`. **No `@tauri-apps/*` import
  outside `ui/codeless-ui/src/shells/desktop/`.**
- **R3** — one responsive component. The `PathPicker` injection is
  the only shell-visible split, and it is a function injection, not
  a parallel UI tree. **No `Foo.web.tsx` / `Foo.desktop.tsx`** under
  the new code.
- **R4** — `attached_workspaces` is the source of truth in SQLite.
  The store subscribes to events; it does not cache authoritative
  state. The initial `listWorkspaces()` call hydrates; the
  subscription keeps it live.
- **R5** — bearer token authorises every new method identically. No
  per-workspace permissions, no separate gating in the UI.
- **MSRV / lint gates** apply to any incidental Rust touched (none
  expected). The UI gates are `pnpm -C ui/codeless-ui lint` and
  `pnpm -C ui/codeless-ui test` (snapshot + Playwright).

## Deliverables (what "done" looks like)

1. `codeless/workspace-attach-ui` branch with one commit per stage,
   pushed via mani.
2. `pnpm -C ui/codeless-ui test` green; the typed-wire snapshot
   covers the four new method signatures.
3. `pnpm -C ui/codeless-ui lint` green; **zero** new
   `@tauri-apps/*` imports outside `src/shells/desktop/`.
4. Manual smoke: `setup/init-session.sh start`, open the browser
   shell, navigate to Settings → Workspaces, attach the repo at
   `~/code/rust/codeless-workspace/codeless`, see it land in the
   table, detach it, see the empty state.
5. `DOCS/WORKSPACE-ATTACH.md` "Milestones" section: M3 flips
   `[~] → [x]`, M4 flips `[ ] → [x]` (with the `/workspaces` route +
   sidebar carved out as a follow-up note in the same edit so the
   doc stays honest).
6. The Playwright happy-path tests for attach + detach pass in CI.

## Open questions (resolve in stage 1, before any UI code)

The four workspace-attach open questions are already resolved
upstream — see [`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)
§"Open questions" and the prior job's
[`SCOPE.md`](../workspace-attach/SCOPE.md). This job inherits those
decisions and adds three M3/M4-specific ones to record here:

1. **Attach modal: does it call `add_repo` itself, or assume the
   repo row already exists?**
   Bias: do both — if `validateWorkspacePath().already_attached` is
   false and there's no `Repo` row matching the canonical path, call
   `add_repo` first, then `attach_workspace`. Idempotent because the
   server collapses on the canonical unique index.
2. **PathPicker fallback when `showDirectoryPicker` is undefined
   (Firefox / Safari):** typed input only, or typed input + recent-
   directories dropdown?
   Bias: typed input only for now; recent-directories is a polish
   item and would need a separate persisted store.
3. **Settings tab placement:** does Workspaces become the first tab,
   or sit between existing tabs?
   Bias: first tab. It's the entry point for a fresh install — the
   user can't do anything else until at least one workspace is
   attached.

Record the chosen answer + one-line *why* under each in this file
during stage 1; no UI code in stage 1.

## References

- Workspace doc (authoritative): [`DOCS/WORKSPACE-ATTACH.md`](../../../DOCS/WORKSPACE-ATTACH.md)
- Server-side job (predecessor): [`.codeless/jobs/workspace-attach/SCOPE.md`](../workspace-attach/SCOPE.md)
- UI architecture: [`DOCS/UI-ARCHITECTURE.md`](../../../DOCS/UI-ARCHITECTURE.md)
- UI port audit: [`DOCS/UI-PORT-AUDIT.md`](../../../DOCS/UI-PORT-AUDIT.md)
- Agent rules: [`CLAUDE.md`](../../../CLAUDE.md)
- Wire types (generated, already in tree): [`ui/codeless-ui/src/lib/rpc/generated/wire.ts`](../../../ui/codeless-ui/src/lib/rpc/generated/wire.ts)
