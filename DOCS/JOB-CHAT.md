# JOB-CHAT — one chat per job, many transports, one supervisor

> Companion to [`JOB-WORKFLOW.md`](./JOB-WORKFLOW.md),
> [`SCOPE-SLACK-INTEGRATION.md`](./SCOPE-SLACK-INTEGRATION.md), and
> [`SCOPE-TELEGRAM-INTEGRATION.md`](./SCOPE-TELEGRAM-INTEGRATION.md).
> Those docs each describe **one transport** that talks to codeless.
> This doc describes the **shared substrate underneath all of them**:
> a single chat thread per Job that any transport can write into, and
> a long-lived supervisor agent that watches the Run and replies in
> that thread. Where this doc disagrees with `JOB-WORKFLOW.md` or
> R1-R5 in `../CLAUDE.md`, **those win** — open an issue and update
> this file.

## The problem

The user already gets per-Job chat in three places:

1. The web UI's `CHAT` tab on the Job page (today).
2. Telegram, per `SCOPE-TELEGRAM-INTEGRATION.md` — notifications +
   command replies, scoped per chat-id.
3. Slack, per `SCOPE-SLACK-INTEGRATION.md` — same shape, scoped per
   thread.

Three problems with that as it stands:

- **The three transports do not share state.** A reply the user
  types into Telegram never appears in the web `CHAT` tab. The agent
  loses half its conversation history depending on which window the
  user happened to use.
- **None of the transports are agentic.** They are command surfaces
  (`/status`, `/resume`) plus notification fan-out. The user cannot
  ask "why has this been running so long?" and get a real answer —
  there is no thing on the other end of the chat with read access to
  the Job's state.
- **There is no place to put the supervisor.** "If the run has been
  going more than an hour, stop it and tell me why" needs a
  long-lived process per Job with read tools, action tools, and a
  voice in the chat thread. Today no crate owns that.

The user's framing from chat: *"chat session is the same as the job
chat, so if I send a message over Telegram to a job I would see it
in the same job chat window"* and *"a smart agent with job
awareness and Q/A and action."* This doc is the design for both.

## What "good" looks like

A Job has **one** chat thread. The thread lives in SQLite. Three
things read and write it:

```
                  ┌──────────────┐
   Web UI ─────▶  │              │ ◀───── Telegram adapter
                  │ chat_messages │
   CLI    ─────▶  │   (SQLite)   │ ◀───── Slack adapter
                  │              │
                  └──────┬───────┘
                         ▲
                         │ subscribe + write
                         │
                  ┌──────┴───────┐
                  │  Supervisor  │
                  │     agent    │
                  └──────────────┘
```

- **Every message** (user-typed or agent-written, from any surface)
  is one row in `chat_messages`, keyed by `job_id`.
- **Every surface** writes inbound via the same `post_job_message`
  RPC and renders outbound by subscribing to
  `ChatMessageAppended { job_id, ... }` on the existing event bus.
- **The supervisor agent** is just one more participant in the
  thread — `role=assistant`, sees the same history every surface
  sees, replies the same way.

Concretely, the affordances the design has to deliver:

| Affordance | What it does |
|---|---|
| Cross-surface history | Telegram reply at 10:01 visible in web UI at 10:02 with no refresh, including the supervisor's response. |
| Cross-surface threading | `(transport, channel, thread) → job_id` mapping so a Telegram thread and a Slack thread independently bind to the same Job and stay in sync. |
| Supervisor read access | Agent can read job state, recent events, the stage log, the handover, and the current template. |
| Supervisor action access | Agent can stop / pause / add a note / post a message — narrow tool surface, audited via the same event stream. |
| Per-Job process lifetime | One supervisor task per **running** Job; it exits when the Run reaches a terminal status. Re-runs spawn a fresh supervisor. |

What this doc does **not** decide:

- The Slack and Telegram surface-by-surface command vocabulary —
  those stay owned by their own SCOPE docs. JOB-CHAT only specifies
  the storage + supervisor substrate they all share.
