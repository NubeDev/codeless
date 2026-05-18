# CODELESS.md — project memory

This file is the project's per-repo memory. It captures durable facts
about *this codebase* that survive across sessions, ticks, and agents.
Ephemeral context (current task, current branch state) lives in the
**session files** in the parent workspace at
[`../DOCS/sessions/`](../DOCS/sessions/), not here.

## Where to find things

Codeless lives inside a multi-repo workspace. The parent
[`codeless-workspace`](https://github.com/NubeDev/codeless-workspace)
holds the design docs, the bundled `mani` binary, and the vendored
`ai-runner` crate.

- Project scope, all decisions, the crate table, all open questions:
  [`../DOCS/SCOPE.md`](../DOCS/SCOPE.md)
- Agent-facing rules (R1-R5): [`../CLAUDE.md`](../CLAUDE.md)
- Autonomous build loop spec: [`../DOCS/JOB-LOOP.md`](../DOCS/JOB-LOOP.md)
- Loop kickoff template:
  [`../DOCS/JOB-LOOP-KICKOFF.template.md`](../DOCS/JOB-LOOP-KICKOFF.template.md)
- Multi-repo workflow: [`../DOCS/MANI.md`](../DOCS/MANI.md)
- Active session docs: [`../DOCS/sessions/`](../DOCS/sessions/)

## What this repo is, today

Phase 1 (crate skeleton) landed on `feat/bootstrap-cargo-workspace`.
Phase 2a (persistence + queue) and Phase 2b (real runners + worktree
threading + cost) sit stacked on `feat/phase-2a-persistence`:

- `codeless-types` — Repo/Job/Stage/Task/Event/Review structs, serde +
  specta. iOS/Android-safe.
- `codeless-rpc` — `RpcServer` trait, args, results, error variants,
  subscribe surface. iOS/Android-safe.
- `codeless-runtime` — `InProcessRpc` over `SqliteStore` + `EventBus`,
  with the Appendix A schema applied on construction. Events persist
  to the `events` table and the cursor comes from the autoincrement
  column; `subscribe(since)` replays from SQLite and chains to the
  live broadcast tail without gaps or duplicates. Lease-based task
  queue with three-scope concurrency caps (global, per-repo,
  per-runner) lives in `SqliteStore`; `spawn_heartbeat` renews
  leases in a background task and a startup-time reaper inside
  `with_db` reclaims expired leases when the core restarts.
  `Runner` trait + `MockRunner` scripted harness, `drive_job`
  driver, tracing subscriber (`try_init_json` / `try_init_pretty`).
  `ClaudeRunnerAdapter` and `AnthropicRunnerAdapter` wrap the
  vendored `ai-runner` Claude (CLI-wrapped) and Anthropic (REST)
  runners as host-side `Runner` impls, plumbing each upstream event
  through `ai_runner_bridge::forward_events` onto the bus.
  `drive_job` provisions a per-job `git worktree` via an optional
  `WorktreeManager`, threads its path into `RunnerContext`, and
  removes the tree on every terminal exit. A cap watcher inside
  `drive_job` races the runner against `cost_cap_cents` (via the
  rollup that `EventBus::publish` now performs on every
  `ai-message-complete`) and `wall_clock_cap_ms`, firing
  `JobStopped { reason: CostCap | WallClock }` plus a
  `CancellationToken` carried in `RunnerContext`. Cap value `0`
  means unlimited. Host-only.
- `codeless-adapters-host` — `SecretStore` (chmod 0600, atomic-rename
  TOML), `WorktreeManager` (`git worktree add/remove/prune`),
  `ai_runner_bridge::{map_event, forward_events}` translating
  upstream `ai_runner::Event`s into `codeless_types::Event`s through
  a caller-supplied publish closure. Host-only; the only crate
  permitted to spawn processes (ai-runner is the other host-only
  member of the workspace and inherits the same boundary).
- `codeless-cli` — `codeless run --repo <p> --runner {mock,claude,
  anthropic} "<prompt>"` streams JSON-line events through the
  selected adapter; `codeless job submit <file.yaml>` parses a
  typed `JobTemplate` (deny-unknown-fields) and calls `submit_job`;
  `codeless review {list,approve,comment,stop}` drives the review
  RPC methods on `RpcServer`; `codeless tail <job-id>` replays the
  persisted event log and continues live until terminal; `codeless
  secrets {set,get,rm,list}` against the secrets file. A global
  `--db <path>` (env: `CODELESS_DB`) flag opens a file-backed
  SQLite pool — stateful subcommands (`review`, `tail`, `job
  submit`) need it so successive invocations share state; `run`
  works without it via the in-memory pool.
- `codeless-server`, `codeless-client`, `codeless-tauri-desktop` —
  still stubs; Phase 2c+ work.
- Vendored `ai-runner` (sibling crate at `../ai-runner`) is a Cargo
  workspace member; it carries `claude-wrapper`, `anthropic-ai-sdk`,
  `async-openai`. One codeless-side patch routes `RestCfg::base_url`
  through the Anthropic SDK builder so the wiremock integration
  test can redirect the SDK at a local stub; a future re-sync from
  upstream rubix-agent should preserve that wiring.
- `ui/codeless-ui/` — the single React + TS UI for all four shells
  (browser, Tauri desktop, iOS, Android). Terax-derived
  (`crynta/terax-ai` @ pinned SHA in `../DOCS/UI-PORT-AUDIT.md`),
  ~198 source files, already includes editor (CodeMirror 6),
  terminal (xterm.js), file explorer, AI chat panel, settings, and
  themes. The `src/lib/rpc/` boundary holds the typed `RpcClient`
  interface plus `HttpSseClient` (browser/mobile), a `TauriIpcClient`
  stub for desktop, and `MockRpcClient` for tests; shell entries
  under `src/shells/{browser,desktop,android,ios}/` construct the
  right client. Active UI work is the Tauri-conversion grind tracked
  in `../DOCS/UI-PORT-AUDIT.md` — 31 files still import
  `@tauri-apps/*` and need rerouting through `RpcClient` or a
  shell-injected capability adapter. New product surfaces
  (repo-grouped jobs dashboard, per-job stage/task timeline, review
  approval card) mount **inside** the existing Terax shell as new
  modules under `src/modules/`, never as a parallel app.

Verify the workspace any time with:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

All three are green as of the Phase 3a wrap-up commit. The
desktop/mobile Tauri shells remain Phase 5+ work; the HTTP/SSE
server now exists and powers the browser demo.

### Phase 3a — browser demo loop

`codeless-server` is an axum library that exposes every `RpcServer`
method as `POST /rpc/<method>` and the `subscribe` stream as
`GET /events` (SSE). The wire contract is locked by the existing
`ui/codeless-ui/src/lib/rpc/http-sse-client.ts`; bearer auth flows
through the `Authorization` header on REST and a `?token=` query on
SSE (browsers cannot set headers on `EventSource`). CORS is
permissive — single-tenant loopback by default per R5.

`codeless serve` (CLI verb) wires the runtime, the bearer token from
the shared secrets file (key `core_bearer_token`), and the axum
bind. `--init-token` generates a 32-char hex token via the OS
CSPRNG, persists it, and prints it once. The browser-side demo path
is documented in [`../DEMO-UI.md`](../DEMO-UI.md): paste the token
into `localStorage`, run `pnpm -C codeless/ui/codeless-ui dev`, and
the Terax-derived `JobsDashboard` mounts against a real core.

## Durable project facts (update as the project evolves)

Add entries here when a fact becomes load-bearing and isn't already
captured in SCOPE.md. Keep entries short; if something needs more than
a paragraph, write a `DOCS/` page in the parent workspace and link to
it.

- **2026-05-17** — Phase 5 design landed in
  [`../DOCS/SCOPE-TAURI-DESKTOP.md`](../DOCS/SCOPE-TAURI-DESKTOP.md).
  Implementation is *not yet* started; the doc spells out the
  blockers reviewing the Phase 5 kickoff prompt surfaced — no
  monolithic `handle_rpc` (~60 per-method `#[tauri::command]`
  shims), missing `rpc_server_info`, required
  `Channel::id() -> CancellationToken` registry for subscribe/
  unsubscribe, full `InProcessRpc` builder wiring (fs + worktrees +
  agent_chat + assistant_data_dir + `spawn_job_driver_loop` +
  `spawn_heartbeat`), and the `tauri-build` / `tauri.conf.json` /
  `capabilities/` / icons scaffolding the current stub crate lacks.
- **2026-05-12** — Bootstrap. Workspace created at
  `codeless-workspace`; `codeless` repo moved under it; vendored
  `ai-runner` from the rubix-agent workspace; mani.yaml + tasks
  written; CLAUDE.md established. Cargo workspace with 8 crate stubs
  landed on `feat/bootstrap-cargo-workspace`.
- **2026-05-12** — Phase 1 skeleton complete on
  `feat/bootstrap-cargo-workspace` (11 stages, 7 ticks, see
  `../DOCS/sessions/2026-05-12-phase-1-crate-skeleton.md`). End-to-end
  `codeless run --once` works against the mock runner; secrets CLI
  and worktree manager both have integration coverage.
- **2026-05-12** — Phase 2a (persistence + queue) complete on
  `feat/phase-2a-persistence` (9 stages, 8 ticks, see
  `../DOCS/sessions/2026-05-12-phase-2a-persistence.md`). `MemoryStore`
  removed; repos/jobs/stages/tasks/events all live in SQLite via
  `SqliteStore`. Events allocate cursors from the autoincrement
  column; `subscribe(since)` does sqlx-backed replay then live
  broadcast tail with cursor-based dedupe at the boundary.
  Lease-based task queue with atomic three-scope concurrency caps
  (global / per-repo / per-runner), CAS completion/failure/heartbeat,
  `spawn_heartbeat` background helper, and startup-time lease reaper
  inside `with_db`. A resumability integration test
  (`tests/resumability.rs`) opens a file-backed SQLite, lands
  non-trivial state, drops the runtime, rebuilds against the same
  file, and proves repos/jobs/tasks/events all survive and the
  cursor allocator keeps climbing. Real-runner adoption from
  `ai-runner`, worktree threading inside `drive_job`, the HTTP/SSE
  server, and review/notifier surfaces are Phase 2b/2c work.
- **2026-05-12** — Phase 2c (CLI completion + notifier) complete on
  `feat/phase-2a-persistence` stacked on Phase 2b (7 stages — stage
  2 split into 2a/2b after the original spec proved infeasible; see
  `../DOCS/sessions/2026-05-12-phase-2c-cli-completion.md`).
  `codeless run --runner {mock,claude,anthropic}` selects the right
  `Runner` adapter (`--api-key` / `--base-url` flags for the
  Anthropic path; `ANTHROPIC_API_KEY` env fallback). A global
  `--db <path>` / `CODELESS_DB` flag opens a file-backed SQLite
  pool via `InProcessRpc::with_file`; a shared `rpc_open` helper
  keeps `run` and the stateful subcommands on the same pool
  configuration. `RpcServer` grows `list_reviews` /
  `approve_review` / `comment_review` / `stop_review`; their
  `InProcessRpc` impls drive the existing review state machine and
  `reviews` table (Pending → Approved / Stopped, comments preserve
  status). `codeless review {list,approve,comment,stop}` wires the
  four methods to clap subcommands. `codeless job submit
  <file.yaml>` parses a typed `JobTemplate` (repo /runner / prompt
  / branch / stages / caps) via `serde_yaml` with
  `deny_unknown_fields`; verbatim YAML round-trips on
  `SubmitJobArgs.template_yaml`. `codeless tail <job-id>`
  subscribes with `since: Some(EventCursor(0))` so persisted
  envelopes replay before the live tail — `None` means live-only
  and would hang on already-terminal jobs.
  `codeless-runtime::notifier` adds a `Notifier` trait +
  `NotificationPayload` and `spawn_notifier` that subscribes to
  the bus and fans out only `JobFailed` and `ReviewRequested`.
  `codeless-runtime::webhook` is the concrete backend: HMAC-SHA256
  over the raw JSON body, signature on `x-codeless-signature`.
  `WebhookConfig` is TOML-shaped to sit alongside the secrets
  file. The webhook impl lives in `codeless-runtime` rather than
  `codeless-adapters-host` because adapters-host is upstream in
  the dep graph and a host-side trait there would have cycled
  the workspace. Tests pin every surface end-to-end:
  fake-claude-binary CLI run, six review-RPC unit cases,
  three-subprocess review-CLI round-trip with conflict on
  re-approve, two-stage YAML round-trip plus unknown-field/missing-
  field rejection, tail replay-and-exit driven by `MockRunner`,
  and a wiremock webhook fixture that verifies the HMAC against
  the shared key.
- **2026-05-17** — Stage 2 of the `slack-integration` job adds the
  `codeless-slack` crate (host-only, listed under `crates/` in the
  workspace `Cargo.toml`). The crate wraps a Slack Socket Mode
  client (`reqwest` + `tokio-tungstenite/rustls`) and exposes
  `SlackConfig::from_secrets` (reading `slack_app_token` /
  `slack_bot_token` / optional `slack_channel_id` from the existing
  `SecretStore`) plus `SlackBot::spawn` / `spawn_with`, which drive
  a reconnecting Socket Mode session that acks every envelope and
  drops the payload. Command parsing and outbound notifications
  arrive in stages 3/6 of the same job. `codeless serve` grows
  `--enable-slack`; when set, missing secrets surface a warning and
  the server still boots (the bot is additive). `setup/init-session.sh`
  forwards `CODELESS_ENABLE_SLACK=1` as `--enable-slack`. Mobile-safe
  status: crate is host-only per R1 and is not on the mobile compile
  path (mobile shells reach the same RPC surface over HTTP/SSE).
- **2026-05-17** — Stage 3 of the `slack-integration` job adds
  `codeless_slack::command` (pure parser; no I/O). `parse(input,
  ThreadContext)` turns a raw Slack message body into a `Command`
  enum (`ListJobs`, `GetJob`, `StartJob`, `StopJob`, `ResumeJob {
  bypass, comment }`, `Help`) for Surface 1 of SCOPE-SLACK-
  INTEGRATION.md. Thread context resolution: `ThreadContext::for_job`
  supplies an implicit job id for in-thread short forms (`stop`,
  `resume`, `resume bypass`, `resume "<comment>"`); `ThreadContext::
  COLD` rejects those with a precise `MissingJobId` error rather
  than a malformed-id false positive. The `bypass` keyword is a
  positional slot between the ULID and the optional trailing
  quoted comment — `resume <id> bypass "..."` sets bypass=true,
  `resume <id> "comment about bypass"` does not. The quoted-comment
  tokenizer accepts `\"` / `\\` escapes, propagates UTF-8, and
  surfaces an `UnclosedComment` error so the operator's typed
  message is fixable on retry. The parser strips a leading
  `<@USERID>` / `<@USERID|alias>` mention so Slack's `@bot`
  rewrite arrives as plain text. 20 unit tests pin the grammar
  end-to-end; `cargo test --workspace`, `cargo clippy -p
  codeless-slack --all-targets -- -D warnings`, `cargo fmt --check`
  all green. Dispatch wiring lives in stage 4.
- **2026-05-12** — Phase 2b (real runners + worktree threading +
  cost) complete on `feat/phase-2a-persistence` stacked on Phase 2a
  (7 stages, see `../DOCS/sessions/2026-05-12-phase-2b-runners.md`).
  `ai-runner` adopted as a Cargo workspace member (one-line
  `workspace = "../codeless"` patch in its `[package]` table is the
  only required edit on the vendored side). `ClaudeRunnerAdapter` +
  `AnthropicRunnerAdapter` wrap the upstream runners as
  `codeless_runtime::Runner` impls and stream events through
  `ai_runner_bridge::forward_events`. `drive_job` now provisions a
  per-job `git worktree` from an optional `WorktreeManager` and
  removes it on every terminal exit. `EventBus::publish` rolls
  every `AiMessageComplete` into `jobs.cost_cents` + `tasks.cost_cents`
  in the same SQLite pool, which is then read by a cap watcher
  inside `drive_job` that races `cost_cap_cents` and
  `wall_clock_cap_ms` against the runner and fires `JobStopped`
  with the matching `StopReason` plus a `CancellationToken` carried
  in `RunnerContext` so the upstream HTTP/CLI client tears down
  promptly. Tests against a fake `claude` binary set via
  `CLAUDE_BINARY` and against a `wiremock`-hosted Anthropic Messages
  SSE response pin both runners end-to-end through the bridge; a
  separate test pins cap=0 → unlimited.

## What works today (per-Job chat substrate, branch `codeless/job-chat`)

- **Unified chat table (C1 of JOB-CHAT.md).** Migrations
  `0024_chat_messages.sql` and `0025_chat_bindings.sql` carry the
  one-thread-per-Job store; `RpcServer` grows `post_job_message`,
  `list_job_messages`, and `bind_chat_thread`; `EventBus` fans out
  `ChatMessageAppended` / `ChatBindingCreated`; the
  `ui/codeless-ui/` `CHAT` tab renders the stream via `RpcClient`
  (no transport-local store, no `@tauri-apps/*` import); the
  `codeless-telegram` adapter ingests inbound updates as
  `post_job_message(transport=telegram)` and forwards non-origin
  rows through the shared
  `codeless-bot-core::chat_forward::classify` helper. The
  asymmetric echo-suppression rule (origin-transport skips,
  non-origin forwards and writes
  `metadata_json.delivery.<transport>`) is enforced by
  `bot_chat_e2e::origin_transport_skips_self_post` and
  `bot_chat_e2e::cross_transport_forwards_with_receipt`.
- **Supervisor agent, read-only tools (C2).** The supervisor is a
  module inside `codeless-runtime::supervisor` (R1 by construction
  — a module-scoped grep test keeps `tokio::process` and
  `std::process` out of the supervisor tree, and the same test
  enforces that the only outbound voice is `post_chat_message`
  with `transport=supervisor`). `drive_job` spawns one supervisor
  when the Run enters `Running` and aborts it on terminal status;
  a resumed Run spawns a fresh supervisor. The read-only tool set
  (`get_job_state`, `read_events`, `read_handover`, `read_template`,
  `read_stage_log`, `read_notes`) routes through existing
  `SqliteStore` / event-cursor reads. The Claude session wires
  through `ClaudeRunnerAdapter` behind the `supervisor-claude`
  cargo feature; `supervisor/prompt.rs` keeps the system prompt and
  per-tool descriptions as reviewer-readable text under the
  2c-per-turn budget. On Run terminal status the supervisor posts
  a deterministic one-paragraph summary from
  `format_terminal_summary` and exits.
- **Action tools and pre-armed goals (C3).** Migration
  `0026_supervisor_goals.sql` carries `id` / `run_id` / `kind`
  (`deadline-stop` / `threshold-stop` / `event-notify`) /
  `condition_json` / `action_json` / `authorised_by` / `status`
  (`armed` / `fired` / `cancelled` / `superseded`) / `created_at` /
  `fired_at`; `condition_json` and `action_json` are validated
  against a typed enum at write time. `supervisor/tools/actions.rs`
  carries `stop_job` (routes through `runtime::cancel_job` — same
  path as the UI's `[stop]` button) and `add_job_note` (writes
  `runs/<run_id>/notes/<ts>-supervisor.md` and commits). Ad-hoc
  destructive actions get a 5-second preview window with
  cancellation on a fresh `ChatMessageAppended` from a user role
  matching `/^wait\b/i`; pre-armed actions fire immediately per
  JOB-CHAT.md Hard rule 4, with the post-action summary as the
  audit trail. The supervisor's `select!` loop combines the event
  bus, a deadline timer, and a `tokio::time::sleep_until` per armed
  goal; on supervisor boot (including a fresh process after a
  server bounce) `run_with_tools` scans `armed` rows for its
  `run_id`, re-arms the deadline timers, and walks rows whose
  condition is no longer reachable to `superseded` with a chat note
  quoting the reason. The new `codeless-slack` crate mirrors
  `codeless-telegram` one-for-one — same
  `chat_bindings.transport='slack'` rows, same
  `codeless-bot-core::chat_forward::classify` decision, same
  `metadata_json.delivery.slack` receipt shape. Load-bearing tests:
  `supervisor_e2e::deadline_stop_fires_at_t_plus_one_hour`,
  `supervisor_e2e::supervisor_rehydrates_deadline_after_restart`,
  `supervisor_e2e::ad_hoc_stop_aborts_on_user_wait`, and
  `slack_chat_e2e::cross_transport_forwards_with_receipt`.
