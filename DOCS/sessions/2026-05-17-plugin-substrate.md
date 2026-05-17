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
5. [x] PS6 — plugin manifest + registry.
6. [x] PS7 — tool-result attachments.
7. [x] PS8 — Assistant agent loop.
8. [x] Plugin #0 `notes` end-to-end (PS-NOTES — manifest + registration
   + smoke test in-tree).
9. [x] PS-ACCEPT — integration-test coverage for items 2-8 + notes
   plugin end-to-end test + Acceptance section updated.

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

Migration `0019_assistant_thread_persona.sql` extends `personas` with
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

## Stage 5 (PS6) — landed

The substrate's plugin loader lives under
`crates/codeless-tools/src/plugin/` with four submodules:

- `manifest.rs` — `plugin.toml` parser. `deny_unknown_fields` on every
  table; `plugin.id` is restricted to `^[a-z_][a-z0-9_]*$` so the
  table-name-prefix check in `migrations.rs` cannot be tricked by
  exotic identifiers; `personas[].allowed_tools` is validated through
  the existing `codeless_types::allowed_tools::validate_patterns`
  matcher (PS3) so syntax skew between plugin authoring and runtime
  enforcement is impossible by construction;
  `default_model_family` is rejected if not one of the known aliases
  (`fast`/`smart`/`reasoning`) — the substrate-doc rule against
  hardcoded provider model ids enforced at load time, not first use.
- `migrations.rs` — static SQL prefix check. Strips line + block
  comments, splits statements on `;` outside of `'..'`/`"..."`/`` `..` ``
  quoted regions, tokenises just enough to spot
  `CREATE TABLE [IF NOT EXISTS]`, `ALTER TABLE`,
  `DROP TABLE [IF EXISTS]`, plus the matching INDEX/TRIGGER/VIEW
  forms, and rejects any whose target name lacks the `<plugin_id>_`
  prefix (substrate-doc OQ-PS-4). Schema-qualified targets
  (`main.personas`) and quoted targets normalise before the check so
  `"personas"` and ``main.`personas` `` both fail. `INSERT` / `PRAGMA`
  are left alone — plugin seed migrations need them.
- `model_family.rs` — codeless-side alias resolver. Built-in defaults
  cover the three known aliases against current Anthropic tiers; an
  operator overlays via a TOML `[model_families]` table loaded from
  `$CODELESS_CONFIG` (or any explicit `--config` path). The resolver
  refuses to introduce unknown aliases — the alias set is the single
  source of truth shared with the manifest validator.
- `registry.rs` — `PluginRegistry::load_plugin(path, &RegistrationTable)`.
  Plugins register via a `PluginToolSink` (staged Vec, collision
  check against the host's `ToolRegistry`, then atomic merge) so a
  plugin colliding on one of its tool ids does not leave a partial
  registration behind. Manifest personas with bare ids are prefixed
  `<plugin_id>:` on insertion; namespaced personas (`<plugin>:<slug>`,
  `builtin:<slug>`) pass through. The applied migrations are
  returned as a `Vec<PluginMigration>` for the runtime to feed sqlx
  -- keeping the SQL apply out of `codeless-tools` is what lets the
  plugin layer compile without a sqlx dep.

The CLI surfaces are `codeless plugin list` and `codeless plugin info
<id>`, both backed by the registry. The host-binary registration
table lives in `crates/codeless-cli/src/plugin.rs::host_registration_table`
and is empty in this commit -- plugin #0 (`notes`) will insert its
`notes_register` here in a follow-up stage. `list` therefore reports
discovered manifests as "no registration entry"; once the entry
lands the same row flips to "loaded" with the tool count.

Tests pin every seam:

- 5 manifest unit tests (substrate-doc shape, uppercase id rejection,
  unknown model family rejection, regex-in-allowed_tools rejection,
  duplicate persona, unknown top-level field).
- 13 migration unit tests (accept prefixed, reject unprefixed CREATE
  / ALTER / DROP, IF EXISTS / IF NOT EXISTS, UNIQUE INDEX,
  schema-qualified, quoted, line + block comments, semicolons inside
  strings).
- 5 model family unit tests (known aliases round-trip, defaults
  cover every family, override layering, unknown alias rejection,
  empty model rejection).
- 5 registry unit tests (end-to-end load, missing registration is an
  error, codeless-table migration rejected, duplicate plugin id,
  duplicate tool id).
- 2 in-tree `plugin_smoke.rs` tests building a `notes`-shaped tempdir
  end-to-end through `PluginRegistry`.
- 3 CLI integration tests in `crates/codeless-cli/tests/plugin_cli.rs`
  (list with a discovered manifest, info errors when the registry is
  empty, list reports the "no plugins compiled in" path).

`cargo test -p codeless-tools -p codeless-cli` green;
`cargo clippy -p codeless-tools --all-targets -- -D warnings` clean
(the pre-existing `codeless-bot-core::dispatcher::manual_range_contains`
warning on this branch is unchanged by this stage); `cargo fmt
--check` clean.

`cargo test -p codeless-runtime -- --skip claude_runner` is green; the
two pre-existing failures on this branch (`claude_runner` CLI smoke
test and `codeless-bot-core::reply::format_parse_error_routes_empty_to_help`)
are unchanged by this stage. The Specta wire snapshot and
`ui/codeless-ui/src/lib/rpc/generated/wire.ts` are regenerated.

## Stage 6 (PS7) — landed

The substrate's tool-result attachment contract (DOCS/PLUGIN-SUBSTRATE.md
item 7) lands as three composable seams plus the renderer that consumes
them:

