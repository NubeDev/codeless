# Codebase Analysis — codeless-workspace

This document is a comprehensive, file-by-file analysis of the
`codeless-workspace` monorepo as of 2026-05-14. It covers the workspace
shell, the inner `codeless` Rust workspace (10 crates), the vendored
`ai-runner` crate, the `hackline` Rust workspace (5 crates), and the
single React 19 + TypeScript UI that targets four shells.

It is structured by repository, then by crate, then by file. Each file
entry summarises purpose, public types, public functions, and any
load-bearing private helpers. Tests are listed at the end of each crate
with one-line purposes.

For "why" rather than "what", read `DOCS/SCOPE.md` (product scope and
architecture decisions) and `CLAUDE.md` (agent rules R1-R5). This file
documents what is in the tree.

---

## 1. Workspace shell — `codeless-workspace/`

The workspace shell is the outer repo (`NubeDev/codeless-workspace`,
public). It tracks shared tooling and docs; the inner `codeless/` repo
is colocated but independent, and `ai-runner/` is vendored from
`rubix-agent` with no `.git` of its own.

### 1.1 Top-level files

| Path | Purpose |
|------|---------|
| `CLAUDE.md` | Agent rules: crate dependency direction (R1), single RPC transport (R2), one UI framework (R3), SQLite truth (R4), single-tenant trust (R5); file-level rules on comments, drive-by refactors, partial implementations. |
| `mani.yaml` | mani task config for the multi-repo workflow. Projects: `codeless` (active), `ai-runner` (vendored read-only). Tasks: `status`, `fetch`, `pull`, `branch`, `diff`, `commit` (requires `MSG=`), `push`, `wire-ts-check`. |
| `bin/mani` | Bundled mani binary; do not replace casually. |
| `ai-runner.PATCHES.md` | Log of every codeless-side edit to vendored `ai-runner/`. Each patch lands a `// codeless-patch-NNN` marker in source. See §3. |
| `.gitignore` | Excludes inner repos (`codeless/`, `terax-ai/`), build artefacts (`target/`, `node_modules/`, `dist/`), SQLite DBs, worktrees, secrets, editor junk. |
| `DEMO-UI.md` | End-to-end browser demo flow: `codeless demo bootstrap` → `codeless serve` → `pnpm dev` → walk through editor / Jobs / Submit / Handover. Includes Claude-runner path, dogfooding via `--target-self`, and 15 UX polish items. |

### 1.2 `DOCS/`

| File | Purpose |
|------|---------|
| `SCOPE.md` | Product scope, all decisions, crate dependency table, open questions. SCOPE.md wins on conflicts. |
| `JOB-LOOP.md` | Autonomous build loop spec: stage batching by complexity (S/M/L), commit+push every stage via mani, schedule next tick, no `--force`/`--no-verify`. |
| `JOB-LOOP-KICKOFF.template.md` | Template for starting a loop session. |
| `JOB-MODEL.md` | User-facing contract: `.codeless/jobs/<name>.yaml` template, `runs/<id>/handover.md`, `runs/<id>/log.md`. Long unsupervised runs. |
| `JOB-WORKFLOW.md` | Iterate loop: edit template between runs, inject feedback notes, re-run with prior handover. |
| `JOB-DIR.md` | Job directory structure and semantics. |
| `JOB-DIR-KICKOFF.md` | Bootstrap procedure for a new job directory. |
| `JOB-EXAMPLE.md` | Worked example of the job-dir lifecycle. |
| `MANI.md` | Multi-repo orchestration with mani; why scripted ticks use mani instead of raw git. |
| `UI-ARCHITECTURE.md` | One codebase, four shells (Tauri desktop, browser, iOS, Android) via swappable `RpcClient` adapters. Load-bearing for R2/R3. |
| `UI-PORT-AUDIT.md` | File-by-file conversion list: Terax-coupled call sites → `RpcClient`. |
| `AGENT-CONTROL-PLANE.md` | Control plane architecture for multi-agent orchestration. |
| `AGENT-CONTROL-PLANE-USAGE.md` | Usage guide for the control plane. |
| `LOOP-CODER.md` | 24-hour unsupervised run design; fresh session per stage. |
| `DOGFOOD-MVP.md` | Golden path: codeless develops codeless from the browser. |
| `PROGRESS.md` | Tracking: shipped on master, in flight per stage, next tracks (autonomy / real-codebase). |
| `MOXXY-INTEGRATION.md` | Integration notes for the moxxy peripheral. |
| `JOBS-UX.md` | Jobs-tab UX spec. |
| `TOOLS-PORTING.md` | Tools porting notes. |
| `START-SERVER-UI.md` | Server + UI start instructions for contributors. |
| `sessions/` | Active session docs (per-loop progress files). |

### 1.3 `scripts/`

| Script | Purpose |
|--------|---------|
| `smoke-demo.sh` | Headless mock demo: build CLI, seed `/tmp` DB + repo, start server on `127.0.0.1:7799`, poll `list_jobs` for completion. Regression guard. |
| `smoke-claude-demo.sh` | Real Claude runner end-to-end: init git repo, register via `demo bootstrap`, start `codeless serve --enable-claude`, submit job to create `hello.txt`, poll for terminal state, verify commit. Requires `claude` binary + auth. |

---

## 2. Inner repo — `codeless/`

The inner `codeless/` repo is its own git project with branches, PRs, and
history independent of the workspace. The workspace `.gitignore` excludes
it.

### 2.1 Workspace-level configuration

| File | Purpose |
|------|---------|
| `Cargo.toml` | Workspace manifest. Edition 2021, MSRV 1.78. Members: 10 codeless crates + vendored `../ai-runner`. Workspace deps: async-trait, thiserror, serde, serde_json, tokio, tracing, futures-util. Lints: forbid unsafe; Clippy warn-all priority -1. |
| `Cargo.lock` | Locked dependency graph. |
| `Makefile` | Dev targets backed by `.codeless-dev/` state dir: `start`, `stop`, `restart`, `backend[-fg]`, `ui[-fg]`, `demo-seed`, `logs`, `status`, `clean`. Defaults `127.0.0.1:7777` backend, `127.0.0.1:5173` UI. |
| `CLAUDE.md` | Project-specific agent rules (subset of workspace CLAUDE.md). |
| `CODELESS.md` | Per-repo project memory: pointers to SCOPE/JOB-LOOP/MANI, current phase, core component sketch. |
| `README.md`, `NOTICE` | Standard meta. |
| `examples/jobs/hello-gin.yaml` | Sample multi-stage Go/Gin template exercising the full JOB-MODEL.md shape (name, goal, stages: scaffold → `go mod tidy` → README). |

### 2.2 `crates/codeless-types/` — domain types (iOS/Android-safe)

ULID-based IDs, status enums, wire types. No I/O, no async. Re-exports
all submodules from `lib.rs`.

#### `src/id.rs`
ULID newtypes generated via macro. `RepoId`, `JobId`, `StageId`,
`TaskId`, `ReviewId` — all expose `new()`, `from_str()`, `Display`.

#### `src/git_auth.rs`
`GitAuth` enum: `Ssh { key_path }`, `Token { env_var }`, `GithubApp
{ app_id, installation_id }`.

#### `src/money.rs`
`CostCents(i64)` transparent newtype; `as_i64()`, `const ZERO`.

