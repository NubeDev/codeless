# Plugin substrate (PS2–PS8 + notes plugin)

Branch:      codeless/plugin-substrate
Status file: this file
Spec:        DOCS/PLUGIN-SUBSTRATE.md (items 2–8)
Job dir:     .codeless/jobs/plugin-substrate/
Goal:        a new workflow ships as `crates/codeless-plugin-<id>/` + a
             `plugin.toml` + `domains/`, with `notes` as plugin #0
             exercising items 1–8 end-to-end.

## Stages

1. [!] [M] PS2 — extract CommonChat: JobChat (RunPane.tsx), AiChat, and
   the chat store collapse into one CommonChat component bound to a
   server-resident thread id; assistant page + in-job chat + in-editor
   AI panel all render the same component. **Partial landing: PS2a
   prep sub-stage shipped (see below). PS2b–PS2d still blocked on
   PS3/PS4.**
   1a. [x] PS2a — `CommonChatProps` gains `threadId` on every variant;
       every call site (`RunPane`, `JobPage`, `JobChatPage`,
       `AssistantPage`, `AiMiniWindow`) passes the server-resident
       thread id it already has on hand. New
       `src/modules/chat/CommonChat.test.tsx` pins each kind's
       routing so a future drive-by drop is caught at compile + run
       time. `pnpm typecheck` and `pnpm vitest run` both green.
   1b. [ ] PS2b — see "Recommended re-split" below; requires PS4.
   1c. [ ] PS2c — same; requires PS2b.
   1d. [ ] PS2d — collapse the kind-dispatch facade once b+c land.
2. [ ] PS3 — server-side capability derivation.
3. [ ] PS4 — chat state moves server-side (R4 compliance).
4. [ ] PS5 — persona / thread-kind data model.
5. [ ] PS6 — plugin manifest + registry.
6. [ ] PS7 — tool-result attachments.
7. [ ] PS8 — Assistant agent loop.
8. [ ] Plugin #0 `notes` end-to-end.

## Stage 1 — halt notes

Marked `[!]`. The stage as written cannot be completed by a UI-only
refactor; it presupposes a single server-resident message model that
does not exist yet, and creating one is items PS3 + PS4 — later stages
in this same job.

### What is actually on the floor

- `ui/codeless-ui/src/modules/chat/CommonChat.tsx` is today a
  discriminated-union facade over three completely different
  implementations. Its own comment block (lines 8–22) names the
  blocker: "Until the three surfaces converge on a single message
  model, they cannot share one internal implementation without a
  behaviour-changing rewrite."
- `JobChat` (RunPane.tsx, 3525 lines) loads chat history from
  `CHAT.md` in the job worktree via `read_job_file`, persists turns
  the same way, and subscribes to `EventFilter::Job{...}` for live
  tool calls. There is no SQLite-backed thread for a job chat.
- `AiChatView` (ai/components/AiChat.tsx) is a pure render over an
  `@ai-sdk/react` `useChat` instance held in `chatStore.ts`. State
  is client-side; messages are not persisted server-side.
  `AiMiniWindow.tsx` is the live wrapper that owns the `Chat` object.
- `AssistantThreadView` is the only surface already on the
  SQLite-backed model (`assistant_messages` via `list_assistant_messages`
  + `append_assistant_message`).

### Why this is not a one-stage job

The stage acceptance is: same `CommonChat` component bound to a
server-resident thread id at all three call sites. That requires:

1. A server-resident thread row + message rows for the in-editor AI
   panel (today the `ai-sdk` `Chat` object is the source of truth) —
   PS4.
2. A server-resident thread row + message rows for the in-job chat
   (today `CHAT.md` in the worktree is the source of truth, and the
   runtime treats that file as part of the durable job artifact) —
   PS4 + a migration plan for existing CHAT.md transcripts.
3. A unified post/subscribe RPC surface keyed on `thread_id`, with
   capabilities derived from the thread row (so removing `kind` from
   the props doesn't change what the runner will execute) — PS3.
4. Only then can the UI collapse to one component without losing
   attachments, image paste, tool-call cards, slash commands, the
   spec-mode toggle, the job-ref picker, the worktree-missing banner,
   or the streaming-token accumulator.

A facade-deepening pass that keeps the three transports under the
hood does not satisfy "bound to a server-resident thread id" — the
`ai-panel` transport simply has no such thread today. A
behaviour-preserving full collapse is multi-day work spanning Rust
RPC additions, SQLite schema additions, a CHAT.md migration, and a
UI rewrite of all three call sites. That is the contents of PS2 +
PS3 + PS4 stapled together.

### Recommended re-split

PS2 as a standalone `[M]` is the wrong shape. Two options for the
job author to pick from before the next tick:

- **Option A — reorder.** Land PS4 (server-side chat state) and PS3
  (server-side capability derivation) first; PS2 then becomes a
  mechanical UI collapse on top of an already-unified message model.
- **Option B — re-split PS2 into sub-stages.**
  - PS2a `[M]` — server: add a generic `chat_threads` table + RPC
    surface (`chat.post`, `chat.subscribe`, `chat.list_messages`) and
    back-fill three thread kinds (`job`, `ai-panel`, `assistant`)
    against it.
  - PS2b `[M]` — migrate `JobChat` off `CHAT.md` onto the new RPCs
    (keep `CHAT.md` as a render artifact, not the source of truth).
  - PS2c `[M]` — migrate `AiChatView` / `AiMiniWindow` off the
    client-side `chatStore` `useChat` ownership onto the new RPCs.
  - PS2d `[S]` — collapse the three view components into one
    `CommonChat` bound to `threadId`; delete the facade.

Either option leaves PS3 / PS4 as the small clean-up they were
originally scoped to be, rather than absorbing them silently into a
PS2 rewrite.

### What landed this tick

- Read PLUGIN-SUBSTRATE.md, ASSISTANT-SCOPE.md, the existing
  CommonChat facade, and the three target implementations to confirm
  the data-source split above (CHAT.md vs `useChat` vs
  `assistant_messages`).
- Landed **PS2a** (the safe prep sub-stage of the recommended
  re-split): every `CommonChat` call site now passes a `threadId`
  matching the server-resident id it already had — `job.id` for
  job/`RunPane`/`JobPage`/`JobChatPage`, `thread.id` for
  `AssistantPage`, the editor `sessionId` for `AiMiniWindow`.
  `CommonChatProps` requires the slot on every variant; the routing
  is pinned by a new vitest in
  `src/modules/chat/CommonChat.test.tsx`. Comment on `CommonChat`
  updated to explain why the slot is now load-bearing (PS3 derives
  capabilities from it; PS4 keys state off it).
- `pnpm typecheck` clean; `pnpm vitest run src/modules/chat` green
  (3 / 3 tests).

PS2b–PS2d (the actual collapse of the three implementations) still
needs PS3 + PS4 to land first. The session doc above records the
recommended sequence for the next tick.
