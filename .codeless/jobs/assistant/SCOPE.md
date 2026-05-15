# Scope — Assistant

> Source of truth: [`DOCS/ASSISTANT-SCOPE.md`](../../../../DOCS/ASSISTANT-SCOPE.md).
> This file is the per-job brief the runner reads before every stage;
> it is intentionally short. When this file disagrees with
> ASSISTANT-SCOPE.md, ASSISTANT-SCOPE.md wins — open an issue and
> update this file.

## Goal

Ship the in-app Assistant: a conversational surface (`/assistant`)
that lets the user view, manage, create and edit-scope Codeless
jobs by talking to the same runner that powers headless ticks. The
assistant is **not** a second runner — it is a thin UI + RPC client
over `codeless-runtime`.

## In scope

- Extract a shared `CommonChat` component out of the existing
  `JobChat` (in `RunPane.tsx`) and `AiChat`, with feature parity:
  message rendering, streaming, attachments (drag-drop + file picker
  + clipboard paste), tool-call cards, scroll anchoring.
- New `/assistant` page: thread-list rail, `CommonChat` for the
  selected thread, collapsible context panel.
- Server-side persistence: `assistant_threads` and
  `assistant_messages` tables.
- New RPCs: `assistant.{listThreads,createThread,deleteThread}` and
  `uploadAttachment(threadId, file)`.
- Action cards for view/manage (start/stop/pause/resume/restart/update),
  create-job (draft → confirm) and edit-scope (inline diff +
  open-in-editor).

## Out of scope

- A new agent runtime. The assistant calls into the existing runner
  via `RpcClient`. No new planner, no new tool framework.
- Per-user permissions, multi-tenant isolation. Single-tenant trust
  boundary (R5) holds.
- Any per-shell UI fork (`Assistant.web.tsx`, `Assistant.mobile.tsx`).
  R3 — one responsive component.
- Mobile-specific behaviour beyond what R1–R5 already require.
- The agent prompts / planner changes — assistant uses what's there.

## Constraints

- **R1**: nothing the assistant does spawns processes from the UI.
  All `process::Command` use stays in `codeless-adapters-host`.
- **R2**: only `RpcClient`. No `@tauri-apps/api`, no direct `fetch`.
- **R3**: one responsive UI; no per-shell forks.
- **R4**: assistant threads, messages, attachments and drafts live
  in SQLite; the UI subscribes via `RpcClient.subscribe()`.
- **R5**: bearer token authorises the assistant identically to every
  other client; no per-job or per-action scopes.
- The `kind` prop on `CommonChat` is UI-only — capabilities are
  derived **server-side** from the thread row, never trusted from
  the client.

## Deliverables

- `codeless/ui/codeless-ui/src/modules/chat/` with `CommonChat`,
  consumed by the three existing call sites.
- New `/assistant` route + thread-list UI.
- `assistant.*` RPCs + tables + migrations.
- View/manage, create, and edit-scope action cards wired end-to-end
  with confirmation gates.

## Open questions (resolve in stage 1)

1. **Attachments directory.** Bias option 1: workspace-scoped dir
   `<codeless-data>/threads/<thread_id>/attachments/`. Job chat keeps
   writing to the worktree. Confirm before milestone 1 ends.
2. **`AiChat` server-state migration.** `chatStore.ts` currently
   holds authoritative chat state client-side, which violates R4.
   Decide whether the store disappears entirely or shrinks to UI
   presentation state only (scroll position, composer draft).
3. **Job-creation "just do it" path.** Bias: no — confirmation is
   cheap and prevents irreversible spend on the wrong scope.

Record the chosen answers in this file (under "Decisions" below)
before stage 3 begins.

## Decisions

### 1. Attachments directory — workspace-scoped (option 1)

Assistant attachments land in
`<codeless-data>/threads/<thread_id>/attachments/<file_id>-<original_name>`,
where `<codeless-data>` is the same data root the runtime already
resolves for SQLite and run logs. Rationale:

- Threads outlive any single worktree. Pinning attachments to a
  worktree makes them disappear when the job is archived or the
  branch is deleted, which is the wrong model for an assistant that
  is intended to span jobs.
- The directory is owned by `codeless-adapters-host`; the UI never
  sees a host path. `uploadAttachment(threadId, file)` returns a
  stable `attachment_id`; downloads/previews flow back through the
  same RPC (`assistant.getAttachment` — to be confirmed when the RPC
  surface is drafted).
- Job chat is **unchanged**: it keeps writing to the active
  worktree, because those attachments are part of the job's
  artifacts and travel with the worktree by design. `CommonChat`
  takes a `uploadAttachment` callback so each call site can plug in
  its own destination.
- Cleanup is bound to `assistant.deleteThread`: removing a thread
  row cascades to its attachments directory in the same RPC.

### 2. `AiChat` server-state migration — store shrinks to UI presentation only

`chatStore.ts` currently owns: api keys, selected model, session
list, per-session `Chat` instances + seeded messages, pending
selections, panel/mini visibility, agent meta, focus signal,
debounced message persistence. After migration:

- **Moves to the server (SQLite, surfaced via `RpcClient`):**
  session list and metadata (`sessions`, `activeSessionId`,
  `hydrateSessions`, `newSession`, `switchSession`, `deleteSession`,
  `renameSession`), persisted messages (`persistMessages`,
  `seedMessages`, `pendingPersist`, `saveMessages`/`loadMessages`),
  api keys (already a secret — moves into the existing
  `secrets.*` RPC namespace once that lands), selected model
  (workspace preference).
- **Stays in the store as UI presentation state only:** `mini`,
  `panelOpen`, `focusSignal`, `pendingPrefill`, `pendingSelections`,
  composer draft, scroll position. None of these survive a
  reload-from-another-shell and that is correct — they are
  presentation, not truth.
- **Stays client-side but is _not_ authoritative:** the in-memory
  `chats` / `seedMessages` maps that hold live `Chat<UIMessage>`
  instances. These are a streaming cache rebuilt from the server's
  message list on hydrate; the server row is the source of truth
  and the UI subscribes to it via `RpcClient.subscribe()`.
- The runner / agent loop continues to drive transport. Tool
  approvals (`approvalResponder`, `respondToApproval`) move with
  the runner — they are responses to live tool calls and live on
  the same RPC channel that streams the call.

Net effect: the zustand store keeps roughly the bottom half of its
current surface (UI presentation + the `approvalResponder` bridge),
and the top half (sessions, messages, keys, model) becomes thin
selectors over `RpcClient`-backed queries. `flushPersist` and
`PERSIST_DEBOUNCE_MS` disappear; debouncing, if still required,
moves into the message-append RPC on the server side.
