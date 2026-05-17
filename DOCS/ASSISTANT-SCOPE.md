# Assistant — Scope

Status: draft
Owner: ap@nube-io.com
Created: 2026-05-15

## Summary

The **Assistant** is an in-app conversational surface that lets a user
view, manage and create Codeless jobs by talking to the same runner
that powers headless tick execution. It is *not* a second runner; it
is a thin UI + RPC client over `codeless-runtime`. The assistant
reuses the existing job chat UI by extracting it into a shared
`CommonChat` component used by both the in-job chat and the top-level
assistant page.

The job chat today lives as the `JobChat` export in
[`codeless/ui/codeless-ui/src/modules/jobs/RunPane.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/RunPane.tsx)
(rendered both inline from `RunPane` and full-page from
[`JobChatPage.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/JobChatPage.tsx)).
A separate surface, [`modules/ai/components/AiChat.tsx`](../codeless/ui/codeless-ui/src/modules/ai/components/AiChat.tsx)
with [`modules/ai/store/chatStore.ts`](../codeless/ui/codeless-ui/src/modules/ai/store/chatStore.ts),
predates the job chat and is used by the in-editor AI panel. The
extraction below folds **both** into `CommonChat`; see §6.

Out of scope for this doc: the agent itself (planner, tools, prompts),
auth/multi-tenant, mobile-specific behaviour beyond what R1–R5 in
[`CLAUDE.md`](../CLAUDE.md) already require.

## Goals

1. One place a user can:
   - List, filter and inspect jobs.
   - Start / stop / pause / resume / restart jobs.
   - Edit a job's scope (the markdown brief the runner consumes).
   - Create new jobs from a conversation (intent → draft → confirm).
2. Reuse the job chat as a `CommonChat` component — same look, same
   attachments, same image upload, same streaming behaviour — in:
   - The assistant page.
   - The in-job chat (`JobChat` exported from
     [`RunPane.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/RunPane.tsx),
     rendered inline by `RunPane` and full-page by
     [`JobChatPage.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/JobChatPage.tsx)).
   - The in-editor AI panel
     ([`AiChat.tsx`](../codeless/ui/codeless-ui/src/modules/ai/components/AiChat.tsx)).
3. All transport flows through `RpcClient` (R2) — no Tauri or `fetch`
   leaks into the UI.
4. SQLite remains the source of truth (R4) — assistant conversations,
   attachments and any drafts are persisted server-side and replayed
   via `RpcClient.subscribe()`.

## Non-goals

- A new agent runtime. The assistant calls into the existing runner
  (planner + tools) via `RpcClient`.
- Per-user permissions. Single-tenant trust boundary (R5) holds.
- A second chat UI. If `CommonChat` can't express something a caller
  needs, the caller extends via props/slots — it does not fork.

## Surfaces

### 1. Assistant page

Route: `/assistant` (sibling of `/jobs`).

Layout:

- Left rail: thread list (recent assistant conversations).
- Main: `CommonChat` instance bound to the selected assistant thread.
- Right rail (collapsible): "context panel" showing what the
  assistant is currently looking at — selected job, draft scope, etc.
  — so the user can see what state actions will mutate.

The assistant page is a *conversation* first; structured actions
(start/stop/edit scope) appear inline as message-attached action cards
that the user confirms before the runner executes them. No silent
side-effects from chat.

### 2. CommonChat (extracted shared component)

Lives at `codeless/ui/codeless-ui/src/modules/chat/` (new module).

Responsibilities:

- Render message list (user / assistant / tool / system roles).
- Streaming token rendering with the existing event format
  ([`eventFormat.ts`](../codeless/ui/codeless-ui/src/modules/jobs/eventFormat.ts)).
- Composer with:
  - Multiline text input.
  - **Attachments**: drag-drop + file picker, same set as job chat
    (images: png/jpg/jpeg/gif/webp; text/code; pdf). Attachments
    upload via `RpcClient.uploadAttachment(threadId, file)` and are
    referenced by id in the outgoing message.
  - **Image paste** from clipboard.
  - Send / stop-stream button.
- Tool-call cards (approval, diff review, todo strip) — reused from
  `modules/ai/components/`.
- Scroll anchoring, "jump to latest", unread divider.

Props (sketch):

```ts
interface CommonChatProps {
  threadId: string;
  client: RpcClient;
  // What kind of thread this is — drives which action cards render,
  // which tools the runner is allowed to call, and the placeholder text.
  kind: "assistant" | "job-overview" | "ai-panel";
  // Optional slots: header, right-rail context, empty state.
  slots?: { header?: ReactNode; contextPanel?: ReactNode };
}
```

