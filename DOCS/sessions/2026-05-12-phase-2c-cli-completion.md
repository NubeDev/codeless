# Build status — Phase 2c CLI completion (real runners + reviews + notifier)

> ⛔ AGENT REMINDER — READ BEFORE TOUCHING THIS FILE
>
> 1. You are running JOB-LOOP. Spec: `DOCS/JOB-LOOP.md`. Project scope:
>    `DOCS/SCOPE.md`. Code-style rules: workspace `CLAUDE.md` and
>    `codeless/CLAUDE.md`.
> 2. One logical batch per tick. Read each stage's `[S|M|L]` tag and
>    batch per JOB-LOOP.md "Hard rules" #3.
> 3. You MUST schedule the next tick before exiting — `CronCreate`
>    recurring:false ~1 min out. If all stages `[x]`, report DONE.
> 4. Update this file in the same commit as the code change.
> 5. ⛔ COMMIT AND PUSH via mani every stage. Never `--force`, never
>    `--no-verify`. If push fails, mark `[!]` and halt.
> 6. ⛔ Comments explain *why*. No emojis. No task-status comments.
> 7. ⛔ R1 cross-platform reach is enforceable. Process spawn stays in
>    `codeless-adapters-host` (and `ai-runner`). Mobile-safe crates
>    must not pull `tokio::process` or `claude-wrapper` transitively.

File: DOCS/sessions/2026-05-12-phase-2c-cli-completion.md
Goal: Finish the remaining SCOPE.md Phase 2 deliverables — CLI
      wiring of the real runners, YAML job-template loader, review
      approval CLI, `codeless tail`, and the outbound notification
      webhook (Notifier trait + generic backend) — so Phase 2 is
      ready for browser-shell work (Phase 3) without further CLI
      churn.
Started: 2026-05-12
Last tick: 2026-05-12 (stage 6 — DONE)
Current stage: DONE (7 / 7)

Repo:        codeless
Branch:      feat/phase-2a-persistence  (Phase 2c stacks on the same
             branch as Phase 2a + 2b — combined PR cuts at end of
             stage 6)
Memory policy: compact every 3 stages
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

## Stages
Format: `[ ] N. [S|M|L] title` — complexity tag mandatory.

- [x] 1. [M] CLI runner selection: `codeless run --runner {claude,
         anthropic,mock}` wires the right adapter from
         `codeless-runtime` into `drive_job`. Default stays `mock`
         so existing tests keep passing. Test uses the fake `claude`
         binary on explicit PATH (per SCOPE testing strategy) and
         asserts events stream through to stdout JSON-line output.
- [x] 2a. [M] Review RPC surface on `codeless-rpc::RpcServer`:
          `list_reviews`, `approve_review`, `comment_review`,
          `stop_review`. Adds the arg/result types in
          `codeless-rpc::methods`, the trait methods on
          `RpcServer`, and the `InProcessRpc` implementations
          that drive the existing review state machine and
          `reviews` table. Unit tests cover each transition
          (approve from AwaitingReview, comment any time, stop
          from AwaitingReview).
- [x] 2b. [S] CLI review surface: `codeless review {list,approve,
          comment,stop}` clap subcommands calling the 2a RPC
          methods. Integration test submits a job, drives it to
          AwaitingReview via a mock runner, and exercises each
          subcommand end-to-end.
- [x] 3. [M] YAML job template loader: `codeless job submit
         job.yaml` parses `{repo, runner, prompt, stages, caps}`
         and calls `submit_job`. Test fixture exercises a 2-stage
         job; round-trip the YAML through serde so a syntax error
         surfaces with line/col.
- [x] 4. [S] CLI tail: `codeless tail <job-id>` subscribes to
         events for the job and streams JSON-line output to stdout
         until terminal status. Reuses the subscriber path that
         `run --once` already exercises.
- [x] 5. [M] Notifier trait + generic webhook backend. Triggers on
         `JobFailed` + `ReviewRequested`. Config lives alongside
         the secrets file (single-tenant). Backend posts JSON to a
         configurable URL with HMAC signing; test against a
         `wiremock` fixture asserting payload shape + signature
         header.
- [x] 6. [S] Phase 2c wrap-up: CODELESS.md refresh, README
         quickstart updated with the new CLI surfaces, three verify
         gates green, branch ready for combined Phase 2a + 2b + 2c
         PR.

Likely batching:
- Tick 1: stage 1 (M).  [done]
- Tick 2: halted — stage 2 split into 2a + 2b.
- Tick 3: stage 2a (M).
- Tick 4: stage 2b (S) + maybe stage 4 (S) if budget allows.
- Tick 5: stage 3 (M).
- Tick 6: stage 4 (S) if not already; or stage 5 (M).
- Tick 7: stage 5 (M).
- Tick 8: stage 6 (S) — wrap-up + DONE.

## Notes
- Phase 2a + 2b are committed and pushed on
  `feat/phase-2a-persistence`. Phase 2c stacks on top; the combined
  PR cuts at the end of stage 6.