#### `src/time.rs`
`UnixMillis(i64)` transparent newtype; `as_i64()`, `const ZERO`.

#### `src/repo.rs`
`Repo { id, name, clone_url, default_branch, local_path, git_auth,
concurrency_cap, default_runner, created_at, updated_at }`.

#### `src/fs.rs`
`FsEntry { name, kind, size, mtime }` and `FsEntryKind { File, Dir,
Symlink }`.

#### `src/job.rs`
`JobStatus` (Draft, Queued, Running, AwaitingReview, Completed, Failed,
Stopped, Paused), `WorkspaceMode` (InRepo, Worktree), `StopReason`
(User, CostCap, WallClock, RunnerCrash). `Job` struct holds id, repo,
status, stop_reason, template_yaml, prompt, runner, branch,
workspace_mode, worktree_path, cost/wall-clock caps, current cost,
model, permission_mode, effort, started_at, ended_at, created_at.

#### `src/stage.rs`
`StageStatus` (Pending, Running, AwaitingReview, Passed, Failed).
`Stage { id, job_id, ordinal, name, status, verify_cmd, started_at,
ended_at, session_id }`. `session_id` enables runner resume.

#### `src/task.rs`
`TaskStatus` (Enqueued, Running, Completed, Failed, Cancelled). `Task
{ id, stage_id, ordinal, status, depends_on, lease_holder,
lease_expires_at, cost_cents, input_tokens, output_tokens, started_at,
ended_at }`. Lease fields drive CAS-based queue.

#### `src/review.rs`
`ReviewStatus` (Pending, Approved, Rejected, Stopped, RerunRequested).
`Review { id, stage_id, status, comment, requested_at, resolved_at }`.

#### `src/event.rs`
`EventCursor(i64)` monotonic from SQLite `AUTOINCREMENT`. `Event` enum
with ~28 variants spanning repo/job/stage/task/review lifecycle:
- Repo: `RepoAdded`, `RepoRemoved`, `RepoUpdated`
- Job: `JobQueued`, `JobPromoted`, `JobStarted`, `JobCompleted`,
  `JobStopped`, `JobFailed`, `JobPaused`, `JobResumed`
- Stage: `StageStarted`, `VerifyStarted`, `VerifyPassed`,
  `VerifyFailed`, `StageCompleted`, `StageSessionCaptured`
- Task: `TaskEnqueued`, `TaskStarted`, `ToolCall`,
  `ToolApprovalRequested`, `AiToken`, `AiMessageComplete`,
  `TaskCompleted`
- Review: `ReviewRequested`, `ReviewApproved`, `ReviewCommented`,
  `ReviewStopped`

`EventEnvelope { cursor, job_id, stage_id, task_id, created_at, event }`
wraps `Event` with metadata.

#### `src/handover.rs`
`Handover { done, next, what_you_need_to_know, open_questions }` —
inter-session contract markdown. `to_markdown()` renders fixed `## `
headings + bullet lists. `from_markdown()` parses tolerantly.
`HandoverParseError::UnknownSection(String)`.

#### Tests
- `tests/specta_snapshot.rs` — Snapshot of Specta TypeScript codegen for
  type drift detection.
- `tests/serde_wire.rs` — Wire-format serde round-trips.

### 2.3 `crates/codeless-rpc/` — RPC contract (iOS/Android-safe)

Single `RpcServer` trait that all transports implement. Argument and
result types derive `specta::Type` for TS codegen.

#### `src/error.rs`
`RpcError` enum: `NotFound`, `InvalidArgument`, `Conflict`, `Internal`.
`RpcResult<T> = Result<T, RpcError>`.

#### `src/subscribe.rs`
`EventFilter::All | Job { job_id }`. `Since = Option<EventCursor>`.
`EventStream = Pin<Box<dyn Stream<Item = Result<EventEnvelope,
RpcError>> + Send + 'static>>`.

#### `src/methods.rs`
~40 typed arg/result structs. Notable: `AddRepoArgs`, `SubmitJobArgs`,
`ListJobsResult`, `StageRollup` (stage + cost/task_count), `JobDiffArgs`,
`JobDiffFile`, `GcWorktreesResult`, `ListJobFilesResult`,
`WriteHandoverArgs`, `AgentChatArgs` (with `ChatContext` and
`ChatAttachmentRef`), `UploadChatAttachmentArgs`, `FsReadDirArgs`,
`FsStatResult`, `ServerInfo` (with `RunnerInfo`, `ClaudeStatus`).

#### `src/server.rs`
`RpcServer` async trait (object-safe via `async_trait`) — 33 methods
covering: repo CRUD, job lifecycle (`submit_job`, `start_job`,
`stop_job`, `pause_job`, `resume_job`, `rerun_job`), `list_stages`,
review gates (`approve_review`, `comment_review`, `stop_review`),
`subscribe`, fs operations, job-file operations, `update_job_template`,
`write_handover`, `agent_chat`, `upload_chat_attachment`,
`cancel_chat_task`, `stop_active`, `gc_worktrees`, `job_diff`.

#### `src/lib.rs`
Re-exports the public surface (`RpcServer`, `RpcError`, `EventFilter`,
`Since`, `EventStream`, all method types).

#### Tests / examples
- `tests/specta_snapshot.rs` — RPC type snapshot test.
- `examples/wire_ts.rs` — Regenerator for the TS contract used by the
  UI; `mani wire-ts-check` runs this and diffs output.

### 2.4 `crates/codeless-runtime/` — core runtime

The heart of the system: SQLite store, event bus, queue, job driver,
state machine, runners, migrations, tracing. 36 src files; tests cover
each subsystem.

#### `src/lib.rs`
Public API and module surface. Re-exports: `EventBus`, `SubscribeFilter`,
`InProcessRpc`, `ChatCancelEntry`, `ChatCancels`, `SqliteStore`,
`Runner`, `RunnerContext`, `RunnerOutcome`, `MockRunner`, `MockStep`,
`NotificationKind`, `NotificationPayload`, `Notifier`, `NotifierError`,
`WebhookConfig`, `WebhookNotifier`, `WebhookSetupError`, `QueueConfig`,
`DriverLoopHandle`, `RunnerFactory`. Functions: `drive_job`,
`spawn_job_driver_loop`, `spawn_heartbeat`, `spawn_stage_recorder`,
`spawn_notifier`, `transition_job/stage/task`, `is_terminal_job`,
`parse_permission_mode`, `now_ms`, `try_init_json/pretty`, `MIGRATOR`.

#### `src/driver.rs`
`drive_job(ctx)` runs one Queued job to terminal state. Order: validate
Queued, provision worktree (or in-repo branch), transition Running and
publish `JobStarted`, spawn cap watcher, invoke `runner.run(ctx)`, on
return check whether watcher already terminated the job; if not,
translate `RunnerOutcome` to Completed/Failed, write `handover.md`,
append to `log.md`. Worktree is preserved on disk after the run for user
inspection. Private helpers: `ProvisionedWorktree`, `provision_worktree`,
`provision_in_repo`, `spawn_cap_watcher` (concurrent watch on cost_cap +
wall_clock + external stop/pause via `WatcherAction`), `fire_pause_or_stop`
(picks Paused vs Stopped based on `has_captured_session`),
`has_captured_session`.

