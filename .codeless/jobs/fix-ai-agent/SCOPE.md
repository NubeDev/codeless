# Scope — fix-ai-agent

Full design lives in
[`DOCS/ASSISTANT-SCOPE.md`](../../../../DOCS/ASSISTANT-SCOPE.md). Read
it before stage 1 — this brief is the trimmed version, not the
source of truth.

## Goal

Finish the three assistant follow-ups (F2 → F3 → F1) so the
`/assistant` page is functionally complete: the user chats with a
real model, confirms action cards that mutate job state through
real RPCs, and reaches the same conversation from the footer
`AiInputBar` on every tab. SQLite remains the single source of
truth for every chat surface.

## In scope

- **F2 — Planner.** Replace `NOOP_ASSISTANT_REPLY` in
  [`crates/codeless-runtime/src/rpc/assistant.rs`](../../../crates/codeless-runtime/src/rpc/assistant.rs)
  with a real model loop reusing the existing `agent_chat` RPC
  ([`server.rs:247`](../../../crates/codeless-rpc/src/server.rs#L247))
  and the in-editor transport at
  [`cliRunnerTransport.ts`](../../../ui/codeless-ui/src/modules/ai/lib/cliRunnerTransport.ts).
  Tokens stream into the assistant thread through the same event
  channel the in-editor AI chat uses.
- **F3 — Tool dispatch.** Implement the dispatcher in
  `codeless-runtime` keyed on `AssistantActionCard.action`, mapping
  every verb to its `jobs.*` RPC. Add the two missing RPCs:
  `jobs.draftFromConversation` and `jobs.updateScope`. The planner
  emits cards; the dispatcher confirms them.
- **F1 — Footer bar.** Rewire the footer `AiInputBar` (mounted from
  [`App.tsx`](../../../ui/codeless-ui/src/app/App.tsx)) so it drives
  the current assistant thread, not a separate `useChatStore`. One
  source of truth: SQLite + subscription, not a footer-local
  buffer. Full-width cards stay on `/assistant`; the footer shows a
  compact "open in /assistant to confirm" affordance.

## Out of scope

- A new agent runtime. The planner reuses `agent_chat`; no parallel
  runner.
- Per-user permissions, multi-tenant trust boundaries. R5 holds.
- A second chat UI. `CommonChat` is canonical; the footer is a
  composer instance, not a new surface.
- Mobile-specific behaviour beyond what R1–R5 already require.
- Migrating job-chat attachments to a SQLite table. The
  workspace-scoped dir for assistant attachments is already
  decided; job-chat attachments stay where they are.

## Constraints

- **R1** — process spawning lives only in `codeless-adapters-host`.
  The planner reuses `agent_chat`, which already respects this; do
  not add `tokio::process` or `std::process` imports anywhere else.
- **R2** — the assistant page and the footer composer import only
  `RpcClient`. Never `@tauri-apps/api/*`, never `fetch(...)` to the
  codeless server directly.
- **R3** — no per-shell UI files (`.web.tsx`, `.mobile.tsx`). One
  responsive `CommonChat`.
- **R4** — assistant messages, attachments, and action-card state
  live in SQLite. The footer does not maintain its own message
  store; `useChatStore` either becomes UI presentation state only
  (scroll, composer draft) or is retired.
- **R5** — the bearer token authorises every surface identically.
  No per-thread or per-action scopes.
- **Comment hygiene** — explain *why*, never *what*. No
  task-status comments (no "added in F2", "TODO from F3a"). The
  comment must still make sense after the branch merges.
- **No drive-by refactors.** Each follow-up is its own commit
  range. Resist tidying adjacent code unless the stage's outcome
  requires it.
- **No half-finished implementations.** A stage either lands or
  the session doc gets `[!]` and the loop halts.

## Open questions

1. **`agent_chat` cwd for assistant threads.** Assistant threads
   are workspace-scoped — pick the workspace root as cwd in F2 and
   record the choice. If the planner needs a per-thread scratch
   dir, derive it from `<codeless-data>/threads/<thread_id>/`
   (same root the attachments decision uses).
2. **Action-card schema authority.** `AssistantActionCard` is
   currently a UI-side type. F3b must promote it (or its server
   mirror) to `codeless-types` so both ends agree on
   `action`/`status` discriminants. Decide in stage F3a alongside
   the new RPC types — don't let the dispatcher and the UI drift.
3. **Footer thread selection.** Last-used vs explicit pin vs
   per-tab pin. Resolve at the start of F1 and record in
   `DOCS/ASSISTANT-SCOPE.md` §F1; do not invent UX mid-stage.
