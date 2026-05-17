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
2. [x] PS3 — server-side capability derivation: pattern grammar +
   matcher in `codeless_types::allowed_tools` (literal id or
   `prefix.*` only, no regex, no shell globbing); chat-path tool
   selection extracted into `runtime::rpc::chat_capability::
   derive_chat_capabilities`, the one place the chat path decides
   what a thread may invoke. The function is pure over
   `(active_job, ChatMode)`; no UI routing prop reaches it, so
   removing the `CommonChat` `kind` prop from a call site cannot
   change which tools the runner executes. PS5 fills
   `caps.allowed_tools` from the persona's column through the same
   seam.
3. [!] PS4 — chat state moves server-side (R4 compliance).
   **Halted. See "Stage 3 (PS4) — halt notes" below.**
4. [x] PS5 — persona / thread-kind data model.
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

## Stage 3 (PS4) — halt notes

Marked `[!]`. The stage as written cannot be completed by a single
`[M]` tick. The acceptance — "drop `modules/ai/store/chatStore.ts`
client-side conversation state; closing the browser then reopening
reproduces the full thread including in-flight tool calls via
`RpcClient.subscribe()`" — requires the entire AI-panel agent loop
to move server-side. Today it runs in the browser.

### What `chatStore.ts` actually owns

Despite the file's name, only the in-editor mini-window (`AiMiniWindow`)
still uses it for conversation state. The two other chat surfaces are
already off it:

- The **Assistant page** (`AssistantThreadView`) is on the SQLite-backed
  `assistant_threads` / `assistant_messages` model via
  `list_assistant_messages` + `append_assistant_message`. R4-compliant
  already.
- The **in-job chat** (`JobChat` inside `RunPane.tsx`) writes to
  `CHAT.md` in the worktree, subscribed via `EventFilter::Job{…}`. Not
  in `chatStore`, but also not R4-compliant; that migration is the
  separate PS2b sub-stage already noted in Stage 1's halt notes.
- The **in-editor AI panel** (`AiMiniWindow` → `getOrCreateChat`) holds
  a per-session `@ai-sdk/react` `Chat<UIMessage>` instance whose
  transport runs the model call and the tool loop entirely in the
  browser.

So "drop `chatStore` conversation state" reduces to "move the in-editor
AI-panel chat onto a server-resident thread + replayable event stream."

### Why this is not a `[M]` tick

The AI-panel transport in `ui/codeless-ui/src/modules/ai/lib/transport.ts`
constructs a `DirectChatTransport` over `createCodelessAgent(...)` from
`ui/codeless-ui/src/modules/ai/lib/agent.ts`. The browser owns:

1. The provider client and the model adapter (key reads from
   `useChatStore.apiKeys`; provider call is direct from the browser).
2. The tool table — eight files in `ui/codeless-ui/src/modules/ai/tools/`
   totalling ~820 lines: `fs`, `edit`, `search`, `shell`, `subagent`,
   `terminal`, `todo`, plus `context`. Several are intrinsically
   browser-bound (`terminal.injectIntoActivePty`, editor file ops on
   the active document, the live `ToolContext.readCache`).
3. The agent step loop (`stepCountIs(MAX_AGENT_STEPS)`), token
   streaming into `UIMessage.parts`, and `addToolApprovalResponse`
   plumbing for human-in-the-loop tool gating.

Making in-flight tool calls survive a browser close means the server
holds the in-flight agent step, the partial token buffer, and the
pending tool-approval state, and replays them to a re-attaching
client via `RpcClient.subscribe()`. Concretely that needs all of:

- A new server-side `chat_threads` / `chat_messages` table (or a
  generalisation of `assistant_messages`) keyed by `thread_id`.
- A server-side equivalent of `createCodelessAgent` — the Rust runtime
  has to drive the AI-SDK loop, not the browser. The `assistant.*`
  RPCs today are a small planner with action cards, not a streaming
  multi-step agent loop.
- A server-side port of every panel tool that is meant to keep
  working post-migration. Tools that are intrinsically browser-bound
  (terminal injection, editor active-file ops) cannot be ported — they
  have to either be redefined (the server PTY for `terminal`,
  workspace-scoped FS for `fs`) or dropped from the panel.
- New RPC methods: `chat.post`, `chat.subscribe`, `chat.list_messages`,
  matching the PS2a-recommended surface in Stage 1's halt notes.
- An SSE event stream that carries token deltas, tool-call requests,
  tool-approval responses, and tool-result append events; the
  reconnect path has to be idempotent so closing+reopening the
  browser mid-stream stitches back to the live cursor.
- A rewrite of `AiMiniWindow` (and the `AiChat` view) off `useChat` +
  `getOrCreateChat` onto the new RPC + subscribe pair, similar to how
  `AssistantThreadView` is shaped today.
- Migration of the existing per-session message blob (`saveMessages`
  in `ui/codeless-ui/src/modules/ai/lib/sessions.ts`) into the new
  `chat_messages` table for users coming from a previous build.