#### `src/job_driver_loop.rs`
`spawn_job_driver_loop` returns `DriverLoopHandle { cancel, join }`. The
loop replays backlog (list_jobs → filter Queued → dispatch), then
subscribes to `JobQueued / JobPromoted / JobResumed` and dispatches
each. Concurrency bounded by `tokio::sync::Semaphore`. `dispatch`
augments the prompt in fixed order: prior handover → job docs →
original prompt; docs come from `template.yaml` `docs:` list or
auto-discovered `*.md` (SCOPE.md first, WORKFLOW.md second, then
alphabetical). `trait RunnerFactory` lets the binary choose how to
construct runners (mock vs claude vs anthropic vs template).

#### `src/state_machine.rs`
Pure transition guards. `transition_job(from, to) → Result<(),
TransitionError>` enforces edges per SCOPE.md: Draft → Queued|Stopped;
Queued → Running|Stopped; Running → AwaitingReview|Completed|Failed|
Stopped|Paused; AwaitingReview → Running|Completed|Stopped|Paused;
Paused → Queued|Stopped; Stopped|Failed → Queued (rerun).
`transition_stage`: Pending → Running; Running → AwaitingReview|Passed|
Failed; AwaitingReview → Running|Passed|Failed. `transition_task`:
Enqueued → Running|Cancelled; Running → Completed|Failed|Cancelled.
`is_terminal_job` true iff status ∈ {Completed, Failed, Stopped}.

#### `src/runner.rs`
`enum RunnerOutcome { Completed, Failed { reason } }`. `struct
RunnerContext { job_id, stage_id, bus, worktree_path, cancel }`. `trait
Runner` with single `async fn run(ctx) -> RunnerOutcome`. Contract:
runner publishes only stage/task/AI events; driver owns Job-row
transitions. Cancel token fires on cap breach, stop, or pause.

#### `src/event_bus.rs`
Two-stage publication: INSERT to `events` table, then broadcast.
`EventBus::publish` rolls up cost on `AiMessageComplete` (`roll_up_cost`
updates `jobs.cost_cents`). `subscribe_since` returns an `EventStream`
with gap-free catch-up: open broadcast subscription before SELECT,
INSERT before broadcast::send, filter live tail by cursor > max_seen.
`enum SubscribeFilter { Job(JobId), Stage(StageId), Task(TaskId), All }`.

#### `src/store.rs`
`SqliteStore` over `sqlx::SqlitePool` + `QueueConfig`. Status enums
serialised to kebab-case wire labels (SCOPE.md Appendix A) via
`job_status_label` / `workspace_mode_label` and parsed via `job_from_row`
/ `repo_from_row`. CRUD across `repos`, `jobs`, `stages`, `tasks`,
`reviews`, `events`. Key methods: `insert_repo`, `list_repos`,
`insert_job`, `update_job`, `list_jobs`, `insert_stage`,
`list_stages_for_job`, `insert_task`, `heartbeat_task`, `lease_next`
(CAS task claiming honouring per-repo/per-runner/global caps),
`release_expired_leases` (reaped on startup), `insert_review`,
`approve_review`, `comment_review`.

#### `src/rpc.rs`
`InProcessRpc` implements `codeless_rpc::RpcServer`. Owns
`SqliteStore`, `EventBus`, optional `HostFs`, `WorktreeManager`, an
`ai_runner::Registry` for the chat panel, the chat CWD, and a chat
cancel map (`Arc<Mutex<HashMap<TaskId, ChatCancelEntry>>>`).
Constructors: `new()` in-memory, `with_file(path)`,
`with_db(pool)` (caller-supplied pool, applies `MIGRATOR`). Fluent
setters: `with_fs`, `with_worktrees`, `with_agent_chat`. Each RPC
method maps to a store call or a side-effecting helper.

#### `src/migrations.rs`
`MIGRATOR = sqlx::migrate!("./migrations")` — compile-time embedded,
forward-only, content-hashed.

#### `src/queue_config.rs`
`QueueConfig { max_global, max_per_repo, max_per_runner }`, each
`Option<u32>`.

#### `src/runner.rs` adapters

- **`claude_runner.rs`** — `ClaudeRunnerAdapter` wraps
  `ai_runner::ClaudeRunner` to implement codeless's `Runner`. Spawns an
  mpsc forwarder task draining upstream events to `bus.publish()`,
  builds a `CliCfg`, invokes `run()`, joins both. Maps
  `RunResult::error` to `RunnerOutcome::Failed`. `DEFAULT_SYSTEM_PROMPT`
  is the headless instruction telling claude to use file tools directly
  (no script-emit), infer language, commit work, emit structured
  handover. `parse_permission_mode` converts wire label to
  `ai_runner::PermissionMode`.
- **`anthropic_runner.rs`** — Mirror adapter for `RestCfg` /
  Anthropic-API runs. Same mpsc + forwarder + outcome-map pattern.
- **`mock_runner.rs`** — Scripted test runner. `MockStep::Emit(Event) |
  Sleep(Duration) | Finish(RunnerOutcome)`. `MockRunner` is a
  `Mutex<Vec<MockStep>>` consumed in order.
- **`template_runner.rs`** — Multi-stage runner. Per stage:
  `StageStarted` → inner `ClaudeRunnerAdapter` (with `resume_id =
  stage.session_id` when present) → `StageCompleted`. REVIEW stages
  emit review-requested but do not block (orchestration gap
  acknowledged). Caps are job-level.

#### `src/notifier.rs`
`trait Notifier { async fn notify(payload) -> Result<(),
NotifierError> }`. `enum NotificationKind { JobFailed,
ReviewRequested }`. `NotificationPayload { kind, cursor, job_id,
stage_id, review_id, created_at, event }`. `spawn_notifier(bus,
sink)` subscribes live (no replay), filters for the two kinds, calls
`sink.notify`. `NotifierError { Transport, Status }`.

#### `src/webhook.rs`
HTTP backend for `Notifier`. `WebhookConfig { url, hmac_key_hex }`
hex-decoded at construction. `WebhookNotifier` POSTs payload as JSON,
signs with HMAC-SHA256 on `X-Codeless-Signature`. Setup errors:
`Hex | EmptyKey`.

#### `src/stage_recorder.rs`
`spawn_stage_recorder(bus, store)` subscribes live, persists Stage and
Task rows from events so the Stages tab can query rolled-up state
without reconstructing from the events table. Live-only subscription —
restart drops backlog; events table is the durable record.

#### `src/heartbeat.rs`
`spawn_heartbeat(task_id, period, pool)` renews `tasks.lease_expires_at`
via `store.heartbeat_task`. Zero rows affected → lease lost (return).
DB error → retry next tick.

#### `src/handover.rs`
`handover_path(worktree, job_id)` → `<wt>/runs/<id>/handover.md`.
`write_handover` async write with parent dir creation.
`extract_handover(text)` finds a ```` ```handover ```` fenced block and
calls `Handover::from_markdown`. `fallback_handover_from_text` synthesises
a `Handover` from the assistant tail (truncated). Private:
`find_fenced_block` case-insensitively matches ```` ```handover ```` or
```` ```handover-md ````.

#### `src/job_dir.rs`
`enum JobLayout { None, Flat, Directory, FlatPreferred }`. `resolve`
checks existence; `flat_yaml_path`, `directory_path`,
`template_yaml_path` build paths; `list_markdown` returns sorted `*.md`
under the directory.

#### `src/session_log.rs`
Append-only `runs/<id>/log.md`. `enum EndReason { Completed, Failed,
Stopped }`. `log_path` returns the path. `append_session_block` adds
one `## Session N` block, counting existing headings to number the new
one. Includes cost + end reason.

