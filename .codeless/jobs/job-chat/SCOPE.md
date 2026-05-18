# Scope — job-chat

The full design lives in
**[`DOCS/JOB-CHAT.md`](../../../DOCS/JOB-CHAT.md)** and its three
companion docs:

- [`DOCS/JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md) — the
  iterate loop the supervisor leans on.
- [`DOCS/SCOPE-TELEGRAM-INTEGRATION.md`](../../../DOCS/SCOPE-TELEGRAM-INTEGRATION.md)
  — Telegram surface commands + notifications.
- [`DOCS/SCOPE-SLACK-INTEGRATION.md`](../../../DOCS/SCOPE-SLACK-INTEGRATION.md)
  — Slack surface commands + notifications.
- [`DOCS/JOB-UI.md`](../../../DOCS/JOB-UI.md) — where the `CHAT` tab
  lives in the job page.

This brief is the trimmed per-job scope. **On any disagreement, the
JOB-CHAT doc wins and the brief should be updated, not the doc.**

The closing-trio gate now routes failed trio rails through
auto-bypass with per-rail `failure_detail` surfaced to the UI
([`DOCS/sessions/2026-05-18-trio-gate-failure-routing.md`](../../../DOCS/sessions/2026-05-18-trio-gate-failure-routing.md));
this job inherits that behaviour and **does not change the gate** —
the supervisor talks about gate failures via its read-only tools,
it does not own them.

## Goal

Land the per-Job chat substrate on `master` via the
`codeless/job-chat` branch in three phases, with REVIEW gates at the
phase boundaries. After this job:

1. Every transport (web, Telegram, Slack, CLI) reads and writes the
   same `chat_messages` table through three RPCs
   (`post_job_message`, `list_job_messages`, `bind_chat_thread`).
   A message typed in any surface appears in every bound surface
   within one event tick. No transport maintains its own message
   store (Hard rule 1 of JOB-CHAT.md).
2. A long-lived **supervisor agent** lives as a module inside
   `codeless-runtime` (Hard rule 2 of JOB-CHAT.md + R1 of
   `codeless/CLAUDE.md`). One supervisor per Run-attempt; spawned on
   `Running`, cancelled on terminal status. Its only outbound voice
   is `post_chat_message` with `transport=Supervisor` — no `eprintln`,
   no `tracing::info` that surfaces to a user, no parallel event
   publish.
3. The supervisor exposes read-only tools (`get_job_state`,
   `read_events`, `read_handover`, `read_template`, `read_stage_log`,
   `read_notes`) and action tools (`stop_job`, `add_job_note`,
   `pause_after_stage` parsed-but-noop). Ad-hoc destructive actions
   get a 5-second preview window; pre-armed actions fire immediately
   when their condition trips.
4. A `supervisor_goals` table backs the **pre-armed action loop**.
   Typing "if this runs more than an hour, stop and tell me why"
   in any surface inserts an `armed` row; the supervisor's
   `select!` loop fires the action at the deadline, marks the row
   `fired`, and posts the post-action summary referencing the
   authorising `chat_messages.id`. Goals survive supervisor restart
   via rehydration on boot — the "if-it-runs-more-than-an-hour"
   example must work after a `pkill codeless && cargo run` mid-Run.
5. Slack adapter ships at parity with Telegram (same
   `chat_bindings` shape, same shared echo-suppression helper in
   `codeless-bot-core`).

Together, these three phases turn the existing per-transport chat
silos into one auditable per-Job thread with a long-lived agent
that can both answer questions and act on pre-authorised intent.

## In scope

### Schema (Rust + SQL)

- New migration **`0024_chat_messages.sql`** matching the table from
  [`JOB-CHAT.md` § Data model](../../../DOCS/JOB-CHAT.md#data-model)
  one-for-one, including the partial unique index
  `chat_messages_external_idx` that narrows the constraint to rows
  where `external_id IS NOT NULL` (SQLite treats NULLs as distinct
  in a regular UNIQUE constraint; the partial index documents the
  invariant in SQL, not just prose).
- New migration **`0025_chat_bindings.sql`** with
  `thread_id NOT NULL DEFAULT ''` as the no-thread sentinel — see
  the row-level comment in the doc explaining why NULL is the
  wrong choice for this PK.
- New migration **`0026_supervisor_goals.sql`** (stage 13) with the
  columns from
  [`JOB-CHAT.md` § C3](../../../DOCS/JOB-CHAT.md#c3--action-tools-deadline--threshold-loops).
- Each migration gets row-level comments explaining the invariant
  the column or index enforces — the supervisor reads these when
  asked "why is `thread_id` an empty string instead of NULL?".

### Wire types (`codeless-types`, mobile-safe)

- `ChatMessage`, `ChatTransport`, `ChatRole`, `ChatBinding`,
  `MessageId` newtype, all with `serde` + `specta::Type` derives
  matching the existing `Job` / `Stage` / `Task` style.
- `serde(default, skip_serializing_if = "Option::is_none")` on
  `run_id` and `external_id` so older replay events decode unchanged
  (additive wire field discipline, same as #31's
  `TodoCompleted.failure_detail`).
- Specta wire snapshot regenerated; UI `wire.ts` regenerated via
  `cargo run -p codeless-rpc --example wire_ts`.

### RPC surface (`codeless-rpc` + `codeless-runtime::rpc::chat`)

- `post_job_message`, `list_job_messages`, `bind_chat_thread` on
  `RpcServer`. Impls in a new
  `codeless-runtime/src/rpc/chat.rs` module backed by a new
  `codeless-runtime/src/store/chat.rs` module. Same shape as the
  existing `jobs::list_jobs` / `repos::add_repo` pairs.

### Events (`codeless-types::Event` + bus emit)

- `ChatMessageAppended { job_id, message }` and
  `ChatBindingCreated { transport, channel_id, thread_id, job_id }`
  added to `Event` with the same per-variant `#[serde(rename =
  "...")]` style every other variant uses.
- `post_job_message` publishes `ChatMessageAppended` after the
  INSERT, transactionally so a failed publish leaves no orphan row.
  `StageRecorder` does **not** get a new arm — chat is its own
  write path; the recorder does not own chat persistence.

### Web UI (`ui/codeless-ui/`)

- Rewrite `ChatTab.tsx` over the new RPCs. Remove whatever
  transport-local store the current tab has and replace it with a
  SWR-style cache keyed on `job_id` that rehydrates from
  `list_job_messages` on mount and appends from
  `ChatMessageAppended` on the SSE stream.
- Vitest `chat_tab_round_trips_a_post` pins the input →
  `post_job_message` → `ChatMessageAppended` → render loop using a
  fixture `RpcClient` (no network).

### Transport adapters

- **`codeless-telegram`** — inbound via `post_job_message` with
  `transport='telegram'` + platform message id as `external_id`;
  outbound subscribes to `ChatMessageAppended` and forwards every
  non-telegram-origin message via the Bot API, then writes a
  delivery receipt to
  `chat_messages.metadata_json.delivery.telegram` (new
  `update_chat_message_delivery` store method; **never** UPDATEs
  `body` or `external_id`). `/codeless bind <job_id>` writes to
  `chat_bindings`. Idempotency on adapter restart is presence-based
  on `metadata.delivery.telegram`.
- **`codeless-slack`** — new crate at parity with Telegram. Same
  `chat_bindings.transport='slack'` rows; same shared helper in
  `codeless-bot-core`. Lands in stage 16.
- **Shared echo-suppression + outbound-receipt helper** in
  `codeless-bot-core`. Asymmetric rule: origin-transport skip
  (sender already saw the message), non-origin transports forward
  and write the receipt. Single source of the rule across both
  bots.

### Supervisor agent (`codeless-runtime::supervisor`, module not crate)

- Module placement is **load-bearing for R1**: lives inside
  `codeless-runtime` because it needs the event bus, sqlx, and the
  same RPC handles `runtime` already wires up. **Do not create
  `crates/codeless-supervisor/`** — the boundary is the module, not
  the crate.
- Spawned by `drive_job` when the Run enters `Running`. Subscribed
  to `ChatMessageAppended` filtered to its `job_id`. Cancelled on
  terminal Run status. A fresh Run spawns a fresh supervisor.
- Read-only tools: `get_job_state`, `read_events`, `read_handover`,
  `read_template`, `read_stage_log`, `read_notes`. All route through
  existing RPCs / store reads — the supervisor module imports
  **no** process types. A grep test in `supervisor/mod.rs` enforces
  zero matches of `std::process`, `tokio::process`, `Command::new`.
- Single write tool: `post_chat_message`. The supervisor's only
  voice. Grep-enforced.
- Model is Claude via the existing claude-runner adapter under a
  host-only Cargo feature. System prompt + tool descriptions live
  in `supervisor/prompt.rs` so they are reviewable as text in PR
  diffs.
- On Run terminal status the supervisor posts a one-paragraph
  summary referencing stage names + visible `failure_detail` from
  any failed stage, then exits.

### Action tools + pre-armed goals

- `stop_job`, `add_job_note` route through the existing RPCs and
  emit the same `JobCancelled` / `JobNoteAdded` events the UI
  buttons produce. `events.actor='supervisor'` so the audit trail
  tells the operator which surface triggered the action.
- **Ad-hoc destructive actions** preview for 5 seconds before
  invoking (configurable). A user message matching `/^wait\b/i`
  during the window cancels.
- **Pre-armed actions** fire immediately when their condition
  trips. The post-action summary references the authorising
  `chat_messages.id` and ends up as a `transport=Supervisor` row
  in `chat_messages`. The audit trail is the original "if X then
  Y" user message plus the summary, both immutable rows.
- `supervisor_goals` rows in v0.1: `kind` ∈
  `{deadline-stop, threshold-stop, event-notify}`. `condition_json`
  and `action_json` validated against a typed enum at write time —
  no free-form JSON in the goal types.
- **Rehydration on supervisor boot** is the load-bearing piece for
  the "if-it-runs-more-than-an-hour" example surviving a server
  restart. A fresh supervisor scans `armed` rows for its `run_id`,
  re-arms timers and event watchers, marks rows whose condition no
  longer makes sense as `superseded` with a reason.

### Tests (integration coverage is the load-bearing piece)

- `bot_chat_e2e::origin_transport_skips_self_post` — a Telegram
  inbound row does **not** re-post to Telegram on the outbound
  subscription.
- `bot_chat_e2e::cross_transport_forwards_with_receipt` — a
  `transport=Web` row writes a Telegram delivery and the platform
  id lands in `metadata.delivery.telegram` (not in `external_id`).
- `bot_chat_e2e::cold_load_summary_posts_once` — `/codeless bind`
  posts exactly one condensed summary, not a full history dump.
- `supervisor_e2e::supervisor_spawns_on_run_start_and_exits_on_run_terminal`.
- `supervisor_e2e::supervisor_answers_what_stage_is_it_on` — using
  the mock runner + canned event timeline.
- `supervisor_e2e::ad_hoc_stop_aborts_on_user_wait`.
- `supervisor_e2e::ad_hoc_stop_fires_after_window`.
- `supervisor_e2e::deadline_stop_fires_at_t_plus_one_hour` —
  `tokio::time::pause` + `advance` to make the test deterministic.
- `supervisor_e2e::supervisor_rehydrates_deadline_after_restart` —
  drop the supervisor mid-Run, spawn a fresh one, advance time,
  assert the goal still fires.
- `slack_chat_e2e` mirrors `bot_chat_e2e` against a
  `CannedSlackApi`.
- `chat_tab_round_trips_a_post` (vitest) on the UI side.

### Documentation

- Update **`DOCS/JOB-CHAT.md`** "Status" rows for C1, C2, C3;
  record resolved-OQ outcomes inline (OQ-CHAT-1..5).
- Update **`CODELESS.md`** with one line per landed surface under
  "What works today" (cross-surface chat, supervisor read-only,
  supervisor action + pre-armed goals).
- Per-job session doc under `DOCS/sessions/` written at the end of
  each phase, same shape as
  [`2026-05-18-trio-gate-failure-routing.md`](../../../DOCS/sessions/2026-05-18-trio-gate-failure-routing.md).

## Out of scope

- **`mcp_forward` from PLUGIN-MCP.** Unrelated surface; the
  plugin-substrate-runtimes job covers MCP contribution.
- **Mobile-shell wiring of chat surfaces.** Phase 6 inherits the
  RPC + wire types for free; mobile UI work is its own job.
- **Multi-user trust on shared channels.** R5 single-tenant MVP
  still holds; gated by the `chat_bindings` row having been
  created by the operator. Phase 7 OIDC fixes this properly.
- **Per-message edit semantics.** Edits insert a new row with a
  "replaces" pointer in a follow-up; v0.1 treats edits as new
  messages (OQ-CHAT-1 bias in JOB-CHAT.md).
- **Typed per-transport `metadata_json` enum.** Stays as
  `serde_json::Value` for v0.1; revisit once two transports are
  in production (OQ-CHAT-5).
- **`pause_after_stage` real behaviour.** Parsed and stored as an
  `event-notify` goal but produces a structured `Failed` on fire
  in v0.1 (no-op until JOB-WORKFLOW (A.5) lands).
- **Drive-by refactors of `codeless-bot-core`** beyond extracting
  the echo-suppression + outbound-receipt helper.
- **A separate `codeless-supervisor` crate.** Hard rule 2 of
  JOB-CHAT.md explicitly forbids this; the supervisor is a module.

## Constraints

- **R1 — mobile-safety is testable.** `codeless-types` stays
  mobile-safe; the supervisor module lives in `codeless-runtime`
  (host-only) and never imports `tokio::process` or `std::process`.
  CI verifies via the existing
  `cargo check -p codeless-client --target aarch64-apple-ios` row.
- **R2 — only `RpcClient`.** Web UI's CHAT tab imports nothing from
  `@tauri-apps/*`; chat I/O goes through `RpcClient`.
- **R3 — one UI framework, forever.** No per-shell `ChatTab.web.tsx`.
- **R4 — SQLite is the source of truth.** No transport-local
  message store; the partial unique index is the integrity
  guarantee, not application logic.
- **R5 — single bearer token.** No per-transport auth scopes;
  Telegram and Slack bindings are operator-created and trusted
  channel-wide.
- **Wire-format additivity.** New variants on `Event` and new
  fields on existing variants use `#[serde(rename = "kebab-case")]`
  + `#[serde(default, skip_serializing_if = "Option::is_none")]`
  so older replay events decode unchanged. Same rule the trio-gate
  fix followed for `TodoCompleted.failure_detail` and
  `StageTrioGateWaiting`.
- **`codeless/CLAUDE.md` comment + file rules.** No emojis, no
  decorative banners, no task-status comments; one concept per
  file; no drive-by refactors; no half-finished implementations.
  Comments explain *why*, never *what*.
- **MSRV 1.78** for all Rust changes.
- **`pnpm -C ui/codeless-ui lint`, `pnpm -C ui/codeless-ui test`,
  `cargo test --workspace`, `cargo clippy --workspace --all-
  targets -- -D warnings`, `cargo fmt --check` all green at every
  stage's `checks` trio.**

## Deliverables (what "done" looks like)

1. `codeless/job-chat` branch with one commit per stage, pushed.
2. Three migrations land (`0024`, `0025`, `0026`); fresh-DB and
   replay-idempotent matrix tests green.
3. Three RPCs and two events ship with unit + integration tests.
4. Web UI CHAT tab works end-to-end against the new RPCs; vitest
   green.
5. Telegram and Slack adapters both round-trip messages with no
   double-render; the asymmetric echo-suppression helper is the
   single source of the rule across both transports.
6. Supervisor module spawns / exits on Run state transitions;
   answers "what stage is it on?" using read-only tools;
   `stop_job` from supervisor matches `actor='supervisor'` on the
   `events` row.
7. "If this runs >1h, stop and tell me why" works end-to-end and
   survives a server restart mid-Run.
8. `JOB-CHAT.md` "Status" section gains acceptance rows for C1,
   C2, C3; open questions resolved inline.
9. All workspace builds (cargo + pnpm) green.

## Open questions (resolve in stage 1, before any implementation)

[`JOB-CHAT.md` § Open questions](../../../DOCS/JOB-CHAT.md#open-questions)
ships with five explicit open questions; this job resolves them
inline. The biases here are starting points — record the chosen
answer + one-line *why* under each in **this** file during stage 1.

1. **OQ-CHAT-1 — Edits: UPDATE row, or insert with "replaces"
   pointer.** Bias: insert-new is simpler and keeps the table
   immutable; edits are rare enough that the noise is acceptable.
   Revisit if it becomes painful.
2. **OQ-CHAT-2 — Per-message visibility of the supervisor's
   pre-action preview.** Bias: persist as a normal `chat_messages`
   row, style it differently in the UI if needed. Audit trail wins.
3. **OQ-CHAT-3 — Multi-user trust on shared channels.** Bias:
   trust every channel member to issue `stop_job` for MVP
   (single-tenant). Gated by the `chat_bindings` row having been
   created by the operator. Phase 7 OIDC fixes this properly.
4. **OQ-CHAT-4 — `chat_messages.run_id` purpose.** Bias: UI
   filtering + analytics only. The supervisor's reading view of
   "the chat" is **per-Job** (`job_id`), never per-Run — confirm
   by writing the supervisor's grounding query as
   `list_job_messages(job_id)`, never
   `list_run_messages(run_id)`.
5. **OQ-CHAT-5 — Typed `metadata_json`.** Bias: keep
   `serde_json::Value` for v0.1; revisit once two transports are
   live and the actual shape has settled. Not a blocker for C1.

In addition this job's stage 1 must record:

- **Final v0.1 transport set.** Doc lists Web, Telegram, Slack,
  CLI, Supervisor — confirm this is the set the wire enum ships
  with (no `Discord`, no `Email` in v0.1).
- **Wire-name convention for `ChatTransport`.** Kebab-case in
  serde rename per repo convention (`#[serde(rename =
  "telegram")]` not `"Telegram"`).
- **Whether `run_id` is `Option<RunId>` on the wire today.**
  JOB-WORKFLOW (B) splits Job from Run; pre-(B) it stays `None`
  on every row.

Record the chosen answer + one-line *why* under each in this file
during stage 1. No implementation work in stage 1 except the
documentation edits the resolutions imply.

## References

- Authoritative design:
  [`DOCS/JOB-CHAT.md`](../../../DOCS/JOB-CHAT.md)
- Iterate loop the supervisor leans on:
  [`DOCS/JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md)
- Transport surfaces:
  [`DOCS/SCOPE-TELEGRAM-INTEGRATION.md`](../../../DOCS/SCOPE-TELEGRAM-INTEGRATION.md),
  [`DOCS/SCOPE-SLACK-INTEGRATION.md`](../../../DOCS/SCOPE-SLACK-INTEGRATION.md)
- Where CHAT lives in the job page:
  [`DOCS/JOB-UI.md`](../../../DOCS/JOB-UI.md)
- Trio-gate routing (the failure path the supervisor describes):
  [`DOCS/sessions/2026-05-18-trio-gate-failure-routing.md`](../../../DOCS/sessions/2026-05-18-trio-gate-failure-routing.md)
- Agent rules: [`CLAUDE.md`](../../../CLAUDE.md),
  [`codeless/CLAUDE.md`](../../../CLAUDE.md)
- UI architecture:
  [`DOCS/UI-ARCHITECTURE.md`](../../../DOCS/UI-ARCHITECTURE.md)
- Project scope: [`DOCS/SCOPE.md`](../../../DOCS/SCOPE.md)
