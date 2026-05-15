# Chat for this job

## user @ 2026-05-15T01:46:00Z

confrim you undertsand the spec

## assistant @ 2026-05-15T01:46:12Z

Confirmed. Here's my read of the `assistant` job spec:

**Goal:** Ship an in-app `/assistant` surface that lets the user view/manage/create/edit-scope jobs by chatting — implemented as a thin UI + RPC layer over the existing `codeless-runtime`, not a new agent runtime.

**Shape of work:**
1. **Stage 1** (prose-only) — resolve three open questions in SCOPE.md "Decisions": attachments dir (bias: workspace-scoped `<codeless-data>/threads/<id>/attachments/`), `chatStore.ts` server-state migration shape, and confirming the "no just-do-it" job-creation path. **REVIEW gate** after.
2. **Stage 3** — extract `CommonChat` from `JobChat` (in `RunPane.tsx`) and `AiChat` into `src/modules/chat/`; rewire all three call sites with no behaviour change, one commit per rewire. **REVIEW gate** after for parity.
3. **Stage 5** — add `assistant_threads` + `assistant_messages` tables and `assistant.{listThreads,createThread,deleteThread}` + `uploadAttachment` RPCs. No UI.
4. **Stage 6** — `/assistant` route with thread rail + `CommonChat` + no-op responder.
5. **Stages 7–9** — layer features behind the live route: view/manage action cards (confirmation-required), draft-from-conversation + create, inline scope-diff edits with paused-job rule.

**Load-bearing constraints I'll defend:**
- R2 — `RpcClient` only; no `@tauri-apps/*`, no direct `fetch`.
- R3 — one responsive `CommonChat`, no per-shell forks.
- R4 — SQLite is source of truth; `chatStore.ts` shrinks or dies.
- The `kind` prop on `CommonChat` is UI-only; **capabilities derive server-side from the thread row**, never trusted from the client.

I'm in **spec mode** — I'll edit only files under `.codeless/jobs/assistant/`, no code, no shell, no commits. Ready for spec edits when you are.