`CommonChat` owns no business logic about jobs. Callers feed it a
`threadId`; capabilities are derived **server-side** from the thread
row (`assistant_threads.kind` / equivalent on job threads), not
trusted from the client. The `kind` prop is UI-only — it picks
placeholder copy, header affordances, and which action-card
renderers to mount. A malicious client passing the wrong `kind`
must not be able to invoke tools the runner wouldn't otherwise
allow on that thread.

### 3. Existing chat call sites

- `JobChat` in [`RunPane.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/RunPane.tsx)
  becomes a thin wrapper that resolves the job's `threadId` and
  renders `<CommonChat kind="job-overview" .../>` with a job-aware
  header slot. The inline render from `RunPane` and the full-page
  render from [`JobChatPage.tsx`](../codeless/ui/codeless-ui/src/modules/jobs/JobChatPage.tsx)
  both go through this wrapper.
- [`AiChat.tsx`](../codeless/ui/codeless-ui/src/modules/ai/components/AiChat.tsx)
  becomes a thin wrapper for `kind="ai-panel"`. Its current
  client-side state in
  [`chatStore.ts`](../codeless/ui/codeless-ui/src/modules/ai/store/chatStore.ts)
  must move server-side to satisfy R4; the store either disappears
  or shrinks to UI presentation state (scroll position, composer
  draft) only.

No behaviour change for existing users — same attachments, same
streaming, same tool cards. The extraction is mechanical.

## Capabilities (what the assistant can do)

The assistant exposes these as runner-side tools. Each tool call
surfaces in the chat as an action card the user confirms before the
runner mutates state.

### View / manage

| Capability | RPC method                                  | Confirmation | Status |
|------------|---------------------------------------------|--------------|--------|
| List jobs  | `list_jobs({ filter })`                     | none (read)  | exists |
| Inspect    | `get_job(jobId)`                            | none (read)  | exists |
| Start      | `start_job(jobId)`                          | confirm: job id + entry stage              | exists |
| Stop       | `stop_job(jobId, { reason })`               | confirm: reason text                       | exists |
| Pause      | `pause_job(jobId)`                          | confirm: job id + current stage            | exists |
| Resume     | `resume_job(jobId)`                         | confirm: job id + resume point             | exists |
| Restart    | `rerun_job(jobId, { fromStage? })`          | confirm: stages that will be discarded     | exists |
| Update     | `update_job(jobId, { fields })`             | confirm: per-field diff of changed fields  | exists |
| Draft      | `jobs.draftFromConversation(threadId)`      | confirm: full `JobSpec` diff               | **new (F3)** |
| Edit scope | `jobs.updateScope(jobId, newMarkdown)`      | confirm: unified diff against current scope | **new (F3)** |

Existing methods live in
[`crates/codeless-rpc/src/server.rs`](../codeless/crates/codeless-rpc/src/server.rs);
the assistant does not invent its own job model. The two **new**
rows are scoped into F3 (tool dispatch) below.

### Create

`jobs.draftFromConversation(threadId) → DraftJob` returns the
proposed `JobSpec` (repo, branch, scope markdown, runner choice).
The user reviews the draft inline (a diff-style card) and confirms
via `jobs.create(draft)`. No job is created until the user clicks
Confirm.

### Edit scope

The scope is a markdown document
([`DOCS/JOB-DIR.md`](./JOB-DIR.md) defines the layout). The
assistant offers two edit modes:

1. **Inline edit** — the assistant proposes a unified diff against
   the current scope; the user accepts/rejects hunks in a
   `PlanDiffReview`-style card; accepted hunks are written via
   `jobs.updateScope(jobId, newMarkdown)`.
2. **Open in editor** — opens the scope file in the existing editor
   tab. No assistant mediation; same `RpcClient` write path.

Editing scope on a *running* job follows existing job-state rules:
the job must be paused before scope can be written, and the runner
re-reads scope on resume. The assistant surfaces the pause as an
action card if the user requests a scope edit on a running job; it
does not bypass the rule.

## Data model additions

- `assistant_threads` table: `id`, `created_at`, `title`, `context`
  (json: selected job id, draft id, etc.).
- `assistant_messages` table: `id`, `thread_id`, `role`, `content`,
  `tool_calls` (json), `attachments` (json: array of attachment refs),
  `created_at`.

### Attachments — current state and required change

Job chat attachments are **filesystem-backed, not table-backed**.
The runner writes uploads under the job's worktree at
`.codeless/chat-attachments/` (see
[`crates/codeless-runtime/src/rpc/chat.rs`](../codeless/crates/codeless-runtime/src/rpc/chat.rs)
around line 182), and outgoing messages carry the attachment refs
inline. There is no `attachments` table to reuse.

Assistant threads are workspace-scoped, not job-scoped, so they
have no worktree to write into.

**Decided: workspace-scoped attachments dir** at
`<codeless-data>/threads/<thread_id>/attachments/`. Job chat keeps
writing to the worktree; assistant chat writes to the workspace
dir. `RpcClient.uploadAttachment(threadId, file)` has one shape;
the runner picks the directory from the thread's kind. Strictly
additive — no migration of job-chat attachments, no new SQLite
attachments table. If we later want one path forever, that's a
separate decision; today we have two.

## RPC additions

Extend `RpcClient`:

```ts
interface RpcClient {
  // existing methods...
  assistant: {
    listThreads(): Promise<AssistantThread[]>;
    createThread(title?: string): Promise<AssistantThread>;
    deleteThread(id: string): Promise<void>;
  };
  uploadAttachment(threadId: string, file: File): Promise<Attachment>;
}
```

`messages` and `subscribe` are already generic over thread id; the
assistant uses the same channels as job chat. This is what makes
`CommonChat` actually shareable.

## Cross-cutting rules (must hold)

- **R1**: nothing the assistant does spawns processes from the UI.
  All `process::Command` use stays in `codeless-adapters-host`.
- **R2**: only `RpcClient`. The assistant page does not import
  `@tauri-apps/api` or call `fetch` directly.
- **R3**: no `Assistant.web.tsx` / `Assistant.mobile.tsx`. One
  responsive component.
- **R4**: assistant threads, messages, attachments and drafts live
  in SQLite. The UI subscribes; it does not cache authoritative
  state.
- **R5**: single-tenant trust boundary. The bearer token authorises
  the assistant identically to every other client; no per-job or
  per-action scopes are introduced.

## Open questions

- Should the assistant be allowed to *create* a job without a draft
  confirmation step if the user explicitly says "just do it"? Bias:
  no — confirmation is cheap and prevents irreversible spend on the
  wrong scope.
- Image rendering parity: job chat renders images inline in
  messages; confirm the same path works for assistant threads
  before treating it as "free" via the `CommonChat` extraction.

## Milestones

Completed in master as of 2026-05-15:

- [x] M1 — Attachments-directory decided (workspace-scoped dir, §Data
  model); `AiChat` server-state migration landed (§Surfaces 3).
- [x] M2 — `CommonChat` extracted; `RunPane`, `JobChatPage`, AI panel
  rewired; visual + behavioural parity verified.
- [x] M3 — `assistant.*` RPCs + tables shipped; `/assistant` page
  lists / creates threads; no-op responder at
  [`assistant.rs:182`](../codeless/crates/codeless-runtime/src/rpc/assistant.rs#L182).

Remaining work lives in §Follow-ups, ordered by dependency:
**F2 (planner) → F3 (tool dispatch + new `jobs.*` RPCs) → F1 (footer
bar)**. Each ships behind the same UI route; partial completion is
visible but gated by feature presence, not feature flags.

## Status (as of 2026-05-15)

F2, F3, F1 all landed on `codeless/fix-ai-agent` (commits `183635a`,
`a8f3bda` / `772ddb9` / `17a74d2`, `50cdc4a`). End state:

- The planner runs `agent_chat` for every non-slash message; the
  no-op responder is gone (assistant.rs no longer references
  `NOOP_ASSISTANT_REPLY`). Slash commands still take the
  deterministic `parse_action` fast-path.
- Planner tool calls are persisted as their own assistant-role rows
  carrying `meta_json: AssistantActionCard`, surfaced through
  `AppendAssistantMessageResult.cards`, and rendered by
  `AssistantThreadView` without UI gymnastics.
- `jobs.draftFromConversation(threadId)` and
  `jobs.updateScope(jobId, newMarkdown)` are first-class methods on
  `RpcServer`; the paused-job rule lives in `update_scope`'s
  implementation so every caller honours it.
- `dispatch_action` routes confirmed `draft_job` / `edit_scope`
  cards through the new methods; the original inline
  `submit_job` / `write_job_file` calls are gone from that
  dispatcher.
- The footer `AssistantFooterBar` binds to the current assistant
  thread via `useAssistantFocus`; footer submits land in the same
  SQLite-backed transcript as `/assistant`. `useChatStore` is
  documented as shrunken to UI-presentation slots (the in-editor
  `AiMiniWindow` keeps it for its SDK transport — folding the
  mini-window into the assistant is a separate follow-up the spec
  already flags).

Follow-ups below are kept for posterity; the per-section
"Completed" markers record where the work landed.

## Follow-ups

### F2 — Wire the planner (replace the no-op responder) — **Completed (183635a, 17a74d2)**

Replace `NOOP_ASSISTANT_REPLY` in
[`assistant.rs:182`](../codeless/crates/codeless-runtime/src/rpc/assistant.rs#L182)
with a real model loop. **Reuse the existing AI-panel transport**
([`cliRunnerTransport.ts`](../codeless/ui/codeless-ui/src/modules/ai/lib/cliRunnerTransport.ts)
and the `agent_chat` RPC at
[`server.rs:247`](../codeless/crates/codeless-rpc/src/server.rs#L247)
that already drives the in-editor chat) so we don't ship a parallel
runner.

Requirements:

- Plumb assistant-thread messages into `agent_chat` with the thread's
  history, returning streamed assistant tokens via the same event
  channel the in-editor AI chat uses.
- Tool-call output from the model is shaped as an
  `AssistantActionCard` and persisted on the message's `meta_json`
  so [`AssistantThreadView.tsx:227`](../codeless/ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx#L227)
  renders it. No new UI work — the card surface already exists.
- The runner-side tool *registry* lands in F3; for F2, emitting cards
  is enough — confirming them is allowed to no-op.
- `agent_chat`'s cwd / registry config (see [`chat.rs:34`](../codeless/crates/codeless-runtime/src/rpc/chat.rs#L34))
  must be set for assistant threads; pick the workspace root as cwd
  since threads are workspace-scoped.

Blocks: F3, F1.

### F3 — Tool dispatch + missing `jobs.*` RPCs — **Completed (a8f3bda, 772ddb9)**

The UI confirms action cards via `confirm_assistant_action` at
[`AssistantThreadView.tsx:112`](../codeless/ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx#L112).
The server-side dispatcher needs to turn a confirmed card into the
right `jobs.*` RPC call.

Requirements:

- Implement the dispatcher in `crates/codeless-runtime/src/rpc/assistant.rs`
  (or a sibling module) keyed on `AssistantActionCard.action`. Map:
  `start` → `start_job`, `stop` → `stop_job`, `pause` → `pause_job`,
  `resume` → `resume_job`, `restart` → `rerun_job`, `update` →
  `update_job`, `draft_job` → `jobs.draftFromConversation` (new),
  `edit_scope` → `jobs.updateScope` (new).
- Add **`jobs.draftFromConversation(threadId) → DraftJob`** and
  **`jobs.updateScope(jobId, newMarkdown)`** to `codeless-rpc` and
  `codeless-runtime`. `updateScope` must respect the paused-job rule
  (§Edit scope) — reject with a typed error on a running job; the
  UI surfaces that as the "pause first" affordance.
- Tests: per-tool dispatch test using `MockRunner`; reject path for
  scope-edit-on-running-job; round-trip test that a confirmed
  `start` card calls `start_job` exactly once.

Depends on: F2 (planner has to emit cards before dispatch is
exercised end-to-end, though dispatch can be tested in isolation).

### F1 — Drive the footer AI bar with the assistant — **Completed (50cdc4a)**

The existing footer composer (`AiInputBar`, mounted from
[`App.tsx`](../codeless/ui/codeless-ui/src/app/App.tsx) under the
`panelOpen` motion section) is currently bound to the in-editor AI
chat (`useChatStore`). Once the planner lands, the footer bar should
be the *primary* assistant entry point so the user has the assistant
available from every tab — not just by switching to the `/assistant`
tab.

Requirements:

- Same composer, same affordances (attachments, image paste,
  tool-call cards), driven by `CommonChat`'s underlying message
  model so there's no third chat surface to maintain.
- The footer bar should target the **current assistant thread**
  (last-used or pinned) by default; an explicit "new thread" button
  resets it. Switching to the `/assistant` tab and selecting a
  different thread updates what the footer bar is bound to.
- Messages sent from the footer must appear in the `/assistant`
  thread's transcript on the next render — there is one source of
  truth (SQLite + subscription), not a footer-local buffer.
- Action cards that need full-width rendering (diff review, draft
  review) should still surface on the `/assistant` tab; the footer
  shows a compact "open in /assistant to confirm →" affordance and
  the user clicks through. The footer is for chat input + short
  responses, not for full-screen review.
- `useChatStore` either becomes the assistant's UI presentation
  store (scroll, composer draft) and loses its message ownership,
  or is retired in favour of `CommonChat`'s state. Either way: the
  in-editor AI chat surface and the assistant become the same
  thing, accessible from two places.

Depends on: F2 (without a real responder there's nothing useful to
expose globally) and F3 (action cards need to actually mutate state
before a "open in /assistant to confirm" hand-off makes sense).