- How `CHAT-EDITS-JOB-SPEC.md`'s "let the chat agent edit the spec"
  flow composes with the supervisor — see *Composition* below for
  the boundary, but the edit semantics stay in that doc.

## Data model

One new table. Two new event variants. No schema churn elsewhere.

```sql
CREATE TABLE chat_messages (
    id              TEXT PRIMARY KEY,    -- ULID
    job_id          TEXT NOT NULL REFERENCES jobs(id),
    run_id          TEXT,                 -- non-null once JOB-WORKFLOW (B) lands;
                                          -- pre-(B) rows leave it NULL. The
                                          -- supervisor's reading view is
                                          -- per-Job (job_id), never per-Run —
                                          -- run_id exists for UI filtering and
                                          -- analytics only. See OQ-CHAT-4.
    transport       TEXT NOT NULL,        -- 'web' | 'telegram' | 'slack' | 'cli' | 'supervisor'
    external_id     TEXT,                 -- transport-specific message id.
                                          -- INVARIANT: NOT NULL whenever
                                          -- transport IN ('telegram','slack').
                                          -- Web / CLI / supervisor messages
                                          -- leave it NULL; the per-row PK is id.
    thread_key      TEXT,                 -- transport-specific thread/channel ref
    author          TEXT NOT NULL,        -- 'user' | 'supervisor' | '<bot-name>' | '<user-handle>'
    role            TEXT NOT NULL,        -- 'user' | 'assistant' | 'tool' | 'system'
    body            TEXT NOT NULL,
    metadata_json   TEXT,                 -- transport-specific extras (attachments, formatting)
    created_at      INTEGER NOT NULL
);

CREATE INDEX chat_messages_job_idx ON chat_messages (job_id, created_at);

-- SQLite treats every NULL as distinct in a UNIQUE constraint, so a
-- naive `UNIQUE (transport, external_id)` would silently allow
-- duplicate ingest for any transport that ever sends a NULL
-- external_id. The partial index narrows the constraint to the rows
-- whose invariant says external_id must exist (Telegram / Slack);
-- web / CLI / supervisor rows are uniquified by their `id` PK alone.
CREATE UNIQUE INDEX chat_messages_external_idx
    ON chat_messages (transport, external_id)
    WHERE external_id IS NOT NULL;

CREATE TABLE chat_bindings (
    transport       TEXT NOT NULL,        -- 'telegram' | 'slack'
    channel_id      TEXT NOT NULL,
    -- thread_id uses the empty string '' as the sentinel for "no
    -- thread on this transport" rather than NULL. The PK below
    -- depends on it; SQLite treats NULLs as distinct and would let
    -- the same (transport, channel) bind to two different jobs if
    -- thread_id were nullable. The empty string is unambiguous on
    -- both Telegram and Slack (no real thread id is empty).
    thread_id       TEXT NOT NULL DEFAULT '',
    job_id          TEXT NOT NULL REFERENCES jobs(id),
    bound_at        INTEGER NOT NULL,
    bound_by        TEXT NOT NULL,        -- user handle on that transport
    PRIMARY KEY (transport, channel_id, thread_id)
);
```

`chat_bindings` is the lookup the Telegram and Slack adapters use to
turn an inbound message on `(transport, channel, thread)` into a
`job_id` for `post_job_message`. The web UI doesn't need a binding —
it already knows the `job_id` from the URL.

Event variants on the existing bus:

- `ChatMessageAppended { job_id, message: ChatMessage }` — fires
  whenever a row is inserted. Every surface subscribes; echo
  suppression rules are specified per direction in *Transport
  adapters* below — the rule is asymmetric and not a simple
  `(transport, external_id)` self-match.
- `ChatBindingCreated { transport, channel_id, thread_id, job_id }`
  — fires when a user runs `/codeless bind` (or equivalent) on a
  transport. Useful for the web UI to show "this Job is also being
  watched in #ops-codeless".

Wire types (`codeless-types`, mobile-safe per R1):