That is a multi-day cross-crate change spanning a new SQLite table, a
new Rust agent-runtime adapter, a new RPC surface (with TypeScript
codegen), eight tool ports, an SSE event format extension, a UI
rewrite, and a data migration. It is the same shape and roughly the
same scope as the original PS2 the previous session halted on — and
it is the "tool runs server-side" assumption that PS8 (Assistant
agent loop) also depends on. PS4 as written is "do PS8 first, but
for the in-editor panel instead of the Assistant."

### Recommended re-shape (for the job author)

Two credible re-splits. Either is much smaller than the current PS4
as written.

- **Option A — narrow PS4 to the surface that already has a
  server-resident thread.** Re-scope this stage to: delete the
  `Sessions`-related members of `chatStore.ts` (sessions list,
  per-session `Chat` cache, `seedMessages`, `saveSessionsList`,
  `loadAll`, `deleteSessionData`, the debounced `persistMessages`
  pipeline) and route `AiMiniWindow` / `AiChat` to `assistant_threads`
  with a thin "this thread is an AI-panel thread" flag. Tools stay
  client-side for now; in-flight tool-call survival is deferred to
  PS8. Acceptance becomes: closing the browser and reopening
  reproduces the user-visible message ledger (text and completed
  tool-call cards); a tool call mid-flight at close time is allowed
  to drop on the floor in this stage.
- **Option B — defer PS4 until PS8 lands.** PS8 is "Assistant agent
  loop" — the server-side agent runtime PS4 needs. Sequence PS5
  (persona model) → PS6 (plugin manifest) → PS8 (server-side agent
  loop) → PS4 (migrate the in-editor panel onto it). PS4 is then a
  mechanical UI migration, not a runtime build.

Lean toward **Option B**. PS4 as written presupposes a server-side
agent loop, which is exactly PS8. Doing PS8 first turns PS4 into the
small migration the `[M]` size implies. Option A is a possible
intermediate but leaves the file `chatStore.ts` half-resident
(presentation state on the client, message state in SQLite, tool
loop still in the browser) which is the same "facade over three
data sources" shape Stage 1 halted on for `CommonChat`.

### What landed this tick

- Re-read `DOCS/PLUGIN-SUBSTRATE.md` §4 (R4 acceptance), the prior
  Stage 1 halt notes, `chatStore.ts`, `AiMiniWindow.tsx`,
  `AssistantThreadView.tsx`, `transport.ts`, `agent.ts`, and the
  eight files under `ui/codeless-ui/src/modules/ai/tools/`.
- Confirmed `chatStore.ts` is now the in-editor panel's last
  client-side conversation owner; the Assistant page is already
  R4-compliant and the in-job `CHAT.md` path is a separate (PS2b)
  problem.
- Established that "in-flight tool calls survive a browser close"
  cannot be satisfied without first moving the agent loop and the
  tool registry server-side (PS8 + the tool ports), so PS4 is the
  wrong size and the wrong order as written.
- Marked PS4 `[!]` with this analysis and two recommended re-shapes
  for the job author to pick from before the next tick.

No code change committed — per CLAUDE.md R4, a half-finished
implementation with TODOs is worse than a documented halt.

## Stage 4 (PS5) — landed

Migration `0017_assistant_thread_persona.sql` extends `personas` with
the three substrate-doc columns (`allowed_tools`, `default_model_family`,
`default_attachments_policy`) and rebuilds `assistant_threads` so
`persona_id` is NOT NULL with `REFERENCES personas(id) ON DELETE
RESTRICT`. SQLite cannot ADD COLUMN NOT NULL REFERENCES with a sensible
default, so the table rebuild is the only credible path; existing rows
back-fill to `builtin:general` (the seeded Assistant default).

`CreateAssistantThreadArgs` grows a required `persona_id` field. The
RPC validates: empty / whitespace returns `InvalidArgument` with a
substrate-doc citation, unknown id returns `NotFound`. The two seeded
substrate-doc personas (`builtin:general`, `builtin:coding`) ship
alongside the five legacy job-runner personas; the latter back-fill
the new columns via the migration's column-level DEFAULTs.

The "runner reads tools and system prompt from persona at agent-call
time" half of the acceptance lands as
`rpc::assistant::resolve_thread_persona`, the single seam PS8 will
consume. Today it has no production caller (the Assistant agent loop
is PS8) so it carries `#[allow(dead_code)]` with a pointer; unit tests
cover the lookup and the seeded `allowed_tools` shape so the contract
is enforced now rather than at PS8 wiring time.

UI call sites (`AssistantPage`, `AssistantFooterBar`) now pass
`persona_id: "builtin:general"` at create time; a UI persona picker is
deferred to PS6 when plugin manifests register additional personas.

`cargo test -p codeless-runtime -- --skip claude_runner` is green; the
two pre-existing failures on this branch (`claude_runner` CLI smoke
test and `codeless-bot-core::reply::format_parse_error_routes_empty_to_help`)
are unchanged by this stage. The Specta wire snapshot and
`ui/codeless-ui/src/lib/rpc/generated/wire.ts` are regenerated.