- The UI (`codeless/ui/codeless-ui/`) is untouched by Phase 2c — UI
  work is a separate workstream tracked in
  `DOCS/UI-PORT-AUDIT.md`. CLI surfaces here become available to
  the UI as RPC methods automatically.
- New CLI subcommands should preserve the existing JSON-line output
  format so `codeless tail <job-id> | jq` continues to work.

## Notes (tick log)
- Stage 1: `RunnerKind` clap ValueEnum on `RunArgs`; `build_runner`
  dispatches to `MockRunner` / `ClaudeRunnerAdapter` /
  `AnthropicRunnerAdapter`. `--api-key` and `--base-url` flags added
  for the anthropic path; key falls back to `ANTHROPIC_API_KEY`.
  Integration test `run_with_claude_runner_streams_ai_events`
  installs the fake `claude` binary used by phase 2b and asserts
  `ai-token` + `ai-message-complete` + `job-completed` reach stdout.
- Stage 6: Phase 2c wrap-up. `codeless/CODELESS.md` refreshed to
  describe the full CLI surface (`run --runner`, global `--db` /
  `CODELESS_DB`, `job submit`, `review`, `tail`) plus the
  notifier/webhook addition, with a new dated Phase 2c entry under
  durable facts. `codeless/README.md` quickstart now shows each
  subcommand and includes a YAML-template + webhook-config
  example. All three verify gates green at HEAD; branch
  `feat/phase-2a-persistence` carries Phase 2a + 2b + 2c stacked
  and is ready for a combined PR.
- Stage 5: `codeless-runtime::notifier` adds a `Notifier` trait +
  `NotificationPayload` (kind / cursor / ids / event) and
  `spawn_notifier` that subscribes to the bus and dispatches only
  on `JobFailed` / `ReviewRequested`. `WebhookNotifier` is the
  concrete backend: HMAC-SHA256 over the raw JSON body, signature
  on `x-codeless-signature`. Webhook impl landed in
  codeless-runtime (not adapters-host) because adapters-host is
  upstream in the dep graph — moving the trait there would have
  cycled. `WebhookConfig` is TOML-shaped so it can sit alongside
  the existing secrets file later. Tests: live wiremock receives
  both triggers + ignores `JobStarted`, signature verifies with
  the shared key, config round-trips through TOML, empty hex key
  is rejected at setup.
- Stage 4: `codeless tail <job-id>` subscribes via
  `EventFilter::Job` with `since: Some(EventCursor(0))` so the
  replay catches every persisted envelope before going live; `None`
  in the bus contract means "live only", which would silently
  hang on an already-terminal job. JSON-line per envelope, exits
  0 on `job-completed`, non-zero on `job-failed` / `job-stopped`.
  `--timeout-secs` flag (default 600, `0` disables) bounds the
  wait. Integration tests drive a job through `drive_job` +
  `MockRunner` so the events table is fully populated, then
  subprocess the CLI and assert the four framing events plus an
  invalid-id rejection.
- Stage 3: `codeless job submit <file.yaml>` parses a typed
  `JobTemplate` (repo / runner / prompt / branch / stages / caps)
  via `serde_yaml` with `#[serde(deny_unknown_fields)]` so a typo
  like `runneer:` surfaces a line/col parse error rather than
  silently defaulting. The verbatim YAML is forwarded to
  `SubmitJobArgs.template_yaml`; the runtime persists it so the
  original description round-trips off the row. `codeless-cli`
  picks up `serde_yaml = 0.9` and `serde = 1`. Three tests cover
  the round-trip (2-stage template), unknown-field rejection
  (line/col present), and a missing required-field case.
- Stage 2b: `codeless review {list,approve,comment,stop}` clap
  subcommands in `crates/codeless-cli/src/review.rs`. Added a
  global `--db <path>` flag (env: `CODELESS_DB`) and a shared
  `rpc_open` helper that opens either a file-backed pool or the
  in-memory pool; both `run` and `review` route through it.
  `InProcessRpc::with_file(path)` exposes the file pool to keep
  sqlx out of the CLI deps. Integration tests in
  `tests/review_cli.rs` seed a review via `with_file`, then drive
  three subprocess invocations (list → comment → approve →
  conflict-on-reapprove) against the same DB file.
- Stage 2a: `RpcServer` gains `list_reviews` / `approve_review` /
  `comment_review` / `stop_review`. `SqliteStore` gets the matching
  CRUD helpers and a `review_status` label/parse pair. Resolved
  reviews block re-resolution via a `Conflict` (shared
  `resolve_pending_review` helper). Tests in
  `codeless-runtime/tests/reviews.rs` cover approve / stop /
  comment / list-filtering / unknown-id (6 cases, all green).
  Comments do not change status, so post-mortem commentary remains
  possible after Approved / Stopped.

## Blockers
(none)

Resolved 2026-05-12 (tick 2 halt): the original stage 2 assumed
review RPC methods existed on `RpcServer`. They did not. Per
human direction, took option (b) — split into 2a (M, RPC surface)
and 2b (S, clap wiring). Stage count is now 7 instead of 6; total
loop budget unchanged.