```rust
pub struct ChatMessage {
    pub id: MessageId,
    pub job_id: JobId,
    pub run_id: Option<RunId>,
    pub transport: ChatTransport,
    pub author: String,
    pub role: ChatRole,
    pub body: String,
    pub metadata: serde_json::Value,
    pub created_at: i64,
}

pub enum ChatTransport { Web, Telegram, Slack, Cli, Supervisor }
pub enum ChatRole { User, Assistant, Tool, System }
```

**v0.1 transport set (settled).** Exactly five variants: `Web`,
`Cli`, `Telegram`, `Slack`, `Supervisor`. `Web` and `Cli` and
`Supervisor` ship with C1 (the substrate); `Telegram` ships at the
end of C1 as the first external transport; `Slack` lands with C3 as
a copy-shape of the Telegram adapter. No other transports (SMS,
Discord, email, webhook) are in scope before Phase 7; adding a sixth
means a new variant **and** a doc PR amending this list — adapters
must not invent unrecognised values.

**Wire-name convention for `ChatTransport` (settled).** Rust
PascalCase variants serialize as lowercase ASCII strings on every
wire (JSON, SQLite, Telegram/Slack metadata, log fields): `Web →
"web"`, `Cli → "cli"`, `Telegram → "telegram"`, `Slack → "slack"`,
`Supervisor → "supervisor"`. This is the contract the SQL
`transport` column in `chat_messages` and `chat_bindings` already
encodes. Specta-derived TypeScript bindings inherit the same casing.
Adapters compare transport values **only** as these lowercase
strings; never as Rust identifiers, display names, or
human-language synonyms ("CLI", "tg", "WebUI" are all wrong on the
wire).

## RPC surface

Three new methods on `RpcServer`, all of them host-only:

| Method | Purpose | Args | Result |
|---|---|---|---|
| `post_job_message` | Append an inbound message. Used by web chat input, Telegram adapter, Slack adapter, CLI. | `{ job_id, transport, external_id?, author, body, metadata? }` | `ChatMessage` |
| `list_job_messages` | Paginated history for cold-loading a transport (e.g. Telegram thread opened after the fact). | `{ job_id, before?: MessageId, limit: u32 }` | `Vec<ChatMessage>` |
| `bind_chat_thread` | Bind a `(transport, channel, thread)` to a Job. Called by Telegram/Slack on `/codeless bind <job_id>`. | `{ transport, channel_id, thread_id?, job_id }` | `ChatBinding` |

The supervisor agent calls `post_job_message` with
`transport=Supervisor` for every reply it generates. No new RPC for
the supervisor — it is **a consumer**, not a peer surface.

## Transport adapters

Each adapter does three things:

1. **Inbound:** on an incoming message in a thread that has a
   `chat_bindings` row, call `post_job_message` with `transport =
   <this>` and `external_id = <the platform's inbound message id>`.
   If no binding exists, the adapter handles the `/codeless bind
   <job_id>` command path itself (or returns an instructional
   message). Adapters **never** invent messages on the user's behalf.