- **Wire shape**: `codeless_types::AttachmentRef { attachment_id, mime?,
  filename? }` is what a tool returns; `AssistantAttachmentCard` (kind
  `attachment_card`) + `AssistantAttachmentCardItem` is what the
  reconciled card looks like on `assistant_messages.meta_json`. Lives
  next to `AssistantActionCard` so both meta-kind variants share the
  same `kind` discriminator namespace and the renderer's parser stays a
  single switch.
- **Schema marker + walker**: `codeless_tools::attachment` exports
  `ATTACHMENT_SCHEMA_REF = "codeless://attachment"`,
  `attachment_ref_schema()` / `attachment_array_schema()` for plugin
  authors, and `find_attachment_refs(schema, value)`. The walker
  understands four shapes (root single, root array, named-property
  single, named-property array); anything richer is intentionally
  unmatched -- the substrate doc limits the contract to those shapes so
  the renderer cannot be tricked into recursing into arbitrary plugin
  output. `Tool::output_schema()` joins `Tool::schema()` with a default
  returning the empty object, so plugin authors opt into attachment
  output by overriding one method on their tool impl.
- **Reconciliation**: `codeless_runtime::rpc::attachment::
  build_attachment_card` is the one place that goes value -> refs ->
  rows -> card. Stored row wins for `filename`/`mime`/`size_bytes`;
  tool-supplied hints that disagree are silently dropped (the doc rule).
  Cross-thread ids are rejected as `InvalidArgument`; dangling ids as
  `NotFound`. PS8 will call this once per resolved tool call -- the
  function is `pub` and the module is `#![allow(dead_code)]` until then
  (same pattern as `resolve_thread_persona` in PS5). Reconciliation is
  centralised here so a future PS8 turn cannot accidentally skip the
  store check: the only way to mint an attachment card is through this
  function.
- **Store accessor**: `SqliteStore::get_assistant_attachment(id)`. The
  list accessor already existed; the by-id accessor was the missing
  piece the reconciler needs.

UI side: `AssistantThreadView` gains an `AttachmentCardView` plus the
matching `parseAttachmentCard` discriminator, slotted in front of the
generic `ToolResultView` so a tool whose result decoded to an
`attachment_card` renders the file list (name + mime + size, one row
per item) instead of a raw-JSON payload. The HTTP download route is
deferred -- the runtime exposes no `assistant_attachments/<id>`
endpoint yet, so the card documents the file the tool produced and
leaves a future PS8 / notes-plugin tick to wire the link.

Wire types added to both specta snapshots (`codeless-types` for the
core shapes, the `codeless-rpc` `wire_ts` example for the UI bundle)
and the UI's `methods.ts` re-exports the three new names so consumers
import from `@/lib/rpc` like every other wire type.

`cargo test -p codeless-tools` (71 passed including 9 new attachment
tests); `cargo test -p codeless-runtime --lib rpc::attachment` (5 new
tests covering store reconciliation, cross-thread rejection, unknown
id, array shape, and empty schema); `cargo test -p codeless-types
--test specta_snapshot` green after regen. `cargo clippy -p
codeless-tools -p codeless-types -p codeless-runtime --all-targets --
-D warnings` clean. `cargo fmt --all -- --check` clean. UI `pnpm
typecheck` and `pnpm vitest run src/modules/chat` green.

The pre-existing `claude_runner` integration-test failure on this
branch is unchanged by this stage.

## Stage 7 (PS8) — landed

The Assistant agent loop now binds the planner to the thread's persona
(DOCS/PLUGIN-SUBSTRATE.md item 8): the persona's `instructions` column
becomes the system prompt and the `allowed_tools` column caps which
built-in actions the planner advertises and accepts.