#### `src/template.rs`
`JobTemplate { name, goal, docs: Option<Vec<String>>, stages:
Vec<StageSpec> }`. `StageSpec { title, review: bool, docs:
Option<Vec<String>> }`. Bare-string and `REVIEW <title>` flat stages
parse into the same structured shape.

#### `src/time.rs`
`now_ms() -> UnixMillis` (UNIX_EPOCH ms, clamped to i64::MAX).

#### `src/tracing_init.rs`
`try_init_json` for hosted mode, `try_init_pretty` for dev/CLI. Default
filter: `info,sqlx=warn,hyper=warn`.

#### Tests (in `tests/`)
`job_dir_workflow.rs`, `notifier.rs`, `heartbeat.rs`, `stop_active.rs`,
`task_queue.rs`, `fs.rs`, `since_replay.rs`, `pause_job.rs`,
`migrations.rs`, `tracing_init.rs`, `cost_rollup.rs`,
`event_persistence.rs`, `queue_caps.rs`, `job_worktree.rs`,
`rpc_in_process.rs`, `rpc_with_db.rs`, `resumability.rs`,
`claude_runner.rs`, `anthropic_runner.rs`, `reviews.rs`, `rerun_job.rs`,
`gc_worktrees.rs`, `cap_cancellation.rs`, `job_driver.rs`,
`chat_cancel.rs`, `resume_job.rs`. Each tests one subsystem end-to-end
against an in-memory `InProcessRpc`.

### 2.5 `crates/codeless-server/` — HTTP+SSE transport

#### `src/lib.rs`
`AuthMode { Required(token), Open }` — `Open` only valid for loopback.
`AppState { rpc: Arc<dyn RpcServer>, auth: AuthMode, info: ServerInfo }`.
`build_router(state)` builds the axum router; `serve_with_shutdown(addr,
state, on_bound)` binds and serves until SIGINT.
`load_bearer_token(&SecretStore) → Result<String, TokenLoadError>`.
`const TOKEN_SECRET_KEY = "core_bearer_token"`.

#### `src/main.rs`
Placeholder binary directing users to `codeless serve`.

#### `src/auth.rs`
`bearer_layer` axum middleware validates `Authorization: Bearer`,
`constant_time_eq` for timing-safe compare. `TokenLoadError` carries a
helpful hint.

#### `src/routes.rs`
`router(state)` mounts every RPC method at `POST /rpc/<method>`,
unauth'd `/healthz`, `/version`, `/server/info`, plus CORS and trace
layers. `map_err(RpcError) → (StatusCode, String)` does the standard
NotFound→404, InvalidArgument→400, Conflict→409, Internal→500 mapping.

#### `src/sse.rs`
`EventsQuery { scope, job_id, since, token }`. `events_handler` returns
an `Sse<Stream>` with keep-alive, auth checked via query token (since
EventSource cannot set headers).

#### Tests
- `tests/routes.rs` — Integration tests for every route + auth + SSE.

### 2.6 `crates/codeless-client/` — HTTP + SSE client

Implements the same `RpcServer` trait against the server crate.

#### `src/http_client.rs`
`HttpRpcClientConfig { base_url, token }`. `HttpRpcClient` is the impl
over a pooled `reqwest::Client`. Generic `call<A,R>(method, args)` and
`call_void` do POST + body parse + status→RpcError mapping. `subscribe`
opens an SSE connection with `?token=` query param. `ClientError`
captures transport-layer failures.

#### `src/sse.rs`
`SseParser` — minimal stateful `text/event-stream` parser handling
frame buffering across TCP chunks (CRLF-tolerant). `feed(bytes) →
Vec<Result<EventEnvelope, RpcError>>`.

#### Tests
- `tests/round_trip.rs` — End-to-end client/server round trip with mock
  server.

### 2.7 `crates/codeless-cli/` — CLI binary

`codeless` is the human-facing entry point. Global flags:
`--secrets-file`, `--db`, `--core <url>`, `--token <bearer>`.
Dual-mode verbs pick `InProcessRpc` (local) vs `HttpRpcClient` (hosted)
via `rpc_open::build_dual_mode`.

#### `src/main.rs`
Subcommand dispatch.

#### `src/secrets.rs`
`codeless secrets {set,get,rm,list}`. `set` reads value from inline arg,
`--from-env VAR`, or stdin. `get` refuses without `--reveal`. `list`
prints names only.

#### `src/run.rs`
`codeless run --once <prompt> --repo <path> --runner {mock|claude|
anthropic}` — register repo, submit job, drive runner to completion,
stream events as JSON lines.

#### `src/serve.rs`
`codeless serve`. Flags: `--bind`, `--init-token [--force]`,
`--no-driver`, `--driver-concurrency`, `--worktree-root`,
`--enable-claude`, `--enable-anthropic`, `--fs-root`, `--require-token`.
`handle` builds runtime, spawns driver loop, builds router, serves.
`init_token` writes a random 32-char hex into secrets. `run_server`
wires the default `RunnerFactory` (mock/claude/anthropic selection +
TemplateRunner for template jobs). `build_server_info` populates
`ServerInfo` with claude probe result and CLI-runner readiness.

#### `src/review.rs`
`codeless review {list, approve, comment, stop}` — local-mode only
(stateful SQLite).

#### `src/rpc_open.rs`
`open(db) → InProcessRpc` and `build_dual_mode(core, token, db) →
Arc<dyn RpcServer>` for verb implementations.

#### `src/repos.rs`
`codeless repos {list, add, remove}` — dual-mode.

#### `src/jobs.rs`
`codeless jobs {list, get, stop}` — dual-mode.

#### `src/tail.rs`
`codeless tail <job-id>` — replays from cursor 0, streams as JSON
lines, exits on terminal event. `--timeout-secs` cap.

#### `src/job.rs`
`codeless job submit <file.yaml>` — parses typed YAML template (denies
unknown fields), forwards as `template_yaml`, prints resulting Job.

#### `src/cost.rs`
`codeless cost summary [--repo] [--json]` — dual-mode cost rollups by
status and runner.

#### `src/demo.rs`
`codeless demo bootstrap` — idempotent seeding of demo repo + mock job
into the configured DB. Flags for name, local-path, clone-url, branch,
prompt.

#### Tests
`jobs_dual_mode.rs`, `serve_tracing.rs`, `serve_driver.rs`,
`serve_notifier.rs`, `repos_hosted_cli.rs`, `tail_cli.rs`,
`serve_cli.rs`, `tail_hosted.rs`, `review_cli.rs`, `secrets_cli.rs`,
`job_submit.rs`, `cost_summary.rs`, `run_once.rs`. Each verb has at
least one CLI-level integration test.

### 2.8 `crates/codeless-mcp/` — MCP server surface

Exposes the `codeless-tools` registry as an MCP server.

