# Scope — workspace-scoping

## Goal

The user can open two browser tabs at `http://127.0.0.1:1420`, pick a
different attached workspace in each (e.g. `codeless` in tab A,
`starter` in tab B), and see:

- only that workspace's jobs in the jobs list,
- only that workspace's file tree in the file explorer,
- only that workspace's live events (submitting a job in tab A does
  not surface in tab B's event stream),
- the same workspace after a hard refresh of the tab.

Today none of those hold: `subscribe()` only knows `All` / `Job`,
`fs_cwd` returns the global worktrees root, the workspace picker does
not drive the file explorer, and `activeRepoId` is in shared storage
so switching workspaces in one tab moves the other tab too.

This job lands milestones 5, 6, and 9 from
[`../../../crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md`](../../../crates/codeless-tauri-desktop/BROWSER-LAUNCHER.md)
as one branch. Milestone 2 (publish-site audit) is folded in as
stage 1 with a REVIEW gate so scope is honest, not assumed.

## In scope

- `EventFilter` gains `Repo { repo_id }` and `Library` variants; old
  `All` stays for the global event-log view only.
- Server-side filter at the fan-out layer in `codeless-runtime`,
  delivering only matching events to each subscriber.
- `fs.*` RPCs (`fs_cwd`, `fs_read_dir`, `fs_read_file`,
  `fs_write_file`, and any siblings discovered in stage 4) take a
  `repo_id` and resolve their jail root from
  `attached_workspaces.fs_root`. Calls with an unknown or detached
  `repo_id` return a typed error.
- UI passes `activeRepoId` on every `subscribe` and every `fs.*` call;
  the file explorer rehydrates when the picker changes; a parallel
  `Library`-scope subscription keeps the picker live.
- `?workspace=<repo_id>` URL parameter is read on load and written on
  every `setActive` via `history.replaceState`, so refresh and
  share-links preserve the workspace.
- Per-tab storage audit so cross-tab `localStorage` leakage does not
  reintroduce the "two views share state" symptom.
- Final smoke test recorded in `DOCS/WORKSPACE-SCOPING-SMOKE.md`:
  two-tab cross-talk test, refresh test, deep-link test.

## Out of scope

- The launcher itself — milestones 7 (revert per-workspace data
  dirs), 8 (browser launcher + tray), 10 (Linux exit test), 11
  (macOS / Windows). This job is "workspace-scoped UI"; the launcher
  is "delivery mechanism."
- The §Security mitigations from BROWSER-LAUNCHER.md (Host
  allowlist, CORS lockdown, random URL prefix). Those are milestone
  4 and a prerequisite for default-off auth on loopback, not for
  workspace scoping. Auth stays `--require-token` opt-in, same as
  today.
- Removing `TauriIpcClient`. It stays unchanged; its `subscribe` and
  `fs.*` shims must also accept the new arguments, but the desktop
  shell path is not being deprecated here.
- Renaming "repo" to "workspace" in the wire types. The schema uses
  `repo_id` because that is the existing `RepoId`. A future rename
  is its own job.
- Multi-tenant auth, per-workspace permissions. R5 single-trust-
  boundary stays.

## Constraints

- **R1** (CLAUDE.md): no `std::process` or `tokio::process` outside
  `codeless-adapters-host`. The workspace-scoping work touches RPC,
  runtime, and UI — none of these are allowed to import process
  spawning. Grep on each commit.
- **R2** (CLAUDE.md): the UI imports only `RpcClient`. Do not add
  `@tauri-apps/api/core` or `@tauri-apps/api/event` imports to
  components in the course of this work.
- **R3**: one UI tree. No `Foo.web.tsx` / `Foo.desktop.tsx` splits.
- **R4**: SQLite stays the source of truth. Workspace state is read
  from `attached_workspaces`, not cached in the client.
- **R5**: single trust boundary unchanged. Bearer token stays the
  authorisation primitive; loopback default-off remains the
  startup default for this job.
- MSRV stays 1.78; `cargo clippy --workspace --all-targets -- -D
  warnings` must be clean; `cargo fmt --check` must pass.
- The `All` variant of `EventFilter` stays for the global log view.
  Do not delete it; deprecation is its own decision.
- `EventFilter::Repo { repo_id }` and `EventFilter::Library` are the
  shapes per BROWSER-LAUNCHER.md §"RPC additions". The peer review
  flagged that two-fields-where-one-would-do is awkward; this job
  adopts the enum-variant shape (one field), not the tag+scope
  shape the doc originally proposed.
- `TauriIpcClient` must continue to compile and pass its existing
  tests after the RPC signatures change — its method shims need
  updating in the same commit as the wire change.

## Open questions

1. Does every `EventBus::publish(...)` call site already publish an
   event whose payload carries a `repo_id` (or that is structurally
   library-scope)? **Stage 1 answers this.** If the answer is no, the
   REVIEW gate decides whether the missing-`repo_id` events get
   patched in this job or split into a separate job that lands
   first.
2. How many `fs.*` RPCs exist today? The doc lists `fs_cwd`,
   `fs_read_dir`; the codeless-rpc tree may have more. Stage 4
   audits exhaustively.
3. Does the existing `subscribe` event-cursor support survive a
   browser tab going to the background and reconnecting? This was
   listed as "verify during milestone 9" in the doc; this job
   verifies it in stage 7 and records the result. If broken, the
   fix is client-side reconnect, not in scope unless trivial.
4. Is `activeRepoId` currently persisted in `localStorage` or zustand
   `persist()`? Stage 8 finds out; if yes, it moves to in-memory or
   `sessionStorage` so two tabs are independent. If no, document
   why and move on.