- `assistant_planner::run_planner_turn` grows a `persona: &Persona`
  parameter. The hard-coded `PLANNER_SYSTEM_PREAMBLE` splits into
  `PLANNER_FRAMING_PREAMBLE` (one framing line everyone shares: "you
  are the Codeless Assistant; tool calls are confirmable cards") plus
  a `## Persona` block carrying `persona.instructions`. The tool
  trailer is rebuilt per turn from the built-in catalogue
  (`BUILTIN_ASSISTANT_TOOLS`) filtered through
  `codeless_types::allowed_tools::tool_allowed` against the persona's
  list. An empty surviving catalogue swaps to a "no tool grants;
  reply in prose" trailer rather than advertising tools the runner
  would drop.
- Incoming `Event::ToolCall` envelopes are filtered through the same
  matcher in the publish closure: `assistant_tool_id` namespaces the
  built-in catalogue under `assistant.<verb>` and passes plugin tool
  names (`notes.append`, ...) through unchanged, so one
  `tool_allowed` check covers both worlds. A disallowed tool is
  logged and dropped -- surrounding prose still lands so the user
  sees the model's explanation.
- `assistant::append_assistant_message` resolves the thread's persona
  via the existing PS5 seam (`resolve_thread_persona`, the
  `#[allow(dead_code)]` marker is removed since the production
  caller now exists) and passes it into the planner. Built-in
  action dispatch on confirm is unchanged -- the cap is at emit
  time, not at execute time, so a card the user has already seen
  always runs.
- Migration `0020_assistant_persona_builtin_tools.sql` updates the
  two seeded built-ins so PS8 acceptance ("an Assistant thread with
  the `general` persona can call one read-only tool, e.g.
  `list_jobs`, end-to-end") matches the seed: `builtin:general`
  gets `["assistant.*"]` and `builtin:coding` gets its existing
  `fs.*`/`shell.*`/`attachments.read` plus `assistant.*`. Append-only,
  per OQ-PS-5.

Tests (all 271 runtime lib tests green):
- 5 new planner tests (persona instructions in prompt, catalogue
  filtered to allowed tools only, no-tools trailer suppresses
  catalogue, disallowed tool calls dropped, allowed tool calls
  retained), 1 unit on `assistant_tool_id` namespacing.
- The existing `planner_tool_call_persists_as_card_and_dispatches_on_confirm`
  test still passes -- exercising the PS8 acceptance end-to-end on
  the seeded `builtin:general` persona with `list_jobs`.

`cargo test -p codeless-runtime --lib -- --skip claude_runner` green
(271 / 271); `cargo clippy -p codeless-runtime -p codeless-types -p
codeless-tools --all-targets -- -D warnings` clean; `cargo fmt --all
-- --check` clean. The pre-existing `codeless-bot-core::dispatcher`
`manual_range_contains` warning on this branch is unchanged by this
stage; the `claude_runner` integration-test failure is unchanged.

## Stage 9 (PS-ACCEPT) — landed

The substrate-doc Acceptance §3 ("each of items 1-8 has integration-
test coverage; `notes` has end-to-end coverage that drives the
Assistant → persona → tool → attachment path") is now satisfied by
`crates/codeless-runtime/tests/plugin_substrate_e2e.rs`, six tests
walking the on-disk `plugins/notes/` plugin through the public seams
the host binary uses at boot:

- `notes_plugin_loads_and_seeds_persona_addressable_by_thread` —
  `PluginRegistry::load_plugin` against the real plugin dir,
  migration apply against the runtime pool, persona upsert, and
  thread create bound to `notes:notes`. The append path round-trips
  through the NOOP planner fallback so the test exercises the
  persona FK without needing a fake runner.
- `persona_allowed_tools_admit_plugin_namespace_only` — PS3 matcher
  pinned at `notes.*` + `attachments.read`; built-in `assistant.*`
  and host-FS `fs.*` ids are rejected, proving the persona's column
  (not a UI routing prop) drives the answer.
- `plugin_tool_output_schema_round_trips_through_attachment_reconciler`
  — PS7 marker contract end-to-end: upload an attachment, walk
  `notes.append`'s real output schema with `find_attachment_refs`,
  reconcile against the live store row, assert the stored row's
  filename/mime/size win over the tool's "lies" hints.
- `plugin_tool_call_executes_through_registry` — PS1 / PS-NOTES
  contract: `notes.append` is reachable through `ToolRegistry::get`
  with a real `ToolCtx`; an empty body surfaces `InvalidArgs`, a
  well-formed body surfaces the documented PS-ACCEPT `Failed` (the
  signal that the per-tool ctx-extension writer is the next stage).
- `planner_allow_filter_admits_plugin_tool_under_persona_namespace`
  — PS8 acceptance: the same `tool_allowed` call the planner's
  publish closure makes still returns true after the persona has
  round-tripped through SQLite and the `assistant_threads.persona_id`
  FK.
- `notes_plugin_directory_shape_matches_substrate_contract` —
  Acceptance §1: the plugin tree is `plugins/notes/` +
  `crates/codeless-plugin-notes/`; no plugin assets leaked into
  `codeless-runtime` / `codeless-rpc` / `codeless-tools` / `ui/`.

`DOCS/PLUGIN-SUBSTRATE.md` Acceptance section now carries a per-item
status table pointing at the integration tests for items 1, 3, 5, 6,
7, 8, plus an honest accounting of items 2 + 4 (PS2 partially landed
via PS2a, PS4 halted; both block the estimator's worked-example
flow, neither blocks the substrate seams).

The `codeless-plugin-notes` crate is dev-dep-only of
`codeless-runtime`, preserving the substrate's tree direction
(`codeless-runtime` does not depend on any plugin crate at the
library layer). The host binary's `host_registration_table` in
`crates/codeless-cli/src/plugin.rs` remains the one place a plugin
crate becomes a runtime dependency.

`cargo test -p codeless-runtime --test plugin_substrate_e2e` green
(6/6); `cargo clippy -p codeless-runtime --tests --test
plugin_substrate_e2e -- -D warnings` clean.