#### `src/lib.rs`
Re-exports `CodelessMcpHandler`, `ServerContext`, `serve_stdio`.

#### `src/main.rs`
Builds a default `ToolRegistry` (BrowseFetchTool, HttpRequestTool) and
serves over stdio. Env: `CODELESS_WORKTREE_ROOT`, `RUST_LOG`.

#### `src/server.rs`
`ServerContext { registry, worktree_root, network_mode, allowlist,
cancel }`. Methods `new`, `with_network`, `build_tool_ctx` (per-call
child cancel token), `serve_stdio`.

#### `src/handler.rs`
`CodelessMcpHandler` implements `rmcp::handler::server::ServerHandler`.
`get_info` returns server capabilities + version (MCP 2024-11-05).
`list_tools` enumerates the registry; `call_tool` dispatches via
`ToolCtx`. `ToolError::Cancelled|Denied` surface as `is_error: true`;
`InvalidArgs` becomes protocol error.

#### Tests
- `tests/stdio_handshake.rs` — MCP handshake over stdio.

### 2.9 `crates/codeless-adapters-host/` — host adapters (process-spawning)

Per R1, this is the only place `tokio::process` / `std::process` is
permitted. All host-side adapters live here.

#### `src/lib.rs`
Module manifest. Re-exports adapters.

#### `src/ai_runner_bridge.rs`
Translates `ai_runner::Event` → codeless `Event`. `map_event(ev,
task_id) -> Option<Event>` (Text→AiToken, ToolUse→ToolCall,
Done→AiMessageComplete; Connected/Error dropped). `forward_events<F,
Fut, E>(rx, task_id, publish)` drains mpsc, calls publisher closure
each item. `usd_to_cents(f64) -> CostCents` rounds and clamps negatives.

#### `src/secrets.rs`
`SecretStore` — TOML store at `~/.config/codeless/secrets.toml`.
BTreeMap-backed for deterministic output. Atomic writes (`.tmp` →
`fsync` → `rename`), 0600 perms on Unix. `open`, `list` (names only),
`get`, `set` (validates key), `remove`, `save`. `SecretError`: Io,
TomlParse, TomlSer, UnknownKey, InvalidKey.

#### `src/claude.rs`
Discovery + probe for `claude` binary. `probe() -> Option<ClaudeStatus>`
returns None only when binary not found. `read_version` runs `--version`
with 2s timeout. `probe_auth` runs `/status --output-format json`,
parses common field names. `discover_claude_binary` searches in order:
`$CLAUDE_BINARY`, PATH, `~/.local/bin`, `~/.bun/bin`,
`~/.npm-global/bin`, NVM versions, VS Code extensions, system paths.

#### `src/git_diff.rs`
`DiffFile { path, status, additions, deletions, is_binary, patch }`.
`diff_against(repo, base, head) → Result<Vec<DiffFile>, GitDiffError>`
validates refs (`BaseMissing`/`HeadMissing` distinct), runs
`git diff --name-status`, then `--numstat`, then per-file patches.

#### `src/worktree.rs`
`WorktreeManager` — per-job `git worktree` at `<base>/job-<id>` on
fresh branch `codeless/job-<id>`. Methods: `new(base)`, `path_for`,
`create(repo, job_id, requested_branch)` (fallback branch name on
collision), `remove`, `reap_orphans`, `list_on_disk`. Types:
`WorktreeHandle { path, branch }`, `OnDiskWorktree { job_id, path,
size_bytes, mtime_ms }`. Errors: `Io`, `GitFailed`, `AlreadyExists`.

#### `src/git_commit.rs`
`commit_paths(repo, subject, paths) → Result<bool, GitCommitError>` —
stage and commit; returns `Ok(true)` if a commit was made, `Ok(false)`
if nothing to commit. Used after writes for audit trail.

#### `src/ai_chat.rs`
Single-turn chat dispatch onto an `ai_runner` CLI runner. `run_chat<F,
Fut, E>(registry, provider, prompt, cwd, task_id, publish, cancel)` —
spawns runner once, streams upstream events through the bridge
translator. `parse_cli_runner_id` maps wire id (`claude`/`codex`/
`copilot`) to `Provider`. `probe_available_cli_runners` probes each
CLI runner. `AgentChatError<E> { RunnerNotRegistered, ForwarderJoin,
Publish }`.

#### `src/fs.rs`
`HostFs` — sandboxed filesystem with root + extra_roots, all
canonicalised once. `is_path_allowed(abs)` is the sandbox guard.
`read_dir`, `read_file`, `write_file`, `stat` are the only ways the
RPC layer touches disk. Errors: `Io`, `Escape`, `NotUtf8`, `BadRoot`.
Parent-traversal, absolute-outside-root, and symlink-escape attempts
are rejected before any disk touch.

#### Tests
- `tests/secrets.rs` — Atomic write, perms, round-trip.
- `tests/worktree.rs` — Create/remove/reap lifecycle.

### 2.10 `crates/codeless-tools/` — LLM-callable tools

Host-only by dependency; exposed via MCP.

#### `src/lib.rs`
Module manifest.

#### `src/tool.rs`
`trait Tool` (async): `name() → &str`, `schema() → &Value`, `call(ctx,
args) → Result<Value, ToolError>`. JSON in/out, JSON Schema validation.

#### `src/registry.rs`
`ToolRegistry` — `HashMap<&str, Arc<dyn Tool>>`. `register(tool) →
Option<previous>` (double-register is a bug). `get`, `iter`, `names`,
`len`, `is_empty`.

#### `src/ctx.rs`
`ToolCtx { worktree_root, network_mode, allowlist, cancel, span }`.
Per-call cancellation via child token.

#### `src/error.rs`
`ToolError`: `InvalidArgs`, `Cancelled`, `Denied`, `Failed`.

#### `src/policy.rs`
`NetworkMode { None, Allowlist, Open }`. `AllowlistFile` — HashSet of
exact hostnames (no wildcards yet).

#### `src/html_text.rs`
HTML → markdown-flavoured text + link extraction. `html_to_text(html)`
prefers `<main>`/`<article>`, drops chrome (nav/header/footer/aside) and
JS/CSS subtrees. `extract_links(html, base_url)` resolves relative
hrefs, skips fragment-only/javascript:/mailto:/tel:/data:, dedups on
final URL. `extract_text_and_links(html, base_url)` is the one-parse
combined call.

#### `src/testing.rs`
`FakeCtx` (ToolCtx + tempdir + cancel token) and `FakeCtxBuilder`.
`fake_ctx()` defaults: empty worktree, `NetworkMode::None`, empty
allowlist.

#### `src/tools/mod.rs`
Re-exports tool families: `BrowseFetchTool`, `HttpRequestTool`,
`BrowserCrawlTool`, `BrowserEvalTool`, `BrowserClickTool`,
`BrowserFillTool`, `BrowserHoverTool`, `BrowserScrollTool`,
`BrowserTypeTool`, `BrowserCookiesTool`, `BrowserExtractTool`,
`BrowserWaitTool`, `BrowserNavigateTool`, `BrowserReadTool`,
`BrowserScreenshotTool`, `BrowserSessionOpenTool`,
`BrowserSessionCloseTool`, `BrowserSessionListTool`. Shared helpers:
`check_network_policy(ctx, host, url)`, `url_host(url)`.