2. **Outbound:** subscribe to `ChatMessageAppended`. The rule for
   whether to forward a message to this transport's channel is
   asymmetric, because the inbound `external_id` is the *source*
   id (Telegram's id for a Telegram inbound) and the outbound send
   produces a *new* platform id that nobody in the loop yet knows:

   - **Message originated on this transport** (`message.transport
     == <this>`): **skip.** The user already sees it in their
     Telegram/Slack client; re-posting would double-render. The
     `external_id` self-match is fine here because both sides are
     the source id.
   - **Message originated on a different transport** (web / CLI /
     supervisor / another bot): **forward.** Call the platform's
     send API, then `UPDATE chat_messages SET metadata_json =
     jsonb_set(metadata_json, '$.delivery.<transport>',
     <platform_id>) WHERE id = ?` so the delivery receipt lives on
     the row. The platform id is **not** written back into
     `external_id` — that column belongs to the originating
     transport and stays stable.

   Idempotency: if the adapter restarts mid-flight and re-receives
   a `ChatMessageAppended` it already forwarded, the
   `metadata_json.delivery.<transport>` field tells it the send
   already happened. Skip on presence.

3. **Cold-load:** on `/codeless bind`, fetch the last N messages via
   `list_job_messages` and post a single condensed "joining
   mid-thread" summary message back to the channel. We do **not**
   replay every prior message into the new transport — that would
   spam Telegram with web-UI history.

Adapter ownership per existing crate layout:

- Telegram adapter — `codeless-telegram` (already scaffolded).
- Slack adapter — new `codeless-slack` crate, same shape, lands when
  Slack moves past `SCOPE-SLACK-INTEGRATION.md`.
- Shared adapter trait + outbound subscription glue —
  `codeless-bot-core`, the existing crate that already owns the
  `EventSource` abstraction referenced in
  [`JOB-WORKFLOW.md`](./JOB-WORKFLOW.md#integration-with-what-already-exists).
- Web UI — `ui/codeless-ui/`, calls `post_job_message` through
  `RpcClient` (R2). No new transport, no `@tauri-apps/*` import.

All three transport crates are host-only per R1. None of them ever
appear in the mobile-safe dependency graph; the mobile UI talks to
the supervisor and the chat history through the same `RpcClient`
that the browser uses.

## The supervisor agent

### Lifetime

One supervisor task is spawned in `codeless-runtime` when a Run
enters `running`. It is cancelled when the Run reaches a terminal
status (`completed` / `failed` / `cancelled` / `paused-pending-user`).
A resumed Run spawns a fresh supervisor — supervisors are
per-Run-attempt, not per-Job.

**Crate placement (load-bearing for R1).** The supervisor lives
**inside `codeless-runtime`**, in a new `supervisor` module
alongside the existing runner / event-bus code. It is **not** a
separate `codeless-supervisor` crate. Reasons: it needs the event
bus, sqlx, and the same RPC handles `runtime` already wires up;
and putting it in `runtime` keeps it on the right side of R1 by
construction — any tool that needs a process call routes through
the existing RPCs in `codeless-adapters-host`, not via a new crate
that might re-import `tokio::process`. Future readers tempted to
create `crates/codeless-supervisor/` should resist; the boundary
is the module, not the crate.

The supervisor is **separate from the coding runner** by design:

- The coding runner is a short-lived process per stage with a
  narrow tool surface, started from a stage prompt and exiting
  when the stage closes.
- The supervisor is a long-lived loop with a conversation tool
  surface, subscribed to the Run's event stream and the Job's
  chat messages.

Fusing them couples conversation cadence with code execution; when
the runner exits the supervisor must stay alive to answer
"why did it stop?"

### Tool surface

Start read-only, narrow:

| Tool | Reads | Notes |
|---|---|---|
| `get_job_state` | jobs row + current run | status, stage, elapsed wall-clock, cost-so-far |
| `read_events` | events tail since cursor | bounded; the supervisor maintains a cursor |
| `read_handover` | current handover.md | same path the next stage's prompt assembler uses |
| `read_template` | current template.yaml | so the supervisor can answer "what stages are left?" |
| `read_stage_log` | `runs/<run_id>/log.md` | what actually happened in stage N |
| `read_notes` | `runs/<run_id>/notes/` | so the user's prior feedback notes are in context |

Then narrowly-scoped actions, gated behind explicit user request
in chat (the agent must quote the user's ask back before invoking
an action tool):

| Tool | Effect |
|---|---|
| `stop_job` | `runtime::cancel_job(job_id, reason)` — same path the UI's `[stop]` button uses. |
| `pause_after_stage` | The JOB-WORKFLOW (A.5) affordance, once it lands. No-ops until then. |
| `add_job_note` | The (A)-punch-list RPC — writes `runs/<run_id>/notes/<ts>-supervisor.md`, commits. |
| `post_chat_message` | `post_job_message` with `transport=Supervisor`. The supervisor's only **voice** tool. |

The supervisor **never** edits the template, never edits the
handover, never resumes a paused Run on its own. Those are user
intent surfaces; the supervisor is a watcher and an executor of
explicitly-asked actions.

### How "if it runs >1h, stop it" works

The supervisor is not a polling daemon. The flow for the user's
example:

1. User in chat: "if this runs more than an hour, stop it and tell
   me why."
2. Supervisor records the intent as a **pre-armed goal** (see Hard
   rule 4 and the `supervisor_goals` table in C3) — `kind =
   deadline-stop`, `deadline = JobStarted.started_at + 1h`,
   `authorised_by = <the user message's MessageId>`. Persisting
   the goal is what makes it survive a process restart.
3. Supervisor uses a `tokio::time::sleep_until(deadline)` arm
   alongside its `Event` subscription. Whichever fires first wins.
   On boot, the supervisor rehydrates open goals from
   `supervisor_goals` and re-arms timers.
4. At the deadline:
   - If the Run is still `running`, supervisor calls
     `read_events` for the last ~5 minutes, summarises the most
     recent stage activity, calls `stop_job` with that summary as
     the `reason` (no 5-second preview — the goal is pre-armed
     per Hard rule 4), then `post_chat_message` with a one-
     paragraph "I just stopped this because you asked me to after
     an hour; here is what was happening" reply that cites the
     original authorising message.
   - If the Run already finished, supervisor marks the goal
     `superseded` and posts nothing.
5. If the user changes their mind before the deadline ("never
   mind, let it run"), the supervisor marks the goal `cancelled`,
   drops the timer, and posts a one-line confirmation.

This is a tool-loop pattern, not a new subsystem. The same shape
covers "ping me when stage 3 finishes," "if cost passes $1, stop,"
and similar deadline / threshold prompts.

### What model drives it

Whichever runner the user has configured as their **assistant
runner** in settings — same plumbing as today's chat panel in the
web UI. The supervisor is not pinned to Claude / GPT / anything
specific; the choice belongs in settings, not in code.

## Composition with adjacent docs

- **`JOB-WORKFLOW.md` (A)** — adds `update_job_handover`,
  `update_job_template`, `add_job_note`. The supervisor calls
  `add_job_note` only; the template/handover RPCs stay user-only.
- **`JOB-WORKFLOW.md` (A.5) pause** — the supervisor's
  `pause_after_stage` tool is the same RPC the run page's button
  calls. The supervisor can suggest pausing; only the user (via
  chat ask + supervisor confirm) actually arms it.
- **`JOB-WORKFLOW.md` (B) Job/Run split** — `chat_messages.run_id`
  becomes non-null at that point. A new Run starts a fresh
  supervisor; the old Run's supervisor exits cleanly.
- **`SCOPE-TELEGRAM-INTEGRATION.md` / `SCOPE-SLACK-INTEGRATION.md`**
  — those docs' Surface 1-3 command vocabularies stay theirs. Both
  adapters become consumers of `post_job_message` /
  `ChatMessageAppended` rather than owning their own per-transport
  message store. The "Out of scope: X-as-an-agent-tool" sections in
  both docs are honoured — the agentic surface is **the supervisor
  in the codeless process**, not a Slack/Telegram-resident agent.
- **`CHAT-EDITS-JOB-SPEC.md`** — that doc's "chat agent edits the
  spec" flow runs **before** a Job is started; the supervisor runs
  **after**. Boundary: the spec-editing agent never has supervisor
  tools, the supervisor never edits the template. Two agents, two
  scopes, one chat surface they take turns being the
  `role=assistant` author on.
- **Plans (P3)** — a PlanRun spawns one Job at a time; each Job's
  supervisor is per-Run as above. The Plan-level "what's happening
  across these jobs?" surface is a separate problem; revisit when
  Plans get UI.

## Recommended sequencing — (C1) → (C2) → (C3)

Same phasing discipline as JOB-WORKFLOW. Each phase ends with
something a user can drive end-to-end.

### (C1) — Unified chat table, no agent

Smallest slice that gets cross-surface history working:

- [ ] Migration: `chat_messages`, `chat_bindings`.
- [ ] Wire types: `ChatMessage`, `ChatTransport`, `ChatRole`,
      `ChatBinding`.
- [ ] RPCs: `post_job_message`, `list_job_messages`,
      `bind_chat_thread`.
- [ ] Event: `ChatMessageAppended`, `ChatBindingCreated`.
- [ ] Web UI: the existing `CHAT` tab's message input + render path
      go through `post_job_message` / `ChatMessageAppended`. No more
      transport-local store.
- [ ] Telegram adapter: inbound writes via `post_job_message`,
      outbound subscribes to `ChatMessageAppended`, `/codeless bind`
      writes to `chat_bindings`.

What (C1) gives the user: a message typed in Telegram appears in the
web UI on the next tick, and vice versa. **No supervisor yet** —
nothing answers; the chat is human-only across surfaces.

### (C2) — Supervisor agent, read-only tools

- [ ] `Supervisor` task spawned by `runtime::start_run`.
- [ ] Subscribes to `ChatMessageAppended` for its Run's `job_id`.
- [ ] Read-only tools: `get_job_state`, `read_events`,
      `read_handover`, `read_template`, `read_stage_log`,
      `read_notes`.
- [ ] `post_chat_message` is the only write tool (the supervisor's
      voice).
- [ ] On Run terminal status, supervisor posts a one-paragraph
      summary and exits.

What (C2) gives the user: ask "what stage is it on?" / "why did
stage 3 take so long?" in any surface, get a real answer that
references the actual event stream and stage log.

### (C3) — Action tools, deadline / threshold loops

- [ ] `stop_job` and `add_job_note` action tools.
- [ ] `pause_after_stage` tool (no-op until JOB-WORKFLOW (A.5)).
- [ ] `supervisor_goals` table + migration. Columns: `id`,
      `run_id`, `kind` (`deadline-stop` / `threshold-stop` /
      `event-notify` / …), `condition_json`, `action_json`,
      `authorised_by` (the `chat_messages.id` of the user message
      that armed it), `status` (`armed` / `fired` / `cancelled` /
      `superseded`), `created_at`, `fired_at`. Load-bearing for the
      "if it runs >1h, stop it" feature surviving a restart —
      C3's example doesn't work without it.
- [ ] Goal rehydration on supervisor boot — scan `armed` rows for
      the Run, re-arm timers / event watchers, drop stale ones.
- [ ] Deadline / threshold intent recognition — the supervisor
      treats "if X then Y" chat requests as inserts into
      `supervisor_goals`, not as in-memory state.
- [ ] Slack adapter parity with Telegram — same `chat_bindings` shape.

What (C3) gives the user: "if it runs more than an hour, stop and
tell me why" works end-to-end.

## Hard rules specific to this surface

These are enforceable by grep or in CI; trip one and the design
intent is broken.

1. **No transport-local message store.** The web chat panel, the
   Telegram adapter, the Slack adapter — none of them maintain
   their own message history table. SQLite `chat_messages` is the
   single store per R4. Any in-memory buffer in the UI is
   presentation state only and rehydrates from
   `list_job_messages` on reconnect.
2. **The supervisor never imports `tokio::process` or
   `std::process`.** It is a module **inside `codeless-runtime`**
   (host-only per R1, see *Crate placement* above) — not a
   separate crate. Its tool surface routes through existing RPCs
   that already live in `codeless-adapters-host`. Process spawn
   stays where R1 puts it.
3. **The supervisor's only voice is `post_chat_message`.** No
   side-channel logging-to-the-user, no `eprintln`-as-UX. Every
   user-visible message from the supervisor is a row in
   `chat_messages` so the audit trail is complete.
4. **The supervisor never auto-invokes destructive actions
   *unless the user pre-armed the exact action*.** Two regimes:

   - **Ad-hoc actions** (the supervisor decides on its own that
     stopping is appropriate, or the user's chat is ambiguous):
     the supervisor posts what it is about to do **before**
     invoking it, with a short window (5 seconds default,
     configurable) for the user to say "wait." Default behaviour
     for any destructive action the supervisor reasons its way to.
   - **Pre-armed actions** (the user explicitly said "if X then
     Y" earlier in the chat and the supervisor recorded the
     intent as a goal — see C3 / `supervisor_goals` below): the
     supervisor invokes the action **immediately** when the
     condition fires, then posts the "I just did X because Y"
     summary. No preview, no nag — the user already authorised
     it. The audit trail is the original "if X then Y" message
     plus the post-action summary, both rows in `chat_messages`.

   The principle is symmetrical: the user is never surprised by a
   destructive action they did not authorise, and never nagged
   about one they did.
5. **Action-tool invocations emit events.** `stop_job` from the
   supervisor produces the same `JobCancelled` event as the UI
   button. The `events` row carries `actor=supervisor` so the
   audit trail tells you which surface triggered the cancel.

## Open questions — settled for v0.1

The five questions raised in earlier drafts are settled below in the
style of SCOPE.md §Open questions §Settled. The numbering
(OQ-CHAT-1 .. OQ-CHAT-5) is load-bearing — other docs and
session notes cite these by number; do not renumber.

1. **OQ-CHAT-1 — Echo suppression on edits: insert-new with a
   `replaces` pointer.** A user editing a Telegram or Slack message
   does **not** trigger an `UPDATE chat_messages SET body = ?`. The
   adapter inserts a fresh row (its own `id`, its own
   `external_id` from the platform's edit-event id) and writes
   `metadata_json.replaces = <prior_message_id>` to link it to the
   row it supersedes. The table is append-only by construction. The
   web UI renders the most recent row in a `replaces` chain and may
   collapse the prior rows behind an "edited" affordance; the
   supervisor reads the whole chain — edits are part of the audit
   trail, not a destructive overwrite. Same shape for adapter-side
   deletes: insert a tombstone row with `metadata_json.deletes =
   <prior_message_id>` and an empty `body`. Revisit only if edit
   churn becomes a measurable fraction of chat volume.

2. **OQ-CHAT-2 — Per-message visibility: persist previews.** The
   supervisor's "I am about to stop the job in 5s" preview is a
   normal `chat_messages` row with `role = system`, `transport =
   supervisor`, and `metadata_json.preview = { window_ms: 5000,
   action: "stop_job", resolves_at: <epoch_ms> }`. The follow-up
   row (the action's "I just stopped this because …" summary)
   carries `metadata_json.resolves = <preview_message_id>` so the
   UI can pair them. Cancelled previews (the user said "wait") are
   still rows; the cancellation message points back via `replies_to`.
   The audit trail beats the transient-UX win, and the UI can style
   `role = system` rows distinctly (dim, smaller, collapsible).
   Pre-armed actions (Hard rule 4, second regime) have **no**
   preview row — only the post-action summary — by design.

3. **OQ-CHAT-3 — Multi-user trust: single-tenant for v0.1, every
   channel member trusted.** Any human in a bound Telegram chat or
   Slack thread may issue any chat-driven action (`stop_job`,
   `add_job_note`, etc.). The trust boundary is the `chat_bindings`
   row: only the operator (the human who runs `/codeless bind` on
   the transport) can create one, and the operator is implicitly
   vouching for everyone they let into that channel. The
   `chat_bindings.bound_by` column records who armed the binding so
   the audit trail names a human even when the destructive action
   comes from a different channel member. Per-user OIDC + per-action
   ACLs are deferred to Phase 7; this doc will not pre-architect
   them. The supervisor does not see, and does not reason about,
   Telegram/Slack user identity beyond the `author` string on the
   inbound row.

4. **OQ-CHAT-4 — Cross-Run continuity: `chat_messages.run_id` is
   for UI filtering and analytics only, never for supervisor
   grounding.** The canonical view of "the chat" is the Job-level
   stream (`job_id`). A fresh supervisor's first action is
   `list_job_messages(job_id, before = None, limit = N)` —
   never `WHERE run_id = ?`. `run_id` exists on the row for two
   reasons: (a) the UI may offer a "this Run only" filter; (b)
   cost / message-count analytics can attribute per-Run. The web UI
   **must not** default to per-Run filtering; if it ever exposes
   one, the filter chrome ("showing Run 2 only — earlier Run
   messages hidden") must be visible enough that the operator
   notices their view has diverged from the supervisor's. Pre-(B)
   rows leave `run_id` NULL; that is not a bug.

5. **OQ-CHAT-5 — Typed `metadata_json`: stay
   `serde_json::Value` for v0.1, revisit after two transports
   ship.** The wire type keeps a `serde_json::Value` blob.
   Adapters and the supervisor write and read keys by name. The
   keys the substrate itself owns are namespaced and documented
   here; transport-specific extras live under
   `metadata_json.<transport>.*` so the substrate's keys and an
   adapter's keys cannot collide:

   | Key | Owner | Set by | Meaning |
   |---|---|---|---|
   | `delivery.<transport>` | substrate | outbound adapter after a successful send | platform-side message id of the delivered copy; presence == "already delivered, do not re-send" |
   | `replaces` | substrate | adapter ingesting a platform edit | `chat_messages.id` of the row this one supersedes |
   | `deletes` | substrate | adapter ingesting a platform delete | `chat_messages.id` of the row tombstoned |
   | `replies_to` | substrate | any writer | `chat_messages.id` of the row this one is a reply to |
   | `resolves` | substrate | supervisor | `chat_messages.id` of the preview row this action-result resolves |
   | `preview` | substrate | supervisor | `{ window_ms, action, resolves_at }` for ad-hoc destructive previews |
   | `<transport>.*` | adapter | inbound adapter | transport-native extras (attachments, formatting, reactions) |

   A typed enum-per-transport replaces this table once two
   transports are in production **and** the actual shape has
   stopped churning for one release cycle. Not a blocker for C1
   and not a blocker for C3.

## What lands in code first (C1's punch list)

| # | Change | Crate / module | Wire impact | Size |
|---|---|---|---|---|
| 1 | `chat_messages`, `chat_bindings` migrations | `codeless-runtime/migrations/` | new tables | S |
| 2 | `ChatMessage`, `ChatBinding` wire types | `codeless-types` | new structs/enums | S |
| 3 | `post_job_message`, `list_job_messages`, `bind_chat_thread` RPCs | `codeless-rpc` + `codeless-runtime/rpc.rs` | new methods | S |
| 4 | `ChatMessageAppended`, `ChatBindingCreated` events | `codeless-types::Event` + bus emit | new variants | S |
| 5 | Web UI: rewrite `CHAT` tab over the new RPCs | `ui/codeless-ui/modules/jobs/` | none | M |
| 6 | Telegram adapter: inbound → `post_job_message`; outbound subscriber; `/bind` command | `codeless-telegram` | none | M |
| 7 | Echo-suppression helper in `codeless-bot-core` | new module | none | S |
| 8 | Cold-load summary message on `/bind` | `codeless-telegram` + shared helper | none | S |

Estimate: one focused session for items 1–5 (the substrate), a
second session for items 6–8 (the first transport). Slack is a
copy-shape of items 6–8 once the pattern is proven.

## Pointers

- Iterate loop the supervisor leans on: [`JOB-WORKFLOW.md`](./JOB-WORKFLOW.md)
- Telegram surface (commands, notifications): [`SCOPE-TELEGRAM-INTEGRATION.md`](./SCOPE-TELEGRAM-INTEGRATION.md)
- Slack surface (commands, notifications): [`SCOPE-SLACK-INTEGRATION.md`](./SCOPE-SLACK-INTEGRATION.md)
- Chat-driven spec editing (pre-Run agent): [`CHAT-EDITS-JOB-SPEC.md`](./CHAT-EDITS-JOB-SPEC.md)
- Single-UI architecture: [`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md)
- Job page tab layout (where `CHAT` lives): [`JOB-UI.md`](./JOB-UI.md)
- Workspace agent rules (R1-R5): [`../../CLAUDE.md`](../../CLAUDE.md)
