# JOBS-UX — chat-as-control center for a job

**Status:** design proposal, 2026-05-14. Supersedes the U1
description in [`PROGRESS.md`](./PROGRESS.md) "RUN page UX
overhaul" — that entry stays as the high-level priority, this
doc carries the load-bearing decisions.

If anything below contradicts [`SCOPE.md`](./SCOPE.md) or
[`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md), those win.

## What this document is for

The current JobPage has three roughly-peer panes (SPEC / STAGES /
RUN), with chat as a sub-feature under RUN. That model treats
the conversation as *one thing the user can do*. This document
moves to a different mental model:

> **The chat window is the control center.** Every observation a
> user has about a job (what the agent did, what it cost, what
> it's about to do, what the user wants it to do next) lives in
> one scrolling, live, conversational pane. Run / pause / stop /
> resume are inline controls on the same composer the user types
> into. The right-pane stats (cost, runtime, status, branch) are
> ambient signal, not the primary surface.

This is the same UX shape as Claude Code, Cursor, and Copilot —
because that shape is the one that survives the "I look away for
a minute, then come back and pick up" cycle, which is exactly
how a developer uses an AI coding tool.

## What's broken in the current UX (and what we're fixing)

Concrete pains observed in the 2026-05-13 dogfood session, in
priority order:

1. **SSE silently stalls.** The page stops receiving events
   without indication; the user has to refresh to see what
   happened. Cost, runtime, and status look frozen.
2. **State doesn't update in real time even when SSE is alive.**
   Cost and elapsed time are computed at fetch time and only
   refresh on lifecycle events (`job-completed`, etc.), so a
   running job's bar looks static for minutes at a time.
3. **Three competing places for "what the agent did".** Timeline
   has events; StageDetail has rollups; JobChat has its own
   parallel conversation that doesn't share state with the
   timeline. The user has to look in three places to reconstruct
   what's happening.
4. **Controls are far from where the user's hands are.** `[run]`
   is in the header. `[stop]` is in the header. The chat
   composer is at the bottom of a tab the user might not be on.
   Pausing the agent to ask a question requires hunting.
5. **No "is the agent thinking right now?" signal.** Token-by-
   token streaming is not rendered; the user sees a long silence
   between tool calls and the final message.

The fix is a single integrated pane with reliable SSE
underneath. Both pieces are required — fixing one without the
other ships a worse experience.

## Mental model

Think of the JobPage as **one continuous conversation about one
job**. Everything the agent does is "the agent saying or doing
something in the conversation"; everything the user does is
"the user saying or doing something in the conversation."

The conversation includes:

- User messages.
- The agent's streaming reasoning and final messages.
- Tool calls (Read, Edit, Bash, etc.) as collapsed cards.
- Lifecycle moments (stage-started, verify-passed, cost-cap-
  warning, job-paused).
- System errors and disconnection notices.

The conversation does **not** include — these stay on the right
pane, as ambient signal:

- The job's metadata (id, branch, worktree, repo).
- The cost / runtime / cap bars (these need to *update*, but
  they don't need to be in the chat stream).
- The stage tree as a navigation aid.
- Files-produced summary, handover summary, final-rollup card
  for completed jobs.

The conversation is one stream sorted by `created_at`. There is
no separate "timeline" vs "chat" vs "stages" tab. There is the
conversation, and there are auxiliary panels that hang off the
right side.

## Layout

```
┌─ TabBar ─────────────────────────────────────────────────────┐
│ Files Jobs [Job 01KR…]                                        │
├─ Left explorer ─┬─ Conversation ─────────────┬─ Right panel ─┤
│ (file tree,     │                            │ status badge   │
│  same as today) │   …                        │ runtime ●●●●   │
│                 │   user: "submit a job…"    │ cost     ●●○○  │
│                 │   ─ stage 1 started ─       │                │
│                 │   [tool] Read stage.rs      │ Summary tab    │
│                 │   [tool] Edit stage.rs      │ Files tab      │
│                 │   agent: "I'm going to…"    │ Handover tab   │
│                 │   ─ verify passed ─         │ Stages tab     │
│                 │   ─ stage 1 committed ─     │                │
│                 │   ─ stage 2 started ─       │ (drill-down,   │
│                 │   …(token stream)…          │  collapsible,  │
│                 │   ─ paused: cost cap ─      │  not the       │
│                 │                            │  primary       │
│                 │                            │  surface)      │
│                 │ ┌─ Composer ──────────────┐│                │
│                 │ │ [Resume ▾] [Stop]   ▲   ││                │
│                 │ │ message claude…          ││                │
│                 │ │ attach · send · ⌘Enter  ││                │
│                 │ └─────────────────────────┘│                │
└─────────────────┴────────────────────────────┴───────────────┘
```

Three columns:

- **Left** — repo file tree, exactly as today. Read-only context.
- **Centre** — the conversation, with the composer pinned to the
  bottom. This is the load-bearing surface; everything else is
  supporting.
- **Right** — collapsible auxiliary panels. Status badge with
  live tick, cost/runtime bars, the tabbed drill-down (Summary /
  Files / Handover / Stages). Defaults to open on desktop;
  collapses on narrow viewports.

## The conversation, in detail

### Message kinds

The conversation is a flat list of messages, each one with a
`kind` that determines rendering:

| Kind | Source | Rendered as |
|---|---|---|
| `user_msg` | user types and sends | right-aligned bubble |
| `agent_msg` | `ai-message-complete` (final), or buffered `ai-token` deltas (streaming) | left-aligned bubble; "streaming" indicator while open |
| `tool_call` | `tool-call` event | collapsed card with tool name + short args summary; click to expand the full args/result |
| `lifecycle` | `stage-started`, `verify-passed`, `verify-failed`, `stage-completed`, `job-paused`, `job-resumed`, `cost-cap-warning`, etc. | thin centred divider with a label and timestamp |
| `error` | `task-failed` reason, transport error, etc. | red-tinted card |
| `system` | "connection lost — reconnecting…", "missed N events while reconnecting" | small grey card |

Every message has a stable id (event cursor for SSE-derived
ones; client-minted ULID for user messages until the daemon
echoes them back).

### Default visibility

Not every event deserves the same prominence. Default rendering:

- **Always-expanded:** `user_msg`, `agent_msg` (final messages),
  `lifecycle` for stage transitions and pauses, `error`,
  `system`.
- **Collapsed but visible:** `tool_call`. Shows one line (tool +
  one-line summary like `Read stage.rs` or `Edit src/lib.rs (3 lines)`).
  Click to expand the full args, output, duration.
- **Hidden by default, toggle to show:** `agent_msg` *intermediate*
  text (the running-thoughts deltas before `ai-message-complete`).
  A "show thinking" toggle near the composer reveals them.

The right-pane has a "raw events" toggle (already exists today)
that flips the conversation to show every event verbatim, no
roll-up. For debugging only — not the primary surface.

### Streaming

`ai-token` events stream deltas. The conversation renders the
in-flight message as soon as the first token arrives, with a
visible "streaming" indicator (a slow pulse, not a spinner —
spinners imply unbounded waiting). On `ai-message-complete`, the
indicator disappears and the message freezes. If the connection
drops mid-stream, the partial content stays visible with a "lost
stream" note; the reconnect logic (below) decides whether to
keep it or replace it from the replayed event stream.

This is the same shape Copilot and Claude Code use. It is the
load-bearing piece that makes the agent feel alive.

### Tool-call cards

Each `tool-call` event becomes a card. Collapsed it shows:

```
🔧 Edit  crates/codeless-types/src/stage.rs               14:22:08
```

Expanded:

```
🔧 Edit  crates/codeless-types/src/stage.rs               14:22:08
    old: "    pub session_id: Option<String>,"           (was None)
    new: "    pub session_id: Option<String>,            (now Some)"
    duration: 412 ms
```

Cards collapse / expand independently — clicking one doesn't
collapse another. State is per-card, ephemeral (does not
persist across refresh). Long-running tools (Bash with > 1s
output) show a "running…" indicator while the call is in flight.

### Lifecycle dividers

Stage transitions and cap-pauses get visual prominence — they
mark inflection points the user wants to be able to scan to:

```
──────────── stage 1 started: types ─────────── 14:21:42
──────────── stage 1 verify: passed ──────────── 14:24:11
──────────── stage 1 committed ──────────────── 14:24:12
──────────── stage 2 started: runtime ────────── 14:24:12
──────────── paused: cost cap ($6.89 / $5.00) ── 14:35:47
```

Centred dividers, monospaced, high-contrast. Same shape as a
git log header. The cost-cap divider includes the actual
numbers and is mildly tinted to draw the eye.

## The composer, in detail

The composer is the bottom-pinned input area in the conversation
column. It always contains a textarea; the **action button** on
its left changes based on job state.

### State-driven primary action

| Job state | Primary button | Secondary action | Textarea behaviour |
|---|---|---|---|
| `Draft` | `[run ▶]` | (none) | normal compose; send is disabled until [run] clicked, or [run + send] in one click |
| `Queued` | `[stop ■]` (cancel) | — | disabled until running |
| `Running` | `[stop ■]` and (when A0 lands) `[pause ⏸]` | — | always active; sending while running queues the message as the next user turn the agent will read at the next resume point |
| `Paused (cost-cap)` | `[resume ▶ …]` opens an inline mini-form for cap bump amount | `[stop]` to terminate | active; sending folds the message into the resume prompt |
| `Paused (wall-clock-cap)` | `[resume ▶ …]` opens a wall-clock bump | `[stop]` | as above |
| `Paused (user)` | `[resume ▶]` | `[stop]` | as above |
| `Stopped`, `Failed`, `Completed` | `[re-run ▾]` dropdown (rerun from scratch / rerun with feedback / rerun from stage N) | — | active; pressing send on a terminal job means "open re-run with feedback dialog pre-filled with this message" |

The primary button's icon and colour change but its **position
never moves** — bottom-left of the composer. Muscle memory.

### Sending a message while the agent is running

The user can always type. Pressing send while the agent is
running:

- (Without A0) Saves the message as a note on the job; surfaces
  inline as "queued for next resume" on the message itself. The
  agent will not see it until the current task ends.
- (With A0) Pauses the agent at the next safe point (after the
  current tool call returns), folds the message into the prompt,
  and resumes. The conversation shows a `paused: user message`
  divider, the user's bubble, and a `resumed` divider when the
  agent picks up.

Either way the *user's experience* is: "I sent a message and
something visible happened." The "queued for next resume"
fallback is acceptable until A0 lands but is explicitly
inferior — the gap should be visible to the user so they
understand why their question isn't being acted on yet.

### Attachments and paste

Attachments work as today (the `agent_chat: route footer AI panel
to host CLI runners` commit shipped the wire). Drag-drop, paste,
attach button — same affordances, same chip row above the
textarea. No change here.

### Keyboard

| Shortcut | Action |
|---|---|
| `⌘/Ctrl + Enter` | Send |
| `Esc` while focused in composer | Blur (and pause the agent if running, when A0 lands) |
| `↑` in empty composer | Recall last user message |
| `⌘/Ctrl + K` | Focus composer (from anywhere on the page) |

## Right pane

The right pane is **ambient signal** — visible when you glance
right, never the primary action surface. It contains:

### Status header (always visible)

```
claude · codeless                                       [re-run]
1m26s / 1h00m  $8.49 / $5.00  ●● stopped: cost-cap

01KRGS17GBXEPFPS2XCFP9MFCR
branch    codeless/capture-claude-session-id
worktree  /tmp/codeless-worktrees/job-01KRGS17GBXEPFPS2XCFP9MFCR
repo      /home/user/code/rust/codeless-workspace/codeless
```

The runtime and cost figures **update every second** while the
job is in any non-terminal state (`running`, `paused`,
`awaiting-review`). The cost is computed as
`last_known_cost + (now - last_known_cost_time) * 0` for live
display (i.e. it doesn't extrapolate — it shows the last value
the SSE stream delivered, and increments only on real events).
The runtime *does* tick with `setInterval`, computed locally as
`now() - started_at`.

The status pill has a small live dot when SSE is connected; the
dot turns grey when reconnecting, red when disconnected. This
is the user's "is the page live?" signal.

### Connection state

A subtle row below the status pill:

- `live` — green dot, SSE connected, last event < 5 s ago.
- `reconnecting…` — yellow dot, EventSource is retrying.
  Shows duration of outage.
- `disconnected — retry` — red dot, three failed reconnects.
  Manual retry button.

Today the user has *no* signal that SSE is broken. This is the
single biggest fix in the redesign and it costs ~20 lines of
client code.

### Tabs (Summary / Files / Handover / Stages)

Default to **Summary**. The user clicks across as needed.

- **Summary** — totals (stage count, commits, files changed),
  the final rollup card for completed jobs, the goal from the
  template.
- **Files** — files-produced list (today's `FilesProduced`
  component, unchanged shape).
- **Handover** — the structured handover editor (today's
  Handover tab, unchanged shape, but the user *reads* this far
  more often once A1 lands).
- **Stages** — the stage tree as a navigation aid. Clicking a
  stage *scrolls the conversation to that stage's start
  divider* — it does not replace the centre pane with a stage
  detail view. The stage tree becomes a table-of-contents for
  the conversation.

## SSE reliability

The most visible reliability gap. The fix is layered and lands
before the visual redesign because the redesign assumes SSE is
trustworthy.

### Client-side

- **Last-event-id resumption.** EventSource's built-in
  reconnection passes `Last-Event-ID` automatically *if* the
  server set `id:` on each event. Audit: the daemon emits a
  `cursor: <int>` in the JSON payload but does not put it in
  the SSE `id:` field. Fix: emit `id: <cursor>` so the browser
  reconnects with the right cursor.
- **Resubscribe on disconnect after backoff.** EventSource
  retries indefinitely on its own; that's correct for transient
  network blips. But the UI needs to *render* the retry — that's
  the connection-state badge above.
- **Catch-up replay handling.** When the daemon serves
  cursor > N on reconnect, the client receives the missed
  events in order and folds them into the conversation. No
  user action needed.
- **Heartbeat detection.** Track time-since-last-event. If
  > 30 s with no event (and no heartbeat — see server-side
  below), surface "stale connection" in the connection-state
  badge even if EventSource hasn't fired `error`.

### Server-side

- **Emit `id: <cursor>`** on every SSE event. This is the
  single most important change — without it the browser's
  built-in reconnect-with-`Last-Event-ID` is useless because the
  server has nothing to seek by.
- **Honour `Last-Event-ID`** on subscription. The events handler
  already accepts `since: Option<i64>` as a query param; the
  fix is to also read the `Last-Event-ID` request header and
  use it as the floor when present, preferring it over a
  conflicting `since=`.
- **Heartbeat every 20 s.** Send a `: heartbeat` SSE comment
  line so idle proxies (nginx defaults to 60s read timeout) do
  not silently kill the connection. Comment lines are ignored
  by EventSource but keep the TCP stream alive.
- **Bounded replay window.** The events table in SQLite already
  has every event; querying `WHERE cursor > ? AND job_id = ?`
  is cheap. Cap replay at the most recent 5000 events per
  reconnect to bound memory; if the gap is bigger, fall back to
  "missed events" notice + force a `get_job` + `list_stages`
  refetch.

### Lifecycle integration

When SSE reconnects after a real outage (not just a heartbeat
miss), the conversation shows a system divider:

```
─────── reconnected, replayed 47 missed events ─────── 14:42:11
```

Clicking the divider expands the missed-events count by kind
("12 tool calls, 31 tokens, 2 stage transitions, 2 lifecycle")
so the user can confirm nothing important was lost without
having to dig.

## Live state plumbing

The "cost doesn't update" bug has a specific cause: the UI
fetches `get_job` once on mount and only refetches on a fixed
set of lifecycle events. Cost mutations happen on
`task-completed`, which the JobPage doesn't subscribe to.

The fix is one of two patterns; pick one based on team
preference:

- **Optimistic local apply.** On every `task-completed` event,
  the UI reads the event's `cost_delta_cents` (which the wire
  schema already carries, or trivially can — small additive
  change) and increments the in-memory job row directly. No
  RPC round-trip; the SQLite write is the authoritative copy,
  but the UI is allowed to know it's about to be N cents higher.
- **Subscribe + refetch.** On `task-completed`, fire
  `get_job(id)` and replace the in-memory row. Cheaper to
  reason about; one RPC per task-completed.

Recommend the second. Simpler, no risk of UI drift from
authoritative state, and `task-completed` does not fire often
enough (typically once per minute or two) to be a load concern.

Elapsed time is purely client-side: a `setInterval(1000)` while
status is in `{running, paused, awaiting-review}` that
recomputes `now() - started_at`. Cleans up on unmount or
terminal status. No SSE dependency.

## Migration from the current implementation

This is not a rebuild from zero. The existing pieces map onto
the new model:

| Today | Becomes |
|---|---|
| `RunPane.tsx` (`PhaseStepper`, `RunHeader`, `RunningBody`) | Right-pane status header + stage tab. The visual *bones* survive; the *role* shifts from "primary surface" to "ambient signal". |
| `JobChat` (`RunPane.tsx:795`) | The new `ConversationPane`. Major rewrite — it stops being its own conversation and becomes the unified event/message view. The composer extracted into its own component. |
| `StageDetail` (placeholder cards + the captured-session-id card we just landed) | Hangs off the Stages tab; clicking a stage in the tree scrolls the conversation, and the right-pane card shows that stage's metadata. |
| `useEventStream` | Unchanged interface; underlying client gets the SSE reliability fixes (heartbeat detection, last-event-id wiring). |
| `useJob` | Add a `task-completed` subscription that triggers refetch. |
| Header `[run]` / `[re-run ▾]` | Move into the composer. Header keeps only the badge and the cost/time bars (and even those migrate to the right pane). |
| Three-tab right pane (Spec / Stages / Run) | Spec moves out — editing the spec is a separate route or modal, not a peer tab to the conversation. Run-as-tab disappears (it *is* the conversation). Stages becomes one of the right-pane tabs. |

The `feat/stage-session-id` branch's `Captured` card we landed
yesterday lives on inside the right-pane Stages tab — no rework
needed.

## What this is not

- **Not a re-skin.** The chat-as-control shift is a real model
  change; we are saying "the conversation is the primary
  surface" and that has implications for which components own
  state, which ones are mounted, and where actions originate.
- **Not multi-conversation per job.** One job, one conversation.
  Multi-job views (the JobsDashboard) are unchanged.
- **Not a replacement for the JobsDashboard.** That stays as the
  fleet overview. This doc is only about the per-job page.
- **Not blocked by A0.** The chat-as-control UI ships without
  intra-stage continuation; the pause / resume controls render
  in a degraded form (`[pause]` greys out with a tooltip, or
  becomes `[stop and queue resume]`). Once A0 lands the buttons
  light up. The UI does not need to be re-wired.
- **Not a place to put long-running PTY sessions or terminals.**
  Those stay on their own shell tab (today's `shell` tab in the
  top bar). The job's chat is for *the coding agent*, not for
  arbitrary shell.

## Phasing

Land in this order so each phase is shippable on its own:

### Phase 1 — SSE reliability (S, 1–2 days)

- Daemon: emit `id: <cursor>` on every SSE event.
- Daemon: honour `Last-Event-ID` header.
- Daemon: heartbeat comment every 20 s.
- Client: render connection-state badge.
- Client: stale-stream detection (no event > 30 s, no
  heartbeat).
- Client: live elapsed-time tick.
- Client: `task-completed` subscription on JobPage triggers
  `get_job` refetch.

Ships immediately. Users feel a real difference on every
existing job. **No UI redesign yet.**

### Phase 2 — Conversation pane (M-L, 3–5 days)

- Define the message model and the `ConversationPane`
  component.
- Refactor `JobChat` into the new pane (drops its
  parallel-conversation duplication; reads from the canonical
  event stream).
- Streaming agent messages via `ai-token` buffer.
- Collapsed tool-call cards with expand-on-click.
- Lifecycle dividers.
- Connection-state divider on reconnect.

Ships the new centre column. Right pane stays as-is during
this phase (visual continuity).

### Phase 3 — Composer with state-driven actions (M, 2–3 days)

- Composer component pinned bottom-of-conversation.
- Primary-button state machine.
- Resume / re-run inline forms.
- "Sending while running" UX (degraded until A0).
- Keyboard shortcuts.
- Header `[run]` / `[re-run]` removed.

Ships the actual chat-as-control model. Requires Phase 2.

### Phase 4 — Right-pane refactor (S-M, 2 days)

- Status header migrated; live elapsed tick already done in
  Phase 1.
- Tabs: Summary / Files / Handover / Stages.
- Stage-click scrolls conversation; does not replace it.
- Spec editing moves to a route or modal.

Final layout in place. The three-tab today-shape disappears.

### Phase 5 — A0 integration (lands with the A0 runtime work)

Once A0's `Paused` state and `resume_job` RPC exist:

- Composer's `[pause]` becomes real.
- `[resume ▶ …]` cap-bump form works.
- "Message while running" actually folds into the next prompt.
- The conversation shows real `job-paused` / `job-resumed`
  dividers.

No UI rewrite needed — the components built in Phases 2–3
already expect these states.

## Open questions

These are the calls I'd want made before implementation starts;
none of them are blocking but each one changes a detail.

1. **Composer width on wide viewports.** Constrain the chat
   column to ~800px even on a 4K display so messages stay
   readable, or let it stretch? Recommend constrain; readability
   wins over screen-fill.
2. **Tool-call card density.** One card per tool call, or
   group consecutive same-tool calls (e.g. 5 `Read`s become
   one "Read 5 files" card)? Recommend single cards initially;
   group only if the conversation becomes obviously cluttered
   in real-world use.
3. **Should `[stop]` ever be primary?** A job has *one* primary
   action at a time. When running, `[stop]` is the only action
   the user might take, so it's primary. When the user is
   composing a message, the *send* button is primary and stop
   becomes secondary. The composer's primary slot is
   contextual — message-in-progress wins over status-based
   default.
4. **What happens to the existing `agent_chat` RPC?** Today
   chat goes through a separate runner path (`agent_chat`)
   distinct from the main job runner. Under the new model the
   "agent" in the conversation and the "agent" running the job
   are the same agent. This means `agent_chat` either becomes
   the *only* way to talk to the agent (the runner reuses its
   plumbing) or it's deprecated in favour of "send a user
   message into the job's runner stream." Recommend: defer
   this decision to Phase 3 implementation; the conversation
   pane works with either backing.

## Cross-references

- The autonomy / R-track / U-track ordering: [`PROGRESS.md` "Next steps"](./PROGRESS.md#next-steps).
- The session-continuation rule that makes pause/resume real:
  [`SCOPE.md` hard rule #1](./SCOPE.md#hard-rules-for-the-coding-runner).
- The transport boundary all four shells inherit:
  [`UI-ARCHITECTURE.md`](./UI-ARCHITECTURE.md).
- The wire types the conversation pane reads (`Event`,
  `Job`, `StageRollup`):
  [`codeless/crates/codeless-types/`](../codeless/crates/codeless-types/).
- The captured `Stage.session_id` the resume action uses (on
  `feat/stage-session-id`): [stage.rs](../codeless/crates/codeless-types/src/stage.rs).