#### `src/tools/*` — individual tools

- **`browse_fetch.rs`** — GET URL → status + body, 1 MB max, 30 s
  timeout. Network policy enforced.
- **`http_request.rs`** — GET/POST/PUT/PATCH/DELETE/HEAD with body and
  headers. Response headers returned (caller redacts).
- **`browser_navigate.rs`** — Navigate existing session;
  `wait_until` + `timeout_ms` pass through to Playwright sidecar.
- **`browser_session.rs`** — `Open` (with user_agent, viewport,
  locale), `Close`, `List`.
- **`browser_read.rs`** — Three modes: `html` (raw), `markdown` (clean
  text + link list), `text` (markdown with structure). Sidecar fields
  (`title`, `byte_length`, `truncated`, `final_url`) pass through.
- **`browser_interact.rs`**, **`browser_misc.rs`**, **`browser_crawl.rs`**,
  **`browser_eval.rs`**, **`browser_screenshot.rs`** — click, fill,
  hover, scroll, type, cookies, extract, wait, crawl, eval,
  screenshot.

#### `src/browser/` — Playwright sidecar

- **`mod.rs`** — Public surface for `BrowserManager`, `RpcError`,
  `RpcRequest`, `RpcResponse`.
- **`manager.rs`** — Manages the Node-based Playwright sidecar
  process; speaks line-delimited JSON-RPC over stdio.
- **`bootstrap.rs`** — `ensure_installed(BootstrapPaths) →
  Result<InstalledSidecar, BootstrapError>` downloads Node + Playwright
  + Chromium on first use. `BootstrapPaths` lets callers control
  install targets.
- **`config.rs`** — Sidecar config types.
- **`protocol.rs`** — JSON-RPC frame types.
- **`sidecar.rs`** — Sidecar process lifecycle.

#### Tests
`browser_advanced.rs`, `browser_lifecycle.rs`, `browser_tools.rs`,
`browser_interact.rs`, `browser_crawl.rs`, `browser_bootstrap.rs`,
`registry_smoke.rs`, `browse_fetch_policy.rs`, `http_request_policy.rs`.

### 2.11 `crates/codeless-tauri-desktop/` — Tauri 2 desktop shell

Phase 5 placeholder. `src/main.rs` is intentionally minimal until the
shell is wired up.

### 2.12 `ui/codeless-ui/` — React 19 + TS, single UI for four shells

Terax-derived React 19 + Vite + TailwindCSS 4 + TypeScript 5.8. Lives
inside the inner repo at `codeless/ui/codeless-ui/`.

#### `src/app/`
- **`App.tsx`** — Root layout using `ResizablePanelGroup`. Wires
  editor, terminal, AI chat, file explorer, jobs; manages cross-module
  state (RPC client, settings window, inline settings); integrates AI
  composer provider and shortcuts dialog.

#### `src/components/`
- **`ai-elements/`** — 9 files: `message.tsx`, `conversation.tsx`,
  `code-block.tsx`, `tool.tsx`, `reasoning.tsx`, `markdown-code.tsx`,
  `snippet.tsx`, `shimmer.tsx`, `context.tsx`.
- **`ui/`** — 38 shadcn/ui components (Radix-based: button, input,
  dialog, tooltip, alert, popover, command, spinner, collapsible,
  item, etc.).
- **`WindowControls.tsx`** — Custom window chrome for Tauri desktop.

#### `src/lib/` — infrastructure

**`rpc/`** — the load-bearing R2 surface:
- `client.ts` — abstract `RpcClient` interface (`call`, `subscribe`,
  `serverInfo`); UI components depend only on this.
- `http-sse-client.ts` — browser + mobile transport. REST POST for
  calls (bearer header), SSE for subscriptions (bearer in query).
  Auto-reconnect, queue buffering.
- `tauri-ipc-client.ts` — desktop transport. Tauri `invoke()` for
  calls, typed `Channel<EventEnvelope>` for streams. Unsubscribe via
  `rpc_unsubscribe(channel_id)`.
- `mock-client.ts` — stubbed client for UI-only dev.
- `methods.ts` — RPC arg/result types (hand-mirrored from
  `codeless-rpc::methods` until Specta codegen takes over).
- `wire.ts` + `generated/wire.ts` — Specta-generated TS types from
  Rust wire format.
- `provider.tsx` — React context + hooks (`useRpc`, `useRpcCall`,
  etc.).
- `hooks.ts`, `config.ts`, `error.ts` — supporting infra.

**`shell/`** — platform capabilities:
- `provider.tsx` — `ShellProvider` context with capability gating.
- `settings-window.ts`, `window-controls.ts`, `kv-store.ts`,
  `autostart.ts`, `network-probe.ts`, `external-opener.ts`,
  `cross-window-events.ts`, `paths.ts`, `app-info.ts`, `updater.ts` —
  each is a thin interface with a stub impl in the browser shell and
  a Tauri impl in the desktop shell.

**Utils**: `route/index.ts`, `platform.ts`, `use-mobile.ts`,
`fonts.ts`.

#### `src/modules/` — features

- **`ai/`** — AI chat & agents. Stores (Zustand): `chatStore`,
  `agentsStore`, `snippetsStore`, `planStore`, `todoStore`.
  Components: `AiChat`, `AiInputBar`, `AiMiniWindow`, `SelectionAskAi`,
  `AgentRunBridge`, `AiToolApproval`, `SnippetPicker`,
  `AgentStatusPill`, `TodoStrip`, `PlanDiffReview`, `AgentSwitcher`.
  Hooks: `useWhisperRecording`. Tools: `subagent.ts`.
- **`editor/`** — CodeMirror 6 editor. `EditorPane`, `EditorStack`,
  `AiDiffPane`, `AiDiffStack`, `NewEditorDialog`, plus language /
  theme helpers under `lib/`. Vim bindings via `@replit/codemirror-vim`.
  Syntax via `shiki`.
- **`explorer/`** — File tree. `FileExplorer`, `FileTreeNode`,
  `ExplorerSearch` (debounced RPC), `InlineInput`, icon helpers.
- **`jobs/`** — Largest module. Dashboard, detail stack, stage detail,
  submit dialog. Subdir `spec/` for template editor (`SpecPane`,
  `TemplateSection`, `MarkdownSection`, `InlineEditor`,
  `parseTemplate`, `mutateTemplate`). Subdir `legacy/` retains the
  older chat-based job page. Supporting: `JobRow`, `JobTimeline`,
  `FilesChanged`, `StageTree`, `HandoverPanel`, `ReviewPanel`,
  `ReviewQueueBadge`, `StatusBadge`, `RunPane`, `WorktreeGcButton`,
  `JobChatPage`, `eventFormat`.
- **`terminal/`** — xterm.js-based terminal. `TerminalStack`,
  `TerminalPane`, `PaneTreeView`, plus pane-state and OSC handlers.
- **`header/`** — `Header.tsx`, `SearchInline.tsx`.
- **`statusbar/`** — `StatusBar.tsx`, `CwdBreadcrumb.tsx`,
  `AiTools.tsx`, `lib/pathUtils.ts`.
- **`preview/`** — `PreviewStack` for web previews.
- **`tabs/`** — Multi-tab navigation.
- **`shortcuts/`** — `ShortcutsDialog.tsx`.
- **`settings/`** — `SettingsApp.tsx`, sections (General, Agents,
  Models, About, Shortcuts), reusable widgets. `main.tsx` is the
  Tauri-window entry.
