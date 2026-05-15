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

_(empty — stage 1 fills this in)_
