# codeless


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