- **`theme/`** — light/dark/editor theme handling.
- **`updater/`** — `UpdaterDialog.tsx`, `useUpdater.ts` (Tauri
  updater).

#### `src/shells/`
- **`desktop/main.tsx`** — Tauri shell. Registers `TauriIpcClient`,
  Tauri plugins (updater, autostart, store, log, opener, window-state).
- **`browser/main.tsx`** — Web shell. Picks `HttpSseClient` (prod) or
  `MockRpcClient` (dev/demo) after a `/healthz` probe.
- **`ios/`, `android/`** — placeholder structure.

#### `src/styles/`
`globals.css` (Tailwind setup + custom properties), `tokens.ts` (design
tokens), `terminalTheme.ts` (xterm theme variants).

#### Key dependencies
- React 19.1, React-DOM 19.1, Zustand 5, react-resizable-panels 4.
- CodeMirror 6 family + `@uiw/react-codemirror` + `@replit/codemirror-vim`
  + Shiki 4.
- xterm 6 + addons (fit, search, web-links, WebGL).
- Vercel AI SDK 6 + provider SDKs (`@ai-sdk/*` Anthropic, OpenAI,
  Google, Groq, Cerebras, XAI, OpenAI-compat).
- Radix UI 1.4, Tailwind 4.2, shadcn 4.3, cmdk 1.1, motion 12.
- Zod 4, clsx 2, tokenlens 1, streamdown 2, `@streamdown/math` 1.
- `@tauri-apps/api` 2 + plugins (autostart, log, opener, os, process,
  store, updater, window-state).

---

## 3. Vendored crate — `ai-runner/`

Vendored from `rubix-agent`; no `.git` of its own. Patched in place;
every patch lands a row in `ai-runner.PATCHES.md` and a `//
codeless-patch-NNN` marker in source.

### 3.1 Overall purpose
Unified multi-provider AI orchestration. Five backends (Claude Code
CLI, OpenAI Codex CLI, GitHub Copilot CLI, Anthropic REST, OpenAI
REST) behind one async `Runner` trait. Handles subprocess spawning,
HTTP, event normalisation, cleanup.

### 3.2 `src/lib.rs`
Module manifest + re-exports: `Registry`, `ProviderStatus`, `Runner`,
`OnEvent`, `Provider`, `CliCfg`, `RestCfg`, `RunnerInput`, `RunResult`,
`RunnerError`, `Event`, `EventKind`, `HistoryMessage`,
`PermissionMode`, `ToolDef`, `ToolChoice`, `ToolUse`, `ToolCallEntry`,
`SessionId`, `AiDefaults`.

### 3.3 `src/types.rs`
- `SessionId` — caller-supplied newtype distinct from upstream CLI
  session ids.
- `Provider` — `Claude | Codex | Copilot | Anthropic | OpenAi`.
- `CliCfg` — prompt, system prompt, model, resume id, MCP url/token,
  tool filter, thinking budget, cwd, permission mode.
- `RestCfg` — prompt, system prompt, model, api key, base url, history,
  max tokens, headers, tool defs, tool choice, thinking budget.
- `RunnerInput` — `Cli(CliCfg) | Rest(RestCfg)`.
- `PermissionMode` — `Default | AcceptEdits | Plan | Bypass`.
- `HistoryMessage` — role + content.
- `ToolDef`, `ToolChoice`, `ToolUse`, `ToolCallEntry`.
- `Event { session_id, provider, kind }`.
- `EventKind` — `Connected{model} | Text{content} | ToolUse{id,
  name, input} | Done{duration_ms, cost_usd, input/output_tokens} |
  Error{message}`.
- `RunResult` — final text, provider, model, upstream session id,
  duration, cost, tokens, tool calls, tool log, structured tool uses,
  error.
- `RunnerError::WrongInputKind { provider, expected, got }`.

### 3.4 `src/runner.rs`
- `trait Runner` — `provider() → &Provider`; `ready() → bool` (probes
  binary or env key); `run(input, session_id, on_event, cancel) →
  Result<RunResult, RunnerError>`.
- `OnEvent = mpsc::Sender<Event>`. REST awaits sends (backpressure);
  CLI runners `try_send` and drop on overflow (warn).

### 3.5 `src/registry.rs`
- `ProviderStatus { provider, available }`.
- `Registry::new`, `with_defaults` (registers all five), `register`,
  `get`, `list`.

### 3.6 `src/defaults.rs`
`AiDefaults { provider, model, anthropic_api_key, openai_api_key }`,
`api_key_for(provider) → Option<String>`.

### 3.7 `src/runners/`
- **`claude.rs`** — Most sophisticated runner. Spawns `claude` binary;
  rich discovery order; writes temp JSON for MCP servers with auth
  headers; maps `low/medium/high` thinking budgets to prompt prefixes;
  parses stream-JSON (system → connect/session, assistant blocks →
  text + tool_use, result → cost/tokens). Returns upstream CLI
  session id for resume. Cancel drops the child (SIGKILL on Unix).
- **`codex.rs`** — Spawns Codex CLI `--quiet --full-auto [--model]
  <prompt>`. stdout lines → text events. Requires `OPENAI_API_KEY`.
  Plain text only; no structured tool calls.
- **`copilot.rs`** — Spawns Copilot CLI `-p <prompt> --allow-all-tools
  --no-ask-user --no-auto-update [-C <cwd>] [--model]`. stdout lines
  → text events. Auth via `copilot` binary (device flow). No
  structured events yet; richer output deferred (ACP server or
  `--log-dir` parse).
- **`anthropic.rs`** — Uses `anthropic-ai-sdk`. Streams
  `ContentBlock::ToolUse` (id + incremental JSON), text deltas, done
  events with token counts. Thinking budget maps to 1024 / 4096 /
  16384 tokens. Default model `claude-opus-4-5`.
- **`openai.rs`** — Uses `async-openai`. Streams text deltas + done.
  No structured tool calls or token counts from stream yet. Default
  model `gpt-4o`.
- **`mod.rs`** — Re-exports.

### 3.8 Patches log (`ai-runner.PATCHES.md`)
| Patch | Summary |
|-------|---------|
| 001 | Claude runner: forward `block["input"]` on `tool_use` so UI shows actual tool args. |
| 002 | Provider-agnostic `PermissionMode` enum + `CliCfg::permission_mode`; map to claude-wrapper modes for headless `Bypass`. |
| 003 | New `CopilotRunner` backend; spawns Copilot CLI with headless flags. |

---

## 4. Workspace `hackline/` — Zenoh-native remote-access for IoT fleets

A separate Rust workspace, also rooted under `codeless-workspace/`.
Provides two planes: a tunnel plane (raw bytes via TCP) and a message
plane (typed envelopes) bridging a cloud gateway to edge devices via
Zenoh. Independent from the codeless work above; included here for
completeness.

### 4.1 `crates/hackline-proto/` — wire types
- `lib.rs` — manifest + re-exports.
- `zid.rs` — `Zid` validated lowercase hex device id (2–32 chars), with
  serde, `From`/`TryFrom`, `as_str()`.
