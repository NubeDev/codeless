# codeless

Rust workspace for the Codeless project — a staged, reviewable AI coding
job runner. This repo is the **inner** repo; design docs, the
multi-repo `mani.yaml`, the autonomous-build loop, and active session
files live in the parent workspace at
[`codeless-workspace`](https://github.com/NubeDev/codeless-workspace).

Start here if you are reading this for the first time:

- Agent-facing rules for this repo: [`CLAUDE.md`](./CLAUDE.md)
- Durable per-repo memory: [`CODELESS.md`](./CODELESS.md)
- Project scope, crate layout, all open questions:
  [`../DOCS/SCOPE.md`](../DOCS/SCOPE.md)
- Autonomous build loop: [`../DOCS/JOB-LOOP.md`](../DOCS/JOB-LOOP.md)

## Quickstart

```sh
# from the inner repo
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# end-to-end dogfood: pick a runner and stream events to stdout as
# JSON lines. --runner defaults to `mock`; `claude` shells out to
# the `claude` binary (or $CLAUDE_BINARY); `anthropic` talks REST
# and reads ANTHROPIC_API_KEY (or --api-key).
cargo run -p codeless-cli -- run --repo /path/to/repo "hello"
cargo run -p codeless-cli -- run --repo /path/to/repo --runner claude "rename foo to bar"

# stateful subcommands need --db <path> (or CODELESS_DB env) so
# successive invocations share state.
export CODELESS_DB=~/.local/share/codeless/codeless.db

# submit a typed YAML job template
cargo run -p codeless-cli -- job submit job.yaml

# follow a job to completion (replays persisted events first)
cargo run -p codeless-cli -- tail <job-id>

# drive a review gate (after a stage hits AwaitingReview)
cargo run -p codeless-cli -- review list
cargo run -p codeless-cli -- review approve <review-id>
cargo run -p codeless-cli -- review comment <review-id> "looks good but rerun verify"
cargo run -p codeless-cli -- review stop <review-id>

# manage the chmod-600 secrets store
cargo run -p codeless-cli -- secrets list
cargo run -p codeless-cli -- secrets set ANTHROPIC_API_KEY --from-env ANTHROPIC_API_KEY
```

### Run the browser demo

`codeless serve` runs the hosted HTTP surface that the React UI in
[`ui/codeless-ui/`](./ui/codeless-ui/) talks to. One-shot setup:

```sh
# pick a DB and mint the shared bearer token; prints it once
cargo run -p codeless-cli -- --db ~/.local/share/codeless/demo.db \
    serve --init-token

# run the server (defaults to 127.0.0.1:7777)
cargo run -p codeless-cli -- --db ~/.local/share/codeless/demo.db serve

# in another terminal, run the UI dev server
pnpm -C ui/codeless-ui install   # first time only
pnpm -C ui/codeless-ui dev
```

Full instructions (including the `localStorage` keys the browser
reads) live in the workspace
[`DEMO-UI.md`](../DEMO-UI.md) quickstart.

### YAML job template shape

`codeless job submit` parses a strict typed shape — unknown keys are a
parse error with line/column, so a `runneer:` typo never silently
defaults:

```yaml
repo: 01J...            # repo id (ULID)
runner: claude          # mock | claude | anthropic
prompt: refactor parser
branch: codeless/refactor-parser
stages:
  - name: plan
  - name: verify
    verify_cmd: cargo test
caps:
  cost_cents: 500       # 0 = unlimited
  wall_clock_ms: 600000 # 0 = unlimited
```

### Outbound webhook notifier

`codeless-runtime::WebhookNotifier` POSTs JSON to a configurable URL
on `JobFailed` and `ReviewRequested`. The body is HMAC-SHA256 signed
with a shared key; the signature lands on the
`x-codeless-signature` header as lowercase hex. Config is TOML-shaped
so it can sit in the secrets file:

```toml
[notifier.webhook]
url = "https://hooks.example.com/codeless"
hmac_key_hex = "deadbeef..."
```

Wire it into a running core with `spawn_notifier(bus,
Arc::new(WebhookNotifier::from_config(cfg)?))`.

State lives in SQLite. The runtime builds against a caller-supplied
`SqlitePool` (or `InProcessRpc::new()` for an in-memory pool in tests);
the Appendix A migrations apply on construction. Repos, jobs, stages,
tasks, and events all persist; a fresh runtime against the same DB
file resumes where the previous one left off, including a startup
reaper that returns expired task leases to the queue.

### Driving a real runner from Rust

The CLI's `run --runner {claude,anthropic}` covers the common path.
Embed the runtime directly when you need finer control over the
`RunnerAdapter` (custom prompt, base URL override for tests, etc.):

```rust
use std::sync::Arc;
use codeless_runtime::{drive_job, ClaudeRunnerAdapter, InProcessRpc};
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_types::{GitAuth, TaskId};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc = InProcessRpc::new().await?;
    let repo = rpc.add_repo(AddRepoArgs {
        name: "demo".into(),
        clone_url: "https://example.test/demo.git".into(),
        default_branch: "main".into(),
        local_path: "/path/to/repo".into(),
        git_auth: GitAuth::Token { env_var: "GITHUB_TOKEN".into() },
        concurrency_cap: None,
        default_runner: None,
    }).await?;
    let job = rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("rename `foo` to `bar` everywhere".into()),
        template_yaml: None,
        runner: "claude".into(),
        branch: "codeless/job-demo".into(),
        cost_cap_cents: 200,   // 0 = unlimited
        wall_clock_cap_ms: 600_000,
    }).await?;

    let adapter: Arc<dyn codeless_runtime::Runner> = Arc::new(
        ClaudeRunnerAdapter::new("rename `foo` to `bar` everywhere", TaskId::new()),
    );
    drive_job(&rpc, job.id, adapter, /* worktrees= */ None).await?;
    Ok(())
}
```

Swap in `AnthropicRunnerAdapter::new(...)` for the REST path; set
`adapter.base_url` to redirect the SDK (used by tests against a
`wiremock` stub). Pass `Some(Arc::new(WorktreeManager::new(...)))` as
the fourth `drive_job` arg in production so every run gets its own
isolated `git worktree`; the worktree is removed on every terminal
exit. The cap watcher inside `drive_job` reads
`jobs.cost_cents` (auto-rolled by `EventBus::publish` on every
`ai-message-complete`) against `cost_cap_cents`, and races
`wall_clock_cap_ms` in parallel; either tripping the cap stops the
job and emits `JobStopped { reason: CostCap | WallClock }`.

## UI

The React UI lives at [`ui/codeless-ui/`](./ui/codeless-ui/) — a
Terax-derived React 19 + TypeScript app that already includes editor
(CodeMirror 6), terminal (xterm.js), file explorer, AI chat panel,
settings, and themes. It is the **single** UI that ships to all four
shells (browser, Tauri desktop, iOS, Android); per-shell files are
forbidden by R3 in [`CLAUDE.md`](./CLAUDE.md).

The transport boundary lives at
[`ui/codeless-ui/src/lib/rpc/`](./ui/codeless-ui/src/lib/rpc/): a
typed `RpcClient` interface with `HttpSseClient` (browser/mobile),
`TauriIpcClient` (desktop, stub), and `MockRpcClient` (tests). Each
shell entry under
[`ui/codeless-ui/src/shells/`](./ui/codeless-ui/src/shells/)
constructs the right client and mounts the same `<App />`.

Active UI work is the Tauri-conversion grind — 31 files still import
`@tauri-apps/*` and need rerouting through `RpcClient` or a
shell-injected capability adapter. Architectural rationale in
[`../DOCS/UI-ARCHITECTURE.md`](../DOCS/UI-ARCHITECTURE.md); per-file
worklist in [`../DOCS/UI-PORT-AUDIT.md`](../DOCS/UI-PORT-AUDIT.md).

```sh
cd ui/codeless-ui
pnpm install
pnpm dev        # browser shell against MockRpcClient by default
```

## Origin

The original fork rationale (Terax as a starting point) lives below
for the history; the active design lives in
[`../DOCS/SCOPE.md`](../DOCS/SCOPE.md).

---

see existing code we can use
- /home/user/code/rubix-workspace/rubix-agent/crates/ai-runner/src/runners

Terax is already a strong base because it gives us a **desktop AI development environment**, not just a blank Tauri shell. It already has Tauri 2 + Rust + React 19, native PTY terminal, editor, file explorer, AI side panel, AI edit diffs, task/planning/search/file tools with approval flow, project memory via `TERAX.md`, local model support, and API-key provider support. ([GitHub][1])

So the scope should be:

## AI Job Loop / Terax Fork — Scope Summary

We will **fork `crynta/terax-ai`** and use it as the base codebase for our own AI coding/job-loop system.

This will become our own product. We will **not aim to stay compatible with upstream Terax**. Terax is the starting point only.

## Why Terax is a good base

Terax already includes a lot of what we need:

```txt
Desktop shell:       Tauri 2 + Rust
Frontend:            React 19 + TypeScript
Terminal:            xterm.js + WebGL
PTY backend:         portable-pty
Editor:              CodeMirror 6
File explorer:       included
AI panel:            included
AI providers:        OpenAI, Anthropic, Google, Groq, xAI, Cerebras, OpenAI-compatible
Local models:        LM Studio support
Approval flow:       tasks, plans, search, file read/write tools with approval
Project memory:      TERAX.md
UI stack:            Tailwind + shadcn/ui + Zustand
```

Terax also stores API keys in the OS keychain and does not use telemetry/account login, which is a good local-first base. ([GitHub][1])

## What we reuse

### Reuse from Terax

* Tauri desktop shell
* React UI structure
* terminal tabs
* native PTY backend
* file explorer
* code editor
* AI side panel
* AI edit diff UI
* provider settings UI
* local model support
* approval-flow ideas
* project memory concept
* shadcn/Tailwind UI stack

## What we add/change

The main new feature is the **AI Job Loop runtime**.

Terax today is more like an AI terminal/editor. We want to turn the fork into a **staged AI coding job runner**.

```txt
Job
├── Stage 1
│   ├── Task 1
│   └── Task 2
├── Stage 2
│   └── Task 1
└── Review Point
```

A job can:

* run from CLI first
* use Claude/Codex/Copilot CLI wrappers
* split work into stages and tasks
* start stages as fresh AI sessions
* run verification commands
* save state to SQLite
* create review points
* wait for user approval/comment/stop
* later show progress in browser and desktop UI

## Core architecture

```txt
Forked Terax
├── Existing Desktop App
│   ├── Tauri
│   ├── React
│   ├── Terminal
│   ├── Editor
│   ├── File Explorer
│   └── AI Panel
│
├── New AI Job Runtime
│   ├── Job / Stage / Task model
│   ├── Crossflow / Bevy runtime
│   ├── SQLite state store
│   ├── Provider CLI runners
│   ├── Review / approval system
│   └── Scheduler
│
├── CLI First
│   ├── create job
│   ├── start job
│   ├── stop job
│   ├── approve review
│   ├── comment review
│   └── status/logs
│
└── Browser Later
    ├── REST API
    ├── SSE live event stream
    └── gRPC for internal/remote runners later
```

## CLI first

The first version should work without the UI.

Example commands:

```bash
ai-job create job.yaml
ai-job start <job-id>
ai-job status <job-id>
ai-job logs <job-id>
ai-job approve <review-id>
ai-job comment <review-id> "Change this before continuing"
ai-job stop <job-id>
```

This keeps the runtime clean and testable before we wire it deeply into the Terax UI.

## Desktop app

The desktop version should reuse the existing Terax app.

Add new UI areas:

```txt
Jobs panel
Stage/task timeline
Live AI output
Review approval panel
Run logs
Provider/session selector
```

The existing terminal/editor/file explorer can stay central because they are useful for reviewing the work.

## Browser support

We should make sure the system works in the **browser as well**, not only desktop.

That means the job engine should not be locked to Tauri commands only.

Use a shared backend API:

```txt
Browser React UI
↓
REST API
↓
Rust job runtime
↓
SQLite + Crossflow + AI runners
```

For live updates:

```txt
Browser UI
↓
SSE stream
↓
Job events
```

Desktop can still use Tauri directly, but the core runtime should also be exposed through REST/SSE so the browser version works.

## SQLite

Use SQLite for runtime state.

YAML/TOML can be used for job templates, but once a job starts, SQLite becomes the source of truth.

SQLite stores:

```txt
jobs
stages
tasks
sessions
provider_runs
events
reviews
approvals
artifacts
locks
```

## Crossflow / Bevy

Use Crossflow from the start.

Crossflow/Bevy runs the job workflow:

```txt
Load Job
↓
Select Stage
↓
Create AI Session
↓
Run Provider
↓
Run Verify Commands
↓
Create Review
↓
Wait for Approval
↓
Continue / Stop / Retry
```

Important boundary:

```txt
Domain model does not depend on Crossflow.
Provider runners do not depend on Crossflow.
Only the runtime layer depends on Crossflow/Bevy.
```

## Provider runners

We should support two provider types:

### 1. Existing Terax model/API providers

Terax already supports OpenAI, Anthropic, Google, Groq, xAI, Cerebras, OpenAI-compatible providers, plus LM Studio/local models. ([GitHub][1])

These are useful for:

* summaries
* review generation
* planning
* lightweight chat
* local/offline workflows

### 2. Coding CLI providers

Add wrappers for:

```txt
Claude Code CLI
Codex CLI
Copilot CLI
```

These are used for actual coding-agent sessions.

## Review points

A review point stops the job and asks the user what to do next.

User options:

```txt
approve
comment
stop
rerun stage
continue with changes
```

Review points can happen:

* after every stage
* after selected stages
* when tests fail
* before risky file changes
* before commit/push
* before moving to the next stage

## REST API later

REST is for the future browser UI.

Example endpoints:

```txt
POST   /api/jobs
GET    /api/jobs
GET    /api/jobs/{jobId}
POST   /api/jobs/{jobId}/start
POST   /api/jobs/{jobId}/stop
GET    /api/jobs/{jobId}/events
POST   /api/reviews/{reviewId}/approve
POST   /api/reviews/{reviewId}/comment
POST   /api/reviews/{reviewId}/stop
```

## SSE later

SSE is for live UI updates.

```txt
GET /api/jobs/{jobId}/events/stream
```

Use this for:

* AI output
* stage started
* task started
* verification started
* verification passed/failed
* review requested
* job stopped
* job completed

## gRPC later

gRPC can come later for internal runtime APIs or remote runners.

Useful later for:

* remote provider runners
* worker processes
* distributed job execution
* cloud-hosted runner control

## Revised MVP

### Phase 1 — Fork + clean base

* fork Terax
* rename/rebrand internally
* verify desktop build
* understand current Rust/Tauri command structure
* understand AI provider/tool approval flow
* identify reusable AI/editor/terminal components

### Phase 2 — CLI job runner

* add SQLite store
* add job/stage/task schema
* add CLI commands
* add Crossflow/Bevy runtime
* add Claude/Codex/Copilot CLI runner abstraction
* run jobs from CLI without UI dependency

### Phase 3 — Desktop integration

* add Jobs panel to Terax UI
* show stage/task state
* stream AI/job events into UI
* add review approve/comment/stop controls
* connect job runs to terminal/editor/file explorer

### Phase 4 — Browser mode

* add REST API
* add SSE event stream
* make React UI work outside Tauri
* handle browser-safe file/project access through backend APIs

### Phase 5 — gRPC / remote runners

* add runner service
* support remote job workers
* support cloud/edge execution later

## Final direction

```txt
Fork Terax
Use it as the desktop/browser UI and ADE base
Add AI Job Loop as the new core runtime
Use SQLite for durable state
Use Crossflow/Bevy for workflow execution
Use CLI first
Expose REST/SSE for browser later
Add gRPC later for distributed runners
Do not maintain upstream Terax compatibility
```

The key point: **Terax gives us the shell, editor, terminal, AI panel, provider settings and approval-flow starting point. We add the staged autonomous job system on top.**

[1]: https://github.com/crynta/terax-ai "GitHub - crynta/terax-ai: Lightweight (7MB) AI terminal emulator (ADE) built in Rust & Tauri & React · GitHub"