- `keyexpr.rs` — `connect()`, `info()`, `health()`, `stream_gw()`,
  `stream_dev()` Zenoh key-expression builders.
- `event.rs` — SSE event variants `DeviceOnline`, `DeviceOffline`,
  `TunnelOpened`, `TunnelClosed`, `TunnelConnection`; `CLOSE_SENTINEL`
  zero-length byte slice for stream close.
- `agent_info.rs` — `AgentInfo { label, allowed_ports }`.
- `connect.rs` — `ConnectRequest { request_id, peer }`, `ConnectAck
  { request_id, ok, message }`.
- `error.rs` — `ProtoError` variants.

### 4.2 `crates/hackline-core/` — TCP ↔ Zenoh bridging
- `lib.rs` — manifest.
- `session.rs` — `open(zenoh::Config) → Result<Session>`.
- `bridge.rs` — `accept_bridge` (agent side: receive request, dial
  loopback TCP, ack, pump bytes), `initiate_bridge` (gateway side:
  issue query, await ack, pump bytes between TCP and stream_gw /
  stream_dev keys). Constants: `READ_BUF` 32 KiB, `ACK_TIMEOUT` 10 s,
  `QUERY_TIMEOUT` 2 s.
- `error.rs` — `BridgeError` (Zenoh, I/O, timeout, rejection).

### 4.3 `crates/hackline-agent/` — device-side binary
- `main.rs` — `main()` async tokio; `init_tracing()`.
- `config.rs` — `AgentConfig`, `ZenohConfig`, `LogConfig`,
  `to_zenoh_config()`.
- `connect.rs` — `serve_connect()` spawns queryable listeners per
  allowed port.
- `info.rs` — Responds to `hackline/<zid>/info`.
- `liveliness.rs` — Manages Zenoh liveliness token.
- `error.rs` — `AgentError` variants.

### 4.4 `crates/hackline-gateway/` — cloud-side control plane
- `lib.rs` — exports `api`, `auth`, `config`, `db`, `state`, `tunnel`,
  `zenoh_client`.
- `bin/serve.rs` — main gateway binary.
- `bin/print_claim.rs`, `bin/reset_claim.rs` — utilities for the
  first-boot claim flow.
- `config.rs` — `GatewayConfig`, `ZenohConfig`, `TunnelEntry`,
  `LogConfig`.
- `state.rs` — `AppState { DbPool, Arc<zenoh::Session> }`.
- `zenoh_client.rs` — wrapper around Zenoh session.
- `error.rs` — gateway-wide error enum.
- `events_bus.rs` — in-process event broadcast for SSE.

`db/` — SQLite via r2d2 + refinery migrations:
- `pool.rs` — `DbPool`, `open()`.
- `migrations.rs` — embeds and runs SQL migrations.
- `devices.rs` — `Device { id, zid, label, customer_id, created_at,
  last_seen_at }` CRUD.
- `tunnels.rs` — `Tunnel`, `TunnelWithZid` CRUD.
- `users.rs`, `audit.rs`, `claim.rs` — corresponding CRUD.

`tunnel/` — manages TCP listeners against `tunnels` table:
- `manager.rs` — `run()` loads active TCP tunnels, spawns listeners.
- `tcp_listener.rs` — per-tunnel TCP `accept` loop, dispatches bridge.
- `bridge.rs` — delegates to `hackline-core::bridge`.
- `http_router.rs` — future HTTP Host-based routing.

`api/` — axum routes:
- `router.rs` — `build()` composes the full router.
- `health.rs` — `GET /v1/health`.
- `devices/` — list, create, get, delete, patch, info (live Zenoh
  query), health (last_seen + latency).
- `tunnels/` — list, create, delete.
- `claim/` — post (first-boot owner), status.
- `audit/` — list (cursor-paginated).
- `events/` — per_device + all SSE streams.
- `users/` — list, create, delete, mint_token.

`auth/` — token + scope:
- `token.rs` — hash + constant-time compare.
- `claim.rs` — first-boot claim logic.
- `middleware.rs` — bearer extraction + enforcement.
- `scope.rs` — device/tunnel scope checks.

### 4.5 `crates/hackline-cli/` — user-facing CLI
- `main.rs` — entry point (stub).
- `client.rs` — reqwest HTTP wrapper.
- `config.rs` — `~/.hackline/` credentials.
- `error.rs`, `output.rs` — typed errors and pretty output.

`cmd/`:
- `login.rs`, `whoami.rs`.
- `device/{list,add,show,remove}.rs`.
- `tunnel/{list,add,remove}.rs`.
- `events.rs` — `hackline events tail --device <id>` SSE client.
- `token/mint.rs`.
- `user/{list,add,remove}.rs`.

---

## 5. Cross-cutting architectural patterns

These are emergent from the file-by-file reading; they explain why the
code is shaped the way it is.

**One trait, many transports.** `codeless_rpc::RpcServer` is the single
contract. The in-process runtime, the HTTP client, and (eventually) the
Tauri shell all implement or consume it. The UI imports only the JS
mirror (`RpcClient`).

**Driver vs. runner separation.** Runners publish stage/task/AI events;
the driver owns Job-row transitions. This keeps stop/pause/cap logic in
one place and makes runner implementations interchangeable.

**Two-stage event publication.** Persist to SQLite first, then
broadcast. Subscribers replay by cursor, then tail live. Gap-free across
restarts because cursor ordering is durable.

**Sandboxed host operations.** `HostFs` canonicalises root paths once
and rejects parent traversal, absolute escapes, and symlink escapes
before any disk touch. `WorktreeManager` keeps git operations in a
fixed per-job path layout.

**Process spawning is contained.** Per R1, only
`codeless-adapters-host` and `ai-runner` spawn child processes. Mobile
builds will never link these.

**Specta-driven TS contract.** Every wire type in `codeless-types` and
`codeless-rpc` derives `specta::Type`. `mani wire-ts-check` regenerates
`generated/wire.ts` and fails CI on drift.

**Idempotent demo + smoke tests.** `codeless demo bootstrap` is
re-runnable; `scripts/smoke-demo.sh` and `scripts/smoke-claude-demo.sh`
exercise mock and real-Claude paths end-to-end.

**Append-only run artefacts.** `runs/<job_id>/log.md` accumulates one
section per run; `handover.md` is overwritten each run but always
parseable back to `Handover` struct via the fenced ```` ```handover ````
block contract.

---

## 6. Coverage summary

| Area | Files | Notes |
|------|-------|-------|
| Workspace shell | ~30 (docs + scripts + mani) | Section 1 |
| codeless Rust crates | 158 `.rs` files across 10 crates | Section 2 |
| codeless UI | ~200 TSX/TS files across `src/` | Section 2.12 |
| ai-runner Rust | 11 `.rs` files | Section 3 |
| hackline Rust | ~70 `.rs` files across 5 crates | Section 4 |

Every Rust source file is named at least once in this document. Every
public type and public function in the codeless and ai-runner crates is
named with its role. The UI is mapped to module granularity rather than
component granularity (component-level documentation would itself
require a separate document of comparable size).

For "why" questions not answered here, the controlling sources are
`DOCS/SCOPE.md` (architecture) and `CLAUDE.md` (rules R1-R5). Anything
in this file that contradicts those sources is wrong and should be
corrected against them.
