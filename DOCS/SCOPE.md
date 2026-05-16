# Codeless — Scope

## One-line summary

Codeless is a staged, reviewable AI coding job runner that manages **many concurrent jobs across many repos**. It runs as a **headless Rust core service** with three peer control surfaces — a browser UI, a CLI, and an **MCP server**. The UI is the primary surface for humans; the CLI and MCP server are the primary surfaces for everything else (scripts, CI, and AI agents such as Claude Code / Codex driving Codeless without ever opening a browser). It is forked from [`crynta/terax-ai`](https://github.com/crynta/terax-ai), but only the **UI layer** of Terax is reused. The AI runtime is rewritten in Rust so it can serve the browser remotely from a box you own.

## Why this scope (the key constraints)

Two constraints drive every architectural decision below.

**Constraint 1 — many concurrent jobs across many repos from a browser.** One person, a core running on a remote machine (home box, VPS, Mac mini), the browser as a thin view. This rules out a tab-resident AI loop and forces the runtime into the Rust core.

**Constraint 2 — a coder loop must run unsupervised for hours.** A developer starts a long job before bed and wakes up to commits on a branch, a `handover.md` showing what landed and what halted, and a few `REVIEW` stages at the points where the loop decided it needed a human. This is the difference between "agent that codes" and "agent that codes for as long as it takes". It forces *fresh session per stage* — every new stage opens with a disposable agent that re-reads the previous stage's handover from disk and the git history from the remote, then runs to its next checkpoint. Within a stage the runner session is continuous (pauses, cap bumps, "ask a question" all keep the same conversation, per hard rule #1 below); across a stage boundary the session always resets, so anything that needed to survive into the next stage has to be on disk or in the commit, not in the agent's head. See [`LOOP-CODER.md`](./LOOP-CODER.md) for the full design intent, and [`JOB-MODEL.md`](./JOB-MODEL.md) for the user-facing handover/log contract. The current build-out of this constraint — handover read/write, re-run with feedback, verify-fail policy, loop-level cost ceiling, and (last) goal-to-stages planning — is tracked as the **autonomy track (A1–A5)** in [`PROGRESS.md` "Next steps"](./PROGRESS.md#next-steps). A separate **real-codebases track (R0–R4)** sits ahead of autonomy and covers "does this fit a project I actually have?" — workspace mode (in_repo vs worktree), per-stage cwd, path scoping, git history hygiene, and dev-server live-reload. R0 is the floor: without `workspace_mode: in_repo` as a first-class option, the single-developer dogfooding loop doesn't match a normal git workflow at all.

Constraint 1 alone is enough to rule out the inherited Terax model. In Terax today the AI loop runs *inside the webview* using the Vercel AI SDK in TypeScript, with each user's API key going from the browser/webview straight to OpenAI/Anthropic/etc. That model is fine for a single-user desktop tool with one job at a time. It does not work for what we want:

- The browser tab can close — long-running jobs can't live there.
- Many jobs running at once need a scheduler, queue, and shared event bus — none of that fits in a tab.
- Browser tabs cannot shell out to `claude` / `codex` / `gh copilot` CLI binaries.
- API keys can live in a browser for a personal tool, but the *jobs themselves* still need to outlive the tab.

So the AI runtime moves to the Rust core. The browser becomes a thin view on top of a typed RPC, and the core is what schedules, runs, and persists jobs across repos.

## What Codeless adds on top of Terax

> Naming: we say "the core" colloquially in prose for the headless service as a concept; the specific Rust crate that implements its state machine is `codeless-runtime` (see Crate layout).

A **Repo → Job → Stage → Task → Review** runtime, with many jobs running concurrently across many repos:

```
Repo A                                 Repo B
├── Job 1 (running)                    ├── Job 4 (queued)
│   ├── Stage 1 ✓                      └── Job 5 (running)
│   └── Stage 2 (in progress)              └── Stage 1
├── Job 2 (awaiting review)            
└── Job 3 (completed)                  
```

### What each level means

The hierarchy is load-bearing — every contributor will need this. Levels, in order:

| Level | What it owns | How it ends | Events it emits |
|---|---|---|---|
| **Repo** | A managed git repository: clone path, default branch, auth method, per-repo caps. Long-lived. | Removed by user (`codeless repo remove`). | `repo-added`, `repo-removed`, `repo-updated` |
| **Job** | One unit of work the user kicked off, scoped to one repo, executed in one worktree on one branch. Owns the cost cap, the wall-clock cap, the chosen runner, the YAML template (if any). | `completed` (all stages green), `failed` (a stage failed and no retry policy applied), `stopped` (user/cap), `awaiting-review` then resumed. | `job-queued`, `job-promoted`, `job-started`, `job-completed`, `job-stopped`, `job-failed` |
| **Stage** | A verify-gated chunk of a job — a coherent body of work that ends at a checkpoint (build passes, tests pass, lint clean, custom verify command succeeds). One stage can contain many tasks. | `passed` (verify green), `failed` (verify red), `awaiting-review` (gate). | `stage-started`, `verify-started`, `verify-passed`, `verify-failed`, `stage-completed` |
| **Task** | One runner invocation inside a stage — one CLI spawn or one REST conversation turn-set. Multiple tasks within the **same stage** share a runner session (the second task passes `--continue <session_id>`; see hard rule #1), so a cap-paused stage resumes mid-conversation rather than re-deriving. The smallest re-runnable unit at the *invocation* level; cross-stage continuity is the stage's concern, not the task's. | `completed`, `failed`, `cancelled` (cost/time cap or user — `cancelled` mid-stage is a pause, not a stop). | `task-enqueued` (carries `depends_on`), `task-started`, `tool-call`, `tool-approval-requested`, `ai-token`, `ai-message-complete`, `task-completed` |
| **Review** | A *state* on a Stage (or, rarely, on a Job), not a node in the tree. When a stage hits a review gate, a `Review` row is created with status `pending`; user actions (`approve`, `comment`, `stop`, `rerun`) drive the stage forward. | `approved`, `rejected`, `stopped`, `rerun-requested`. | `review-requested`, `review-approved`, `review-commented`, `review-stopped` |

Tasks are the atomic re-runnable invocation unit and the smallest observability granule. Stages are the verify-gated unit *and* the session-continuity unit — a paused / resumed / cap-bumped stage continues the same runner conversation, while a new stage opens a fresh one (per hard rule #1 below). Reviews are gates, not nodes — this matters for the data model: `reviews` is a table joined to `stages` (with `stage_id`), not a separate level in the tree.

A job can:

- be defined as a YAML/TOML template, or kicked off ad-hoc from the browser/CLI
- belong to a specific repo, executed in an isolated `git worktree`
- run concurrently with other jobs on the same repo (each in its own worktree) or different repos
- spawn provider sessions (model API **or** coding CLI — both are first-class for the coding loop)
- run verification commands between stages
- persist all state to SQLite (repos, jobs, stages, tasks, sessions, events, reviews, artifacts)
- pause at review points and wait for approve / comment / stop / rerun
- be observed live from the browser UI, or polled/streamed from the CLI

## Architecture

The same React UI ships as **browser** (MVP), **Tauri desktop**, and later **Tauri mobile**. The CLI and the MCP server are the **headless surfaces** — any human (CLI) or AI agent (MCP) can drive the core without opening a UI. All clients drive the same Rust core through the same RPC trait.

```
┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐
│ Browser  │  │ Desktop  │  │ Mobile   │  │ CLI      │  │ MCP clients  │
│ (React)  │  │ (Tauri 2)│  │ (Tauri 2)│  │ (Rust)   │  │ (Claude Code,│
│ [MVP]    │  │ [Phase 5]│  │ [Phase 6]│  │ [Phase 2]│  │  Codex, ...) │
│          │  │          │  │          │  │          │  │ [Phase 2]    │
└────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘  └──────┬───────┘
     │ SSE+       │ Tauri      │ SSE+       │ in-process   │ MCP stdio
     │ REST+WS    │ IPC        │ REST+WS    │ or SSE+      │ or MCP HTTP
     │ (hosted)   │ (local)    │ (hosted)   │ REST+WS      │ (Streamable)
     ▼            ▼            ▼            ▼              ▼
   ╔════════════════════════════════════════════════════════════════════╗
   ║                       codeless-runtime (Rust)                      ║
   ║   Repo → Job → Stage → Task state machine                          ║
   ║   Scheduler • Queue • Tools • Reviews • Event bus • SQLite         ║
   ║   Worktree manager • Secrets abstraction                           ║
   ║                                                                    ║
   ║   Coding loop (any of, user picks per job):                        ║
   ║       ai-runner::Claude / Codex / Copilot  (CLI wrappers)          ║
   ║       ai-runner::Anthropic / OpenAI         (REST + API key)       ║
   ║   Helper role (optional):                                          ║
   ║       Rig — planner, reviewer, summariser, job memory              ║
   ╚════════════════════════════════════════════════════════════════════╝
                ▲                                ▲
                │                                │
       codeless-adapters-host          codeless-adapters-desktop
       (worktrees, shell, PTY,         (local FS, OS keychain,
        local FS, secrets file)         native dialogs)
```

> **MVP scope:** browser + CLI + MCP server + core, deployed as a single binary on a box you own. Tauri desktop and mobile are deferred but architecturally first-class — the crate split and the "one React codebase" rule below guarantee they remain a packaging exercise, not a re-architecture. The CLI and MCP server are *not* deferred: they ship in Phase 2 so the product is fully usable headless before the browser shell exists.

### Crate layout (load-bearing, not aspirational)

The Rust side is **explicitly split into reusable crates** so that browser, desktop, and mobile are different *compositions* of the same parts, not different forks. The split is:

| Crate | Contains | iOS-safe | Android-safe | Notes |
|---|---|---|---|---|
| `codeless-types` | Pure data: Repo, Job, Stage, Task, Event, RPC message types (serde). **No I/O.** | ✅ | ✅ | Source of truth for the wire format. |
| `codeless-rpc` | RPC trait + transport-agnostic client. | ✅ | ✅ | No assumptions about how bytes move. |
| `codeless-runtime` | State machine, **job-queue scheduler** (picks which queued job runs next under the concurrency cap), **session scheduler** (fires the next fresh agent session for a running job), queue, `sqlx`, event bus, `tokio`. **No process spawn, no PTY.** | n/a | n/a | The brain. Host-only — runs on the hosted server and inside Tauri desktop. Never compiled for mobile (mobile is always a thin client to a hosted core — see Phase 6). The two schedulers are distinct mechanisms: the job-queue scheduler is `tokio`-supervisor over `jobs.status`; the session scheduler is what fires the next session-tick for a running job — see [`LOOP-CODER.md`](./LOOP-CODER.md) "What blocks this today". |
| `codeless-adapters-host` | Worktree manager, shell, PTY, `ClaudeRunner`/`CodexRunner`/etc., local FS, secrets-file backend. | ❌ | ❌ | Process spawning gated here. Never compiled into mobile. |
| `codeless-adapters-desktop` | OS-keychain secrets backend, native file dialogs. **Created when there is more than one thing to put in it** — until then, lives inside `codeless-tauri-desktop`. | ❌ | ❌ | Most things first thought of as "desktop" actually live in `adapters-host`. |
| `codeless-server` | Hosted HTTP binary: axum SSE + REST + WS surface, auth middleware. Depends on `runtime` + `adapters-host`. | n/a | n/a | The MVP shell. |
| `codeless-mcp` | MCP server binary (stdio + Streamable HTTP). Exposes the **same RPC trait** as `codeless-server` as MCP tools/resources/prompts. Depends on `runtime` + `adapters-host` for local mode, or on `client` when proxying to a remote hosted core. | n/a | n/a | The headless agent-driven surface. Parity with CLI is enforced in CI — see "MCP surface" below. |
| `codeless-client` | SSE + REST + WS client. Used by browser and mobile webview. | ✅ | ✅ | The single thin-client library. |
| `codeless-cli` | CLI binary; calls `runtime` in-process, or `client` over network when pointed at a hosted core. Bundles `codeless mcp` as a subcommand that execs `codeless-mcp` so a single binary serves both surfaces. | — | — | |
| `codeless-tauri-desktop` | Tauri app; depends on `runtime` + `adapters-host` (+ `adapters-desktop` once it exists). | — | — | Phase 5. |
| `codeless-tauri-mobile` | Tauri 2 mobile app; depends **only on** `types` + `client`. | ✅ | ✅ | Phase 6. Cannot accidentally pull in process-spawning code. |

Process-spawning runners (anything that launches `claude`, `codex`, etc.) are **Cargo-feature-gated** so a mobile build physically cannot include them. This is what makes the layering enforceable rather than aspirational.

### Directory and repo layout

#### External repos that sit alongside `codeless`

| Repo | Role | Source |
|---|---|---|
| `codeless` | This repo — Rust core + UI | new |
| `terax-ai` (forked) | UI components to port from | [`crynta/terax-ai`](https://github.com/crynta/terax-ai) — fork or read-only reference |
| `ai-runner` | `Runner` trait + `ClaudeRunner` / `CodexRunner` / `AnthropicRunner` / `OpenAIRunner` | **Vendored** from `rubix-agent` workspace into `ai-runner/` (no `.git`), patched in-place. See [`CLAUDE.md`](../CLAUDE.md) and [`ai-runner.PATCHES.md`](../ai-runner.PATCHES.md). |
| `mani.yaml` workspace | Multi-repo dev orchestration sitting above the repos | new — lives in `~/code/rust/` (existing parent), not a new dir |

The mani workspace lives at `~/code/rust/mani.yaml` (the existing parent of `codeless/`):

```
~/code/rust/
├── mani.yaml                # multi-repo orchestration
├── codeless/                # this repo
├── ai-runner/               # vendored or forked from rubix-agent workspace
└── terax-ai/                # reference for UI port
```

#### Inside `codeless/`

```
codeless/
├── Cargo.toml                          # workspace root
├── CLAUDE.md                           # rules an AI agent must follow (Phase 1)
├── CODELESS.md                         # project memory, per-repo notes
├── crates/
│   ├── codeless-types/                 # pure data, serde, no I/O — iOS+Android safe
│   ├── codeless-rpc/                   # RPC trait, transport-agnostic — iOS+Android safe
│   ├── codeless-runtime/               # state machine, scheduler, sqlx, event bus — host-only
│   ├── codeless-adapters-host/         # worktrees, shell, PTY, runners — host-only, feature-gated
│   ├── codeless-server/                # axum SSE+REST+WS binary (Phase 3)
│   ├── codeless-client/                # SSE+REST+WS client — used by browser + mobile
│   ├── codeless-cli/                   # CLI binary
│   └── codeless-tauri-desktop/         # Tauri 2 shell (Phase 5, stubbed in Phase 1)
├── ui/
│   └── codeless-ui/                    # single React + TS codebase, RpcClient interface
├── migrations/                         # sqlx::migrate! files, forward-only
└── DOCS/
    └── SCOPE.md
```

Reserved for later phases (not created in Phase 1):

- `crates/codeless-adapters-desktop/` — extracted from `codeless-tauri-desktop` in Phase 5 once it has more than one thing in it.
- `crates/codeless-tauri-mobile/` — Phase 6.

#### Runtime data (created at first run, not committed)

Path comes from the path-provider abstraction (Rule 3); for the hosted host on Linux this is `$XDG_DATA_HOME/codeless/`:

```
$XDG_DATA_HOME/codeless/
├── codeless.db                  # SQLite — repos, jobs, stages, tasks, events, reviews
├── db.backup.<timestamp>        # pre-migration backups
├── audit.log                    # auth events; never sweep-deleted
├── secrets.toml                 # provider keys, bearer token (chmod 600)
├── repos/
│   └── <repo-name>/.git         # shared object storage per repo
└── worktrees/
    └── <job-name>-<id>/         # `worktree`-mode jobs only; reaped on completion.
                                 # `in_repo`-mode jobs (the default) edit the
                                 # user's existing clone directly — no entry here.
```

Per-job state that survives sessions — `.codeless/jobs/<name>.yaml`
(the template) and `runs/<name>/handover.md` + `log.md` (the
inter-session contract) — lives **in the user's repo** and is
committed. SQLite + worktrees above are the runtime's private cache.
See [`JOB-MODEL.md`](./JOB-MODEL.md) for the in-repo layout.

### Rule 1 — One transport interface, many implementations
<!-- enforced_by: crates/codeless-predicates/src/probes/direct_fetch.rs -->

All clients talk to the core through a **single typed RPC interface**, with the wire schema generated from Rust (source of truth) into TypeScript via [`specta`](https://github.com/oscartbeaumont/specta) + [`tauri-specta`](https://github.com/oscartbeaumont/tauri-specta). Hand-written client types are not allowed — clients silently drift otherwise.

Why `specta` over `ts-rs`: `specta` keeps browser SSE/REST types and Tauri IPC `invoke()` types generated from the same Rust source via `tauri-specta`, which is exactly what makes "the UI never knows which transport it got" actually true. `ts-rs` has no Tauri-IPC story, so picking it would force hand-written wrappers around `invoke()` in Phase 5 — a violation of this rule.

Hedge: `tauri-specta` is still pre-1.0 and has had churn. The Phase 1 commitment is just the Rust→TS generation for the SSE/REST surface (stable `specta` features only); the Tauri-IPC binding is exercised in Phase 5. If `tauri-specta` is genuinely blocking by then, generate the Tauri-IPC types separately with `specta` and write the thin glue ourselves — still one source of truth, just one extra step.

#### The `RpcClient` interface (TS)

The interface every UI module imports. Sketched here so Phase 1 has something concrete to build:

```ts
interface RpcClient {
  // Request / reply — REST POST in browser, invoke() in Tauri
  call<M extends RpcMethod>(method: M, args: RpcArgs<M>): Promise<RpcResult<M>>;

  // Streaming subscriptions — SSE in browser, Tauri 2 Channel<T> in Tauri
  subscribe<E extends EventTopic>(
    topic: E,
    filter: EventFilter<E>,
    sinceCursor?: string,
  ): AsyncIterable<EventOf<E>>;

  // PTY — WebSocket in browser, Tauri 2 Channel<T> over a long-lived command in Tauri
  openPty(sessionId: string): PtyChannel;

  // Blob / file upload — REST multipart in browser, Tauri fs plugin in desktop
  uploadBlob(target: BlobTarget, data: Blob): Promise<BlobRef>;
}
```

Tauri 2's [`Channel<T>`](https://v2.tauri.app/develop/calling-rust/#channels) is what makes the streaming and PTY cases tractable — it gives the Tauri shell a typed event stream from Rust to JS without a roundtrip per message. Without `Channel<T>` the streaming case would force transport details into call sites and Rule 1 would collapse.

| Client | Transport | Phase |
|---|---|---|
| Browser | **SSE** (events) + **REST** (commands) + **WS** (PTY sessions only) over HTTPS, authenticated | **MVP** (Phase 3) |
| CLI (local) | Direct in-process call into `codeless-runtime` | Phase 2 |
| CLI (against hosted core) | Same SSE + REST + WS as browser, authenticated via API token | Phase 3 |
| Desktop (Tauri) | Tauri IPC, typed through the same RPC trait | Phase 5 |
| Mobile (Tauri 2) | Same SSE + REST + WS as browser, same auth | Phase 6 |

If a feature works on one client it works on all of them, because they call the same surface. The browser MVP proves the hosted transport early; desktop and mobile then drop onto the same wire format.

#### Why SSE for events (not WebSocket)

Codeless traffic is asymmetric: clients send *commands* (REST POSTs — start job, approve review, comment, stop) and receive *events* (job state changes, AI token streams, tool calls, verify output). That's the exact shape SSE was designed for. SSE is plain HTTP — works through every proxy and CDN, native `Authorization` header support, native `Last-Event-ID` resume. `axum::response::Sse` makes the server side trivial; `EventSource` (browser) and a `reqwest`-based stream (Tauri) cover the clients.

**On mobile, SSE handles the foreground case only.** When the app is backgrounded, iOS kills the connection within ~30s; backgrounded clients learn about events via APNs/FCM (Phase 5) and reconcile the missed window via SSE replay using `Last-Event-ID` on next foreground. Push is therefore not a feature — it is the *required other half* of the event delivery story on mobile.

**WebSocket is reserved for PTY sessions only.** A live terminal is genuinely bidirectional, latency-sensitive, and binary-ish, and earns its own transport. Everything else stays on SSE.

### Rule 2 — The AI loop is in the runtime, not the frontend

The job runtime, the agent loop, tool dispatch, and the approval gate all live in `codeless-runtime`. Provider keys live server-side (secrets file in personal hosted mode, OS keychain on desktop, KMS or vault in multi-tenant hosted mode if that ever ships). The frontend renders state and submits intents — it never calls providers.

### Rule 3 — SQLite is the source of truth for runs

YAML/TOML defines job templates. Once a job starts, SQLite is authoritative. All clients render from the same store. Closing the window does not lose work. SQLite paths come from a **path-provider abstraction**, not hardcoded — desktop uses `dirs`, mobile uses platform-specific app-private dirs (iOS Application Support, Android app-private), hosted uses a configured volume.

### Rule 4 — Event schema expresses DAG state from day one

Phase 2 *executes* stages and tasks linearly. The event *schema* must already be able to describe a DAG. These coexist: events carry the relationship; the scheduler ignores it and runs in order. Topological scheduling can land later (no Phase 1/2 commitment) without a wire-format break.

Concretely, a `task-enqueued` event carries `depends_on: TaskId[]` (empty array in linear execution). A linear-only schema would omit that field, and adding it later forces every client to be re-released. With it present from day one, switching to topological execution later is a runtime change only.

The relationship lives on `task-enqueued`, not `task-started`, because dependencies determine *whether/when a task can run*. By the time `task-started` fires, all entries in `depends_on` are already in a `completed` state — `task-started` doesn't need to repeat them. Phase 2 emits both events back-to-back (deps are always satisfied at enqueue time in linear mode); Phase 7+ may have non-trivial gaps between them.

Example — `task-enqueued`, linear vs. DAG-ready:

```jsonc
// Linear-only (rejected): no way to describe parallelism later
{ "type": "task-enqueued", "task_id": "t2", "stage_id": "s1" }

// DAG-ready (adopted): linear today (depends_on always [previous]), parallel later
{ "type": "task-enqueued", "task_id": "t2", "stage_id": "s1", "depends_on": ["t1"] }
```

### Rule 5 — One UI framework, forever

If a Tauri 2 mobile feature genuinely doesn't work, write a thin native Tauri plugin — do **not** fork to a second UI framework. The "one React codebase, four shells" promise dies the day a `Foo.swift` or `Foo.kt` UI lands. (Cross-referenced from open questions; surfaced as a Rule because violations are silent and irreversible.)

## Repos — first-class entity, workspace per job (in_repo or worktree)

You manage **many repos** with Codeless, and **many jobs run concurrently on the same repo**. That makes repos a first-class entity in the data model and forces a real answer to the workspace question.

### Data model

A `repos` table sits alongside `jobs`, with each job carrying a `repo_id`. A repo row holds:

- Clone URL (HTTPS or SSH)
- Default branch
- Local clone path on the host
- Git auth method (SSH key path, `GITHUB_TOKEN`, GitHub App, etc.) — per repo, because different repos may belong to different orgs
- Per-repo concurrency cap (default: global cap)
- Per-repo default coding runner (so most jobs don't have to specify)

**Identity**: a repo is identified internally by `repo_id` (UUID). The user-facing handle is `name` — unique within the install, user-chosen at `codeless repo add` time, defaulting to the URL's last path segment. Two `repo` rows may point at the same clone URL (e.g. you want to track `acme-prod` and `acme-staging` against the same remote with different default branches/auth). The CLI accepts `<name>` everywhere; conflicts at add-time fail with a clear "name already taken — pick another or use `--name`" error.

Listing jobs is filterable by repo (`codeless jobs --repo foo`, `GET /api/jobs?repo=foo`). The browser shows a repo-grouped view by default.

### Workspace mode — `in_repo` (default) or `worktree` (opt-in)

A job picks where its edits live via `Job.workspace_mode`:

| Mode | Where edits land | When it's right |
|---|---|---|
| `in_repo` (**default**) | The user's existing local clone of the repo, on a fresh branch `codeless/<job-name>` created at submit. Files the agent edits are the files in the user's normal working copy. | The developer is at their desk, watching one job, wants their normal git tooling to "just work" — `git log`, `git diff`, IDE, dev server — all pointed at the agent's branch the same way they would for a human contributor's branch. The dogfooding default. |
| `worktree` | A separate `git worktree add` checkout (e.g. `/tmp/codeless-worktrees/<job-id>` or under `.codeless/worktrees/`), on the same job-specific branch. Same `.git`, different working dir. | Many jobs run concurrently against the same repo (the overnight / fleet use case), or the user wants strong physical isolation so a runaway job can't scribble in the checkout they're editing. The original constraint-2 use case. |

Both modes commit to a job-specific branch
`codeless/<job-name>` (or `codeless/<slug>-<id>` if the slug
collides). Merging back into the user's target branch is an
explicit step the user (or a follow-up job) performs — squash,
rebase, or `--no-ff` is the user's call, not the system's, so the
project's git history stays the user's history.

**Concurrency rule.** Only **one `in_repo` job per repo at a time** —
attempting to submit a second one returns a clear conflict ("repo
X is already in use by job Y in `in_repo` mode; stop it or submit
as `worktree`"). `worktree` jobs have no such limit and bound only
on the global / per-repo concurrency caps.

**The chosen workspace persists for the job's lifetime, not the
session's.** A long unsupervised run is many fresh sessions firing
in sequence against the same checkout — every session attaches to
the same working dir, sees what the previous session committed,
picks up where it left off via `runs/<name>/handover.md`. The
workspace is reaped only when the job reaches a terminal state
(`completed`, `failed`, `stopped`) — never between sessions. In
`in_repo` mode "reaped" means the branch is left in place for the
user to merge or delete; in `worktree` mode the worktree directory
is `git worktree remove`'d (the branch still survives in the
shared `.git` for review).

**Why `in_repo` is the default.** Worktrees solve a real
concurrency problem but introduce a foreign-checkout step that
makes the most common single-developer flow feel wrong — the user
can't `cd` into their own repo to test the change, can't see the
branch in `git branch`, can't use their normal dev server against
agent edits without first rebuilding from a `/tmp/...` path. For
the dogfooding and personal-project use cases that's friction with
no payoff, because there's only one job running anyway. Worktrees
remain a first-class mode because the overnight-fleet use case
needs them; they're just no longer the default.

The roadmap entry that lands `workspace_mode` is
[`PROGRESS.md` "Next steps" R0](./PROGRESS.md#r0--workspace_mode-in_repo--worktree-s-blocking-real-dogfooding).

### Hard rules for the coding runner

Independent of which runner is configured, these rules are
load-bearing for the long-run constraint:

1. **Sessions reset at autonomous stage advancement, not at every stage end.**
   - *Within a stage* a runner session is continuous. Cost-cap and
     wall-clock-cap mid-stage are **pauses**, not terminations; the
     user stopping a stage to ask a question is a **pause**; a
     daemon restart while a stage is in flight is recoverable. The
     runtime captures `RunResult.session_id` on the stage row (see
     `Stage.session_id`) and the next runner invocation against
     that stage passes `--continue <session_id>` (or the
     provider-equivalent) so the agent picks up the same
     conversation: same in-context files, same half-formed plan,
     same "I just considered approach X and rejected it." This is
     what makes pause / ask / resume / raise-the-cap feel like
     Claude Code instead of an alien tool, and it stops every
     interruption from costing a fresh codebase-exploration.
   - *Autonomous stage advancement resets the session.* When the
     loop itself promotes stage N → stage N+1 (verify green, no
     human in the loop), the fresh agent re-onboards from the
     workspace (whichever `workspace_mode` the job chose), the
     previous stage's `handover.md`, and the repo's `CLAUDE.md` /
     `CODELESS.md`. This is what keeps context bounded across an
     8-hour run — stages bound context; sessions do not have to.
     A stage's `session_id` is **never** reused by a *later* stage.
   - *Interactive resumption on a halted, failed, or
     completed-but-not-promoted stage continues the same session.*
     If a stage fails verify, hits a review gate, or is stopped by
     the user, and the user later opens the stage chat and sends
     a message, the runtime resumes with `--continue <session_id>`
     against the same conversation. The session is "warm" — the
     agent already knows what it just tried, what failed, and why.
     This is the debug-and-fix path; forcing a fresh session here
     would mean re-deriving five minutes of context the user is
     trying to act on. The user can opt into a hard reset
     (**`new session + handover`**) when the conversation has
     become polluted and a clean re-onboard is cheaper than
     continuing.
   - *Idle warm sessions have a timeout.* A warm session held
     open for interactive resumption costs RAM and (for CLI
     runners) a subprocess. After `session_idle_timeout` (default:
     30 min, configurable) the session is archived to disk; the
     next user message transparently becomes a `new session +
     handover` against that archive.
   - The user controls stage boundaries (commit + verify gate);
     codeless does not silently roll the session over from one
     stage into the next. See [`LOOP-CODER.md`](./LOOP-CODER.md)
     "What blocks this today" for the failure mode the
     advancement reset prevents, and [`JOB-UI.md`](./JOB-UI.md)
     for the UI surfaces (`Stage-N` live chat, `rerun now` vs.
     `new session + handover`) that expose the warm-session /
     reset distinction to the user.
2. **Push after every stage, never defer.** A session that commits
   but doesn't push leaves the next session reading a stale remote.
3. **A failed verify is operator-visible.** The default behaviour is
   *halt the stage* — no silent retry, no skip-and-continue — and
   write the failure into `handover.md` so the user sees it on wake.
   Per-stage policy may opt into a *bounded* retry-with-feedback
   (capped attempts, counted against the job's cost + wall-clock
   caps, and the verify output is folded into the next session's
   prompt) or escalation to a `Review` row. Unbounded retry, silent
   skip, and any behaviour the operator can't see in the morning
   review queue all stay forbidden. The autonomy roadmap that lands
   this lives in [`PROGRESS.md` "Next steps" A3](./PROGRESS.md#a3--verify-fail-policy-agent-decides-retry-vs-escalate-m).

## Concurrency model — scheduler + queue + caps

The runtime is a **scheduler over a job queue**, not a single-job state machine. Concrete shape:

- **Global concurrency cap** — how many jobs run at once across the whole core (default: small, e.g. 4).
- **Per-repo concurrency cap** — how many jobs can run on the same repo (default: 2 — enough to parallelise, not enough to exhaust the box).
- **Per-runner concurrency cap** — how many sessions of `ClaudeRunner` can run concurrently (the Claude Code CLI is RAM-hungry; this prevents OOM).
- **Queue** — jobs submitted past the cap sit in a `queued` state. The event stream emits `job-queued`, `job-promoted` (cap freed up), `job-started`.
- **Backpressure visibility** — the API and UI both expose queue depth and the user's position. The CLI can say "queued, 3 ahead."

The scheduler is `tokio`-based: a single supervisor task owns the queue and spawns per-job tasks under the cap. No external job queue (Redis, NATS) — SQLite is the queue. Standard "leased rows + heartbeat + reaper on restart" pattern.

This commitment is **Phase 2**, not "later if needed." The moment you have two repos, you need it.

**Scale: this is intentional for one-box, dozens-of-jobs-at-a-time, single-tenant.** It is not a forever-architecture.

> **What "single-tenant" means here**: one trust boundary. Many concurrent jobs across many repos, but they all run with the privileges of one logical owner — the OS user that owns the core process. It does not mean "one user, one job at a time." Phase 7 ("multi-tenant") is the version where multiple trust boundaries coexist on the same core, with sandboxing between them.

The known ceilings: SQLite write contention starts to bite around tens of writes/sec sustained, and `leased rows + heartbeat` polling does not span hosts. **Phase 7 replaces the queue** with whatever the multi-tenant runtime needs (Postgres + LISTEN/NOTIFY, NATS, etc.) — not now. If single-tenant load ever pushes SQLite past ~50 concurrent jobs, that's the signal to revisit early; until then, no.

## Disconnected operation — clients come and go, jobs don't

The core's whole point is that jobs survive client disconnect. This forces a few decisions:

- **Event retention.** Every event is persisted to SQLite. Retention is by *job lifetime + N days* (default: 30 days after job completion). Cheap, bounded, queryable. Expected scale per box (single-user, dozens of jobs/day): well under 10 MB/month of event rows after token-deltas are excluded. Indexing strategy: `(job_id, cursor)` covers the common "replay since cursor for this job" path; `(created_at)` covers the retention sweep. `VACUUM` runs nightly; the retention sweep runs hourly. If the row count exceeds expectations (rare event storms, runaway tool-call loops), the per-job cost / wall-clock caps catch it first.
- **Catch-up cursor.** Clients reconnecting pass `Last-Event-ID`; the core replays everything since that cursor before resuming the live stream. Cursors are monotonic per stream and stored alongside events.
- **Token-stream replay policy.** AI token streams (per-token deltas) are replayed if the client reconnects mid-stream, but not retained long-term — only the final assistant message is kept after the response completes. This keeps SQLite from bloating with per-token rows.
- **Approval timeout.** Review gates do **not** time out by default. A job awaiting approval stays awaiting approval for as long as it takes. The user can configure a per-job timeout that auto-stops or auto-rejects, but the default is "wait forever, the user will get to it."
- **Outbound notifications.** When a job needs attention (review requested, job failed), the core posts to a configurable webhook (ntfy.sh, Pushover, Discord, generic webhook). Email via SMTP is the boring fallback. This is a `Notifier` trait with pluggable backends, **Phase 2** — not Phase 6.

## Cost — visible and cappable from day one

For direct-API coding runners, runaway agents are expensive. Cost is treated like time: tracked, displayed, and bounded.

- Each `Event` emitted by an API runner carries `input_tokens`, `output_tokens`, and `cost_usd` (computed from the model's published rate).
- Running cost is summed per task, per stage, per job, and exposed on every state-fetch.
- **Per-job cost cap** (configurable, default: $5). Hitting the cap cancels the runner via `CancellationToken` and transitions the job to `stopped-cost-cap`.
- **Daily and monthly caps across all jobs** (configurable, default: $20 / $200). Hitting the daily cap blocks new API-driven jobs until the next day; queued jobs sit waiting.
- CLI-wrapper runners are out of scope for cost tracking — the vendor handles billing, and we have no token visibility into Claude Code's internal calls.
- **Wall-clock cap is the CLI-wrapper equivalent of the cost cap** (configurable, default: 8 h/job to match the long-run use case). Without it a stuck `claude` session would run unbounded against the user's Pro subscription. Same `CancellationToken` mechanism, same `stopped-wall-clock` terminal state. Direct-API runners get the wall-clock cap *too* (cost cap and time cap are independent fuses). **Tiebreaker**: if both fire effectively simultaneously, the terminal state is whichever cap's cancel-event has the lower event cursor. The supervisor doesn't race them — the first observed `cap-tripped` event wins and the second is suppressed.

**Phase 2** delivers cost tracking, per-job cost caps, and per-job wall-clock caps. Daily/monthly caps land with Phase 3 (browser UI to display and configure them).

**Loop-level aggregate ceilings (open item, planned post-MVP).** Per-job caps don't protect a long unsupervised run where 20 jobs each spend up to their cap; the user wakes up to "$100 spent, no useful progress". A two-tier `Continue` / `Escalate` / `Halt` ceiling on cumulative loop cost and wall time, evaluated by the runtime before firing each session, is the highest-priority budget feature after the per-job caps. Tracked here because SCOPE is the doc contributors read first; design intent and the rubix-borrow context live in [`LOOP-CODER.md`](./LOOP-CODER.md) "Ideas we should borrow" #1.

## Security model — single-user trust, full host blast radius

The MVP is a **single-user, single-tenant tool you run on a box you trust**. It is not multi-user-safe. State this plainly so no one ships it as one.

### Auth (Phase 3)

- **Bearer token** lives in a host-side config file (e.g. `~/.config/codeless/auth.toml`), generated by `codeless auth init` on first run. The same token authorises browser, CLI, and mobile clients — there is no separate browser/CLI credential.
- **Browser session**: the bearer token is exchanged for an HttpOnly + Secure + SameSite=Strict session cookie at `/api/auth/login`. UI code never sees the token. Cookies have a configurable lifetime (default 30 days) with sliding renewal.
- **CLI / mobile / Tauri-IPC**: `Authorization: Bearer …` header on every request. Tokens stored in OS keychain on desktop, in iOS Keychain / Android Keystore on mobile (biometric-unlocked), in `~/.config/codeless/auth.toml` for the CLI.
- **Rotation**: `codeless auth rotate` mints a new token and invalidates the old one. No expiry by default; rotation is the user's responsibility (documented in deployment guide).
- **Threat model**: a leaked bearer is full host-side RCE — the core runs `claude` / `codex` with full repo write access *and* exposes a PTY. The token must be transported only over HTTPS (Caddy/Cloudflare Tunnel/Tailscale Funnel terminate TLS) and never logged.

This is **explicitly not multi-user-safe** — one token, full host access. Phase 7 replaces this with OIDC and per-tenant sandboxing.

### PTY surface — the biggest blast radius in the product

A PTY in the core is a full shell on the host. Hard rules:

1. **Authn required**: the PTY WebSocket route requires the same bearer / cookie as every other route. No anonymous PTY.
2. **Scope per session (open-time only)**: a `pty` session is created via REST (`POST /api/pty?cwd=<worktree-path>`) and the WS attaches by `session-id`. The `cwd` at open is **constrained to a known worktree**; PTYs cannot be opened against arbitrary host paths. Once the shell is alive, however, the user can `cd /` and run anything the core's OS user can run — there is **no in-PTY sandboxing in MVP**. The cwd-on-open check is a footgun guard, not a sandbox.
3. **No privileged escalation**: the PTY runs as the same OS user that owns the core process. We do not `sudo`, `setuid`, or grant any capability the parent doesn't already have.
4. **Idle and lifetime caps**: PTY sessions die after configurable idle (default 15 min) and absolute lifetime (default 4 h). Sessions are not orphaned across core restarts — they're killed and the row is reaped.
5. **Logged**: every PTY open/close is structured-logged with the user identity, requested cwd, and resolved worktree.

This is still a "RCE on your dev box if the bearer leaks" surface. The mitigations above are about preventing accidental footguns and making misuse detectable, not about defending against a token compromise.

### Auth-agnostic transport

The SSE + REST + WS surface treats auth as middleware: the route handlers see an `AuthedUser` extracted by middleware and don't care whether it came from a bearer header, a session cookie, or (Phase 7) OIDC + JWT. Swapping bearer → OIDC for Phase 7 is a middleware change, not a wire-format break for clients. **This is a Phase 3 design constraint.**

### Secrets file format (Phase 1)

- Path: `~/.config/codeless/secrets.toml` (XDG-respecting on Linux, `~/Library/Application Support/codeless/secrets.toml` on macOS, `%APPDATA%\codeless\secrets.toml` on Windows).
- Format: TOML, hand-editable, but managed via `codeless secrets <verb>` so users don't have to. Verb is plural-`secrets` everywhere; matches the file name and the underlying concept of a set of named entries.
- Contents: API keys per provider (`anthropic`, `openai`, `openai_compatible[<name>]`), git auth credentials (PATs, SSH key paths), bearer token. One row per logical secret.
- **Encryption at rest**: not in MVP. The file is `chmod 600`, owned by the core's OS user. Hosted mode runs on a box you trust; multi-tenant hosted (Phase 7) replaces this with KMS / vault.
- **Never logged, never sent to client**. The frontend gets a list of *which* secrets are set, never the values.

### Crash recovery — what happens when a runner dies mid-task

- **In-flight `task` rows** carry a `lease_holder` and `lease_expires_at`. The `lease_holder` string is `<pid>:<startup-nonce>` where `startup-nonce` is a `Uuid::new_v4()` minted once per core process start. (We considered using the Linux boot ID, but it's not portable to macOS or Windows, and PID-reuse inside a single boot — fast container restarts, namespaced restarts — would still match a dead leaseholder. A per-startup UUID dodges both problems.) On core restart, the supervisor scans for tasks whose `startup-nonce` doesn't match the current process and transitions them to `failed-runner-crash`, with the partial cost recorded.
- **Worktrees**: a job whose task crashed leaves its worktree on disk. The reaper either preserves it (default — user can inspect / re-run from where it was) or removes it (configurable). It does not silently delete user-visible work.
- **PTY sessions**: killed on core restart, no resume.
- **Cost**: partial cost from a crashed task is recorded. The per-job cost cap counts it.

## Worktree model — caveats

Worktrees are the right answer (see Repos section), but two failure modes deserve a sentence:

- **LFS / submodules / hooks**: out of scope for MVP. Worktrees share `.git` but interact badly with `core.hooksPath` and LFS smudge filters. Documented limitation; the workaround is "use repos without LFS or non-trivial submodules."
- **Disk pressure**: the dominant cost is *working trees*, not git objects. N concurrent worktrees × `node_modules/` or `target/` is what fills the disk, not the shared `.git`. The per-repo concurrency cap (default 2) is the disk knob. Document this in the deployment guide; do not pretend "shared objects" makes worktrees free.

## What we reuse from Terax (and what we don't)

Terax is a real codebase, not a template. Reuse must be honest about what is actually portable.

### Reuse (visual layer)

- CodeMirror 6 editor configuration and theming
- xterm.js terminal component (with a new PTY transport — see below)
- File explorer React component
- AI message / streaming markdown rendering
- Approval-card UI component
- Settings shell, providers form layout
- Tailwind + shadcn/ui setup
- Project-memory concept (`TERAX.md` → `CODELESS.md`)

### Replace

| Terax piece | Why it's replaced |
|---|---|
| `src/modules/ai/lib/agent.ts` (AI SDK loop) | Moves to Rust core |
| `src/modules/ai/lib/transport.ts` | Moves to Rust core |
| `src/modules/ai/tools/*` (TS tool definitions) | Tools now run in Rust; TS keeps only the schema for UI rendering |
| `src/modules/ai/lib/native.ts` (direct Tauri invokes) | Replaced by the typed RPC client |
| `src/modules/ai/lib/keyring.ts` (Tauri keychain only) | Replaced by an `RpcSecrets` interface with desktop + hosted backends |
| Zustand chat/session stores | Replaced by store-backed-by-RPC: state lives in the core, UI subscribes |
| `src-tauri/src/modules/pty` | Keep on desktop; in hosted mode PTY is exposed over WebSocket |
| `src-tauri/src/modules/shell` | Same — execution happens in core, transport differs |

### Drop

- Telemetry/account opinions inherited from Terax (we'll make our own choices)
- Updater plugin assumptions (re-evaluate for hosted mode)
- Any hard assumption that `cwd` is a local path the UI can see

## One UI, four shells

**The UI is a single React + TypeScript codebase.** All four delivery targets — browser (MVP), desktop, iOS, Android — render the *same* components from the *same* source tree. There is no platform fork.

[Tauri 2](https://github.com/tauri-apps/tauri) supports macOS, Windows, Linux, iOS, and Android natively, and the same React frontend builds as a static bundle for browser delivery. That makes a single UI codebase achievable, so we commit to it — even though only the browser shell ships in the MVP.

```
codeless-ui  (single React + TS codebase)
    ├── shell: browser   (static build → hosted core via SSE + REST + WS)   [MVP]
    ├── shell: desktop   (Tauri 2 → local runtime via Tauri IPC)             [Phase 5]
    ├── shell: ios       (Tauri 2 → hosted core via SSE + REST + WS)         [Phase 6]
    └── shell: android   (Tauri 2 → hosted core via SSE + REST + WS)         [Phase 6]
```

The UI imports a single `RpcClient` interface. The shell decides which implementation gets injected:

- Browser, iOS, Android → `HttpSseClient` (SSE for events, REST for commands, WS for PTY)
- Tauri desktop → `TauriIpcClient` (Tauri `invoke` under the hood)

The UI never knows which transport it got. That is the contract. Building the browser shell first proves the contract under the *harder* transport (network); desktop drops in afterwards with the easier one (in-process IPC).

### Rules that make one codebase work

1. **No UI module imports `@tauri-apps/api/core` directly.** All backend calls go through `RpcClient`. This is the rule that makes mobile and browser viable; if it leaks, the plan collapses. (Terax violates this in ~35 files today — that is the work we are explicitly absorbing.)
2. **Responsive design is mandatory.** Same component tree, breakpoints decide layout. Terminal panel collapses to a tab on phones; file explorer becomes a drawer; review approval cards reflow. No `Foo.web.tsx` / `Foo.mobile.tsx` files.
3. **Behavioural differences (clipboard, file picker, share sheet, biometric unlock) live behind shell-injected interfaces**, never inside components.
4. **Touch and keyboard from day one.** Don't ship a desktop-only interaction model and "add touch later" — retrofits never happen.

### Per-target capability matrix

| Concern | Desktop | Browser | iOS / Android |
|---|---|---|---|
| Runtime location | Local (`codeless-runtime` in-process) | Hosted core | Hosted core |
| Filesystem | Local FS | Hosted workspace | Hosted workspace |
| Terminal / PTY | Local PTY | PTY in core, streamed over WS | PTY in core, streamed over WS |
| Coding CLI (`claude` / `codex` / `copilot`) | Runs on user's machine | Runs on the host box (single-tenant) | Runs on the host box (single-tenant) |
| Provider keys | OS keychain | Secrets file on the host, never sent to client | Secrets file on the host, never sent to client |
| Auth | None (local user) | Bearer token in config (personal hosted) | Bearer token, biometric-unlocked on device |
| File upload / `git clone` | N/A | Hosted core | Hosted core |
| Push / background | N/A | Browser notifications | APNs / FCM via Tauri 2 plugin |
| Build artefact | `.dmg` / `.msi` / `.AppImage` | Static bundle behind the API | `.ipa` (App Store) / `.aab` (Play Store) / `.apk` (sideload, F-Droid, dev installs) |

The browser shell ships in the MVP (Phase 3). Desktop follows (Phase 5); mobile follows (Phase 6). If the transport rules hold and the `RpcClient` boundary holds, both are packaging exercises, not re-architectures.

## Runtime — plain async, not an ECS

The job runtime is a staged state machine, not a real-time simulation. We use:

- `tokio` for async
- `sqlx` for SQLite
- A plain Rust state machine (enum + transitions) for Job → Stage → Task
- `tokio::sync::broadcast` (or similar) for the event bus

We explicitly **do not** adopt an ECS such as [Bevy](https://github.com/bevyengine/bevy), [hecs](https://github.com/Ralith/hecs), or [Flax](https://github.com/ten3roberts/flax). ECS is a worldview built for real-time simulation; importing it for a staged-job state machine is overkill and would pull contributors into a paradigm that doesn't match the problem. If we later need DAG-style parallel task fan-out, the right move is a topological sort + worker pool, not an ECS.

## Coding loop — CLI wrappers OR direct APIs, user picks per job

> **What codeless is for.** VS Code, Cursor, JetBrains, Claude Code
> already do editing, chat, and refactor well. Codeless does not
> compete with them. Codeless does the one thing they don't:
> **runs a long, unsupervised coding session for hours, across
> many fresh agents, without losing context or burning the budget.**
>
> This section covers *which runner* a job uses (CLI vs direct API).
> The design intent for the long-run model lives in
> [`LOOP-CODER.md`](./LOOP-CODER.md). The user-facing framework —
> `.codeless/` layout, job YAML, handover.md, log.md — lives in
> [`JOB-MODEL.md`](./JOB-MODEL.md), with a worked walkthrough in
> [`JOB-EXAMPLE.md`](./JOB-EXAMPLE.md). **The current MVP target —
> "codeless develops codeless, visibly, from the browser" — is
> spec'd in [`DOGFOOD-MVP.md`](./DOGFOOD-MVP.md);** read that
> before starting any UI or wiring work. Operational rules the
> *codeless developers* follow when building codeless live in
> [`JOB-LOOP.md`](./JOB-LOOP.md) — that doc is for building
> codeless, not for what codeless does for end users.

**Rule:** The coding loop is whatever the user configures for the job. Both modes are first-class:

| Coding mode | Driver | Key required? | Cost model | Notes |
|---|---|---|---|---|
| **CLI wrapper** | Claude Code, Codex, GitHub Copilot CLI | No — uses the user's existing vendor login on the host | Counts against the user's vendor subscription | Best when you already pay for Claude Pro / Copilot |
| **Direct API** | `AnthropicRunner`, `OpenAIRunner` (existing in `ai-runner`), local OpenAI-compatible endpoint (LM Studio / Ollama) | Yes — per-provider key in the core's secrets store | Per-token billing, visible in the UI, cost-cappable per job | Best when you want fine-grained control, local models, or no vendor lock-in |

Both modes implement the same `Runner` trait; the job template names which runner to use. Switching between them is a one-line change in the job's YAML.

### Helper role — Rig, optional, never gates a job

> Persona / subagent / runner separation, and how chat-side personas extend into job stages without inventing a new template system, is in [`AGENT.md`](./AGENT.md). Personas are *advisory context for a runner* under the rules below — they shape the prompt, they do not become a fourth runner.


Independent of which runner drives coding, **Rig** ([0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig)) powers an optional set of *non-coding* helpers: planning, post-stage review summaries, commit-message generation, job-memory search, cheap-model routing for trivial calls.

#### Why Rig over AutoAgents / ADK-Rust

All three are real, active, Rust-native — but they target different layers:

| Library | Shape | What it owns |
|---|---|---|
| **Rig** | LLM client library | The API call, retries, streaming, tool schemas, embeddings, vector stores |
| AutoAgents | Actor-based agent framework | All of the above + the agent loop + actor environment + multi-agent pub/sub + WASM tool sandbox + guardrails |
| ADK-Rust | Agent platform (Rust port of Google ADK) | All of the above + sessions + workflow agents + graph orchestration + A2A protocol + realtime voice + deployment story |

Codeless already owns the outer layer — its scheduler, state machine, event bus, SQLite, and `Runner` trait. We need a **library that fills the LLM-API hole inside our runners**, not a framework that wants to own the outer loop.

- **AutoAgents** wants to own `Task`, the ReAct loop, the `Environment`, agent lifecycle, and memory. Our runner would become a 200-line shim disabling most of it. Smaller community (~627 stars), single commercial vendor.
- **ADK-Rust** wants to own `Runner`, `Session`, `RunConfig`, telemetry, deployment. Pre-1.0 churn (breaking minor versions every few months), ~322 stars, no named external adopters. Proc-macro tool model is wrong for a job runner that needs dynamic per-job tool schemas.
- **Rig**: library shape. Broadest provider coverage (Anthropic, OpenAI, Gemini, Cohere, Mistral, Bedrock, VertexAI, Ollama, HF, OpenAI-compat). Broadest vector-store coverage (SQLite, LanceDB, MongoDB, Neo4j, Pinecone, Qdrant, SurrealDB). ~7.2k stars. Doesn't impose an outer loop.

Rig is also the right cross-platform choice: as a pure library it slots into `codeless-runtime` cleanly, while AutoAgents' actor environment and ADK-Rust's session/runtime would each pull platform-coupled deps that fight our crate-split rules.

Re-evaluate this decision if any of the following happen:

- Rig stops being maintained (currently active, no signal of this).
- We need a feature only the other two ship — WASM tools, A2A, realtime voice — none of which are on the roadmap.
- A clear winner emerges in the Rust agent-framework space and the community migrates.

Cherry-pick candidates if Rig falls short later: `adk-rag` (if Rig's vector-store ergonomics aren't enough), AutoAgents' WASM-sandboxed tool execution (if we ever expose user-uploaded tools). MCP support is in all three, so it's not a differentiator.

Hard rules for the helper role:

1. **A job must run end-to-end with zero helpers configured.** If no planner is configured, the user writes stages in YAML. If no reviewer is configured, the review screen shows raw diff + verify output. Helpers enhance; they never gate.
2. **Helpers never inject context into a CLI-wrapper coding session.** Claude Code / Codex do their own context gathering; we don't fight them.
3. **Helpers stay out of the coding loop.** No "Rig agent that writes code." Coding goes through a `Runner`, not through Rig.

> **RAG operates at the job/stage level, not the file/symbol level.** Memory surfaces past work to humans (and to planners producing the next job's stages). It does not try to be a code-aware retrieval system competing with the coding CLI's own context gathering.

Where Rig is genuinely useful long-term:

- **Commit-message summariser** *(Phase 2 nice-to-have, lands when the
  coder loop works end-to-end)*: takes the staged diff + the stage
  title and produces a commit message. The smallest possible first
  Rig use, proves the helper boundary holds, free quality win on
  every tick.
- **Stage-result reviewer feeding the `Review` table** *(high-payoff
  for the long-run use case)*: when a stage's title is prefixed
  `REVIEW`, the runtime materialises it into a `Review` row and the
  operator wakes up to a queue of reviews. A Rig-backed reviewer
  attaches a "what changed, what's risky, what I'd ask a human
  about" summary to each row. Per hard rule #1, raw diff + verify
  output must still work without it.
- **Job-memory RAG** *(highest-payoff Rig use against the long-run
  constraint — see [`LOOP-CODER.md`](./LOOP-CODER.md))*: SQLite-backed
  vector store of past job summaries, prior `runs/*/log.md`, and
  `CODELESS.md` notes. A fresh-session-per-session agent has no memory
  of prior dead ends; RAG lets it look them up instead of carrying
  them in-prompt. This is the bridge between "fresh agent every tick"
  and "the loop gets smarter over time".
- Planner: convert user goal → stages/tasks, user-reviewable before
  the job runs. Defer until ~10 real jobs exist to train against — a
  bad plan poisons the whole job, and hand-written YAML/TOML
  templates are the right Phase 1 answer.
- Summariser (other roles): titles, release notes, search-answer
  generation.
- Cheap-model routing: titles and summaries on a small model, review
  on a smart one. Also the right home for the future "did this
  verify output mean success?" check when codeless drives user repos
  with custom `verify.sh`.

## Multi-repo dev setup — use `mani`

Codeless development spans multiple repos (this repo, the vendored/forked `ai-runner`, the Terax-derived UI). Set up a `mani` workspace to manage them rather than juggling them by hand. `mani` already does cross-repo `status` / `fetch` / `branch-switch` / `commit` — see [block-flutter-workspace/MANI.md](/home/user/code/flutter/block-flutter-workspace/MANI.md) for the pattern. The `mani.yaml` lives at `~/code/rust/mani.yaml` (the existing parent of `codeless/`) — see "Directory and repo layout" above. No new workspace parent dir is created.

## Runner layer — adopt the `rubix-agent/ai-runner` crate

We do **not** design the provider-runner abstraction from scratch. The existing `ai-runner` crate from the `rubix-agent` workspace already has the right shape and is battle-tested. **Adoption mode: vendored** (copied into `ai-runner/` at the workspace root, no `.git` of its own, patched in-place; every patch logs a row in [`ai-runner.PATCHES.md`](../ai-runner.PATCHES.md) and leaves a `// codeless-patch-NNN` marker in source). Crate is internally controlled — same author / org as Codeless.

- A `Runner` trait with typed `CliCfg` / `RestCfg` inputs, streaming events through a bounded `mpsc::Sender<Event>`, and `CancellationToken`-based cancellation.
- `ClaudeRunner` built on `claude-wrapper`, including production-quality binary discovery (env override → PATH → well-known bin dirs → editor-shipped copies in VS Code / Cursor / vscode-server / Windsurf) and MCP HTTP config.
- `CodexRunner` for the Codex CLI.
- `AnthropicRunner` and `OpenAIRunner` for REST providers.
- Defined backpressure semantics: REST runners `.await` sends; CLI runners use `try_send` from sync stream callbacks and `warn!` on overflow.
- `kill_on_drop` for CLI children, `select!` against cancel for REST.

Codeless adopts this crate (vendored, forked, or workspace dependency — TBD) and builds the job runtime on top of it. New work goes into:

- A `RigHelperRunner` (helper role — planner/reviewer/summariser) that lives alongside the existing runners.
- Job/stage/task orchestration in `codeless-runtime`, calling `Runner::run` for each provider session.
- Persistence of `RunResult` + streamed `Event`s into SQLite.

This means the "provider runner abstraction" is **not** a Phase 1 deliverable — it already exists. Phase 1 wires it into `codeless-runtime`.

## CLI surface

**The CLI and the MCP server must each be able to do everything the GUI can do.** The GUI is a convenience; the two headless surfaces are not. Every feature ships with CLI + MCP coverage on day one, and parity is a CI check (see "MCP surface" below), not a convention. The MCP surface mirrors this CLI command-for-command, so anything documented here is also reachable as an MCP tool unless it carries an explicit `opt_out_reason` (today: `codeless secrets get/set` and `codeless auth *` — secret material does not cross an MCP boundary).

> **Minimum dogfoodable CLI surface** (Phase 1 → stable from Phase 2): `codeless run --once`, `codeless chat`, `codeless tail`, `codeless session attach`. These four are what makes "AI agents and devs can test against the CLI before the browser shell exists" real. Everything below is the full Phase 2 surface — implement the four first, then the rest.

```bash
# Repos (Phase 2)
codeless repo add <git-url> [--name foo] [--branch main] [--auth ssh:~/.ssh/id_ed25519]
codeless repo list
codeless repo remove <name>

# Jobs (Phase 2) — multi-repo, multi-concurrent
codeless job create <repo> job.yaml
codeless job start <job-id>
codeless job status <job-id>
codeless job logs <job-id> [--follow]
codeless job stop <job-id>
codeless jobs [--repo foo] [--status running|queued|review|completed|failed]

# Ad-hoc, no YAML (Phase 1, stable from Phase 2)
codeless run --repo <repo> --once "<prompt>"   # single-turn, fire-and-forget
codeless chat --repo <repo>                    # interactive REPL against the same runner

# Reviews (Phase 2)
codeless review list
codeless review approve <review-id>
codeless review comment <review-id> "Change X before continuing"
codeless review stop <review-id>

# Secrets (Phase 1) — manages the chmod 600 secrets.toml
codeless secrets set <key> [value]             # value from arg, env, or stdin; never logged
codeless secrets get <key>                     # returns the value; refuses to print without --reveal
codeless secrets rm <key>
codeless secrets list                          # names only, never values

# Auth (Phase 3) — manages the bearer token used by browser/CLI/mobile
codeless auth init                             # mints the bearer, writes ~/.config/codeless/auth.toml
codeless auth rotate                           # mints a new bearer, invalidates the old
codeless auth show                             # prints whether a token exists, never the value

# Sessions / global (Phase 2)
codeless session list
codeless session attach <session-id>           # re-attach to a running session from another client
codeless tail                                  # global event stream — "what's happening right now"
codeless provider list

# Against a hosted core (Phase 3)
# By default, --core picks up the bearer from ~/.config/codeless/auth.toml.
# --token / $CODELESS_TOKEN override that file.
codeless --core https://codeless.example.com <any-of-the-above>
codeless --core https://codeless.example.com --token $CODELESS_TOKEN <any-of-the-above>
```

**Local CLI bypasses auth.** When `--core` is not set, the CLI calls `codeless-runtime` in-process and there is no transport — auth would be a self-loop. Bearer / cookie auth is only enforced by `codeless-server` over HTTP. The local CLI runs with the OS-user privileges of the invoker, which is the right model: same trust boundary already.

`codeless run --once` and `codeless chat` are the dogfood-from-day-one entry points — no YAML, no state machine, just talk to a runner against a repo. `codeless tail` and `codeless session attach` are what make "drive the core from any client" feel real.

## SSE + REST + WS surface (Phase 3 — the browser MVP runs on this)

```
# Repos (REST)
POST   /api/repos
GET    /api/repos
GET    /api/repos/{name}
DELETE /api/repos/{name}

# Jobs (REST)
POST   /api/jobs                            # body: { repo, template | prompt, runner, cost_cap }
GET    /api/jobs?repo=&status=
GET    /api/jobs/{id}
POST   /api/jobs/{id}/start
POST   /api/jobs/{id}/stop

# Reviews (REST)
GET    /api/reviews?status=pending
POST   /api/reviews/{id}/approve
POST   /api/reviews/{id}/comment
POST   /api/reviews/{id}/stop

# Cost (REST)
GET    /api/cost/summary?range=today|month
GET    /api/cost/caps
PUT    /api/cost/caps                       # body: { per_job_usd, daily_usd, monthly_usd }

# Events (one SSE stream per client, with Last-Event-ID resume)
GET    /api/events?since=<cursor>&repo=<name>   # text/event-stream; ?repo= optional filter

# PTY (the only WebSocket route)
WS     /api/pty/{session-id}
```

The SSE stream carries: `job-queued`, `job-promoted`, `job-started`, `stage-started`, `task-enqueued`, `task-started`, `tool-call`, `tool-approval-requested`, `ai-token` (streaming delta), `ai-message-complete`, `cost-updated`, `verify-started`, `verify-passed`, `verify-failed`, `review-requested`, `cap-tripped`, `job-completed`, `job-stopped`, `job-failed`. PTY frames travel over the dedicated WebSocket route only.

gRPC is **not** in scope for the MVP. It can come later for runner-to-core comms in a distributed deployment.

## MCP surface (Phase 2 — headless control plane for AI agents)

Codeless exposes the **same operations** as the SSE+REST+WS surface through an MCP server, so any MCP-capable agent (Claude Code, Codex, a future bespoke automation) can drive jobs without a UI. This is a load-bearing requirement: "drive Codeless from an agent over MCP, with no UI running" is one of the product's primary use cases. The UI is never allowed to be the only way to do anything.

The concrete implementation plan — which crates are involved, the
`codeless-tools-jobs` translator layer, slice-by-slice rollout — lives
in [`AGENT-CONTROL-PLANE.md`](./AGENT-CONTROL-PLANE.md). Read that
alongside this section before doing the work; this section is the
contract, that doc is the build plan.

### Transports

- **stdio MCP** — the agent spawns `codeless mcp` (or `codeless-mcp`) as a child process. This is the default for local single-user dev and matches Claude Code's expectation for MCP servers. No bearer token: same trust model as the local CLI (R5 / "Local CLI bypasses auth"). The MCP child does **not** carry its own runtime — it is an `RpcClient` translator that connects back to the long-running `codeless-server` daemon, so every job it submits, polls, or cancels lives on the same SQLite and event bus as the browser UI and CLI. Multiple agents (Claude Code, Codex, Copilot) sharing a single daemon is the whole point of this transport, and an in-process runtime per child would defeat it.
- **Streamable HTTP MCP** — `codeless mcp --listen :PORT` (or `codeless-mcp --listen`) for a long-running hosted core that multiple agents can attach to. Reuses the same bearer token as `codeless-server`. The MCP HTTP server is the same axum process or a sibling — the choice is an implementation detail, but they share the auth middleware.

### Tools (each mirrors a method on the RPC trait)

```
codeless.repo.add | .list | .remove
codeless.job.create | .start | .stop | .status | .list
codeless.job.run_once                # ad-hoc, no-YAML entry point — the dogfood path
codeless.job.chat                    # interactive REPL turn, returned as a streamed tool result
codeless.review.list | .approve | .comment | .stop
codeless.cost.summary | .caps.get | .caps.set
codeless.session.list | .attach
codeless.provider.list
codeless.secrets.list                # names only; .get/.set are intentionally CLI-only
```

### Resources (read-only, subscribable)

```
codeless://repos                     # full repo list
codeless://jobs                      # all jobs, filterable
codeless://jobs/{id}                 # job state + stage tree
codeless://events?since=<cursor>     # the same event stream as SSE, served as an MCP resource subscription
codeless://reviews?status=pending
```

Resource subscriptions are the agent-facing equivalent of the SSE stream: the MCP host re-delivers updates as `notifications/resources/updated` so the agent can react to `review-requested`, `verify-failed`, `cap-tripped`, etc. in real time.

### Prompts

The job templates registered under `codeless job` are surfaced as MCP prompts so an agent's host can present them to the user (or fill them in itself) before kicking a job off.

### Parity is a CI check, not a convention

Adding a method to the RPC trait without adding it to the MCP tool list **fails the build**. The same check covers the CLI: every RPC method has a CLI entry point and an MCP tool, or carries an explicit `opt_out_reason` in a small registry that CI reads. This is how the rule "CLI and MCP each do everything the GUI does" survives drift over time. The mechanism (proc-macro, build.rs, or a hand-maintained table validated by a unit test) is left to the implementer; the *check* is mandatory.

### What never crosses the MCP boundary

`codeless.secrets.get`, `codeless.secrets.set`, and the entire `codeless.auth.*` surface are **not** exposed as MCP tools. An MCP client is an LLM running prompts that may be attacker-controlled (a malicious doc, a poisoned tool result, a prompt-injection inside a repo). Secret material and the bearer-token lifecycle stay under the user's direct hand — CLI for local, browser-with-bearer for hosted. The `opt_out_reason` for these is `"secret material; out-of-band only"`.

**Asymmetry between stdio and HTTP MCP.** Both transports surface
the same tool list (minus secrets/auth above). They differ in their
security boundary:

- **HTTP MCP** — bearer token authenticates the *client*. The user
  has explicitly granted the bearer to an agent host they trust.
- **stdio MCP** — there is no bearer; the child process inherits
  the user's local trust. The real security boundary is the **MCP
  host's per-tool approval UX** (in Claude Code: the "approve this
  tool call?" prompt). Tools with side-effects beyond reading are
  still exposed because the host gates each call.

`codeless.secrets.*` and `codeless.auth.*` are excluded from *both*
transports because per-call approval is not a strong enough boundary
for them: a single approved call could read the entire bearer token
or rotate it. These operations are irreversible-in-aggregate and
belong out-of-band regardless of how good the host's approval UX is.

## Provider runners

All runners implement the existing `Runner` trait from `ai-runner` (see "Runner layer" above). Three categories, one trait:

1. **CLI-wrapper coding runners** — `ClaudeRunner`, `CodexRunner`, future `CopilotRunner`. Spawn the vendor CLI as a child process with stdio piped through, stream events, cancel via `kill_on_drop`. **No API key** — CLIs use the user's existing vendor login on the host. In personal hosted mode that's one login on your box; multi-tenant is an open question (see below).
2. **Direct-API coding runners** — `AnthropicRunner`, `OpenAIRunner`, plus an OpenAI-compatible runner targeting LM Studio / Ollama / vLLM. **API key required**, stored in the core's secrets store. Cost is metered per token and reported on the event stream; per-job cost caps cancel the runner via `CancellationToken` when the cap is hit.
3. **Helper runner** — a `RigHelperRunner` wraps Rig for the helper role (planner, reviewer, summariser, RAG). Activated only when the user configures a provider for helpers (which can be the same provider as the coding runner, or a cheaper model).

The job's YAML picks one runner from category 1 or 2 to drive the coding loop. Category 3 attaches independently.

## Testing strategy

Correctness across many concurrent jobs is the whole product, so testing is a Phase 1 deliverable, not a polish task.

- **`codeless-types` and `codeless-rpc`**: unit tests on the serde round-trip and the wire schema. `specta` snapshot tests on the generated TS — the test re-runs codegen and `git diff --exit-code`s the output against a committed snapshot. CI fails if the generated TS changed without a matching snapshot update in the same PR. This forces clients to be updated in lockstep with Rust schema changes.
- **`codeless-runtime`**: state-machine unit tests per transition (job → stage → task → review). Property tests on the queue: caps respected, no double-lease, recovery from a killed leaseholder. Integration tests using an in-memory SQLite and a `MockRunner` that drives the event stream from a script.
- **Concurrency**: at least one test that runs ~20 short jobs across 3 repos under tight per-repo caps and asserts ordering + cap invariants. `loom` for the supervisor/lease logic if it gets gnarly; not by default.
- **`codeless-adapters-host`**: worktree manager tested against a real local repo. PTY tested with a tiny `echo`-style child. CLI runners tested with a fake `claude`-style binary on PATH (Cargo-feature-gated test helper, not shipped). Tests **set `PATH` explicitly** to point at the fake binary; we never trust the developer's host `claude` install for tests, otherwise CI is flaky on machines that happen to have the real CLI installed (or not).
- **`codeless-server`**: end-to-end tests with `axum::testing` — bearer auth, SSE replay via `Last-Event-ID`, PTY happy-path + idle expiry.
- **No mocks for the database**. SQLite is fast enough to run real per-test.
- **CI**: GitHub Actions, `cargo test --workspace`, `cargo clippy -D warnings`, `cargo fmt --check`. The TS side runs `tsc --noEmit` and `vitest` (if components have logic).

## Observability — the operator story

The event bus is the user story. The operator story (when the core misbehaves) is separate:

- **`tracing` crate with structured spans** across the job → stage → task hierarchy. Span fields include `job_id`, `stage_id`, `task_id`, `repo`, `runner_kind`. One span per task; tool calls are child spans.
- **`tracing-subscriber`** writes JSON to stdout in hosted mode (so Docker/systemd journals are queryable) and pretty-prints in dev.
- **No external collector dependency for MVP.** OpenTelemetry export is gated behind a Cargo feature and a config flag — opt-in, not opt-out.
- **Health endpoint**: `GET /api/health` is **unauthenticated** and returns only non-sensitive coarse counts (`{status, uptime, active_jobs, queued_jobs, db_size_mb}`). It exists for Tailscale Funnel / Cloudflare Tunnel probes, which need to hit it without holding a bearer. It deliberately omits anything user-identifying or workload-shaped (no repo names, no job IDs).
- **Audit log**: structured-logged auth events (login, token rotate, PTY open) go to a separate `audit.log` (configurable path). Excluded from the event retention sweep, but **rotated by size**: when `audit.log` reaches 10 MB it's renamed to `audit.log.1`, gzipped to `audit.log.1.gz`, and a new `audit.log` is opened. We keep the most recent 10 rotated files and delete older ones. This is the core's job, not the operator's — no logrotate dependency.

## Database migrations

SQLite is the source of truth, so schema migrations are load-bearing.

- **`sqlx::migrate!`** with versioned, forward-only migrations in `migrations/`. No down-migrations — we don't roll back; we forward-fix.
- **Phase 1 ships the initial schema**. Every later schema change is a new migration file.
- **Event schema changes are append-only**: new event types and new fields on existing types are fine; renaming or removing fields is not (clients depend on them). Rule 4 (DAG-ready schema from day one) is what minimises mid-flight breakage here.
- **Backup before migrate**: on startup the core copies the SQLite file to `db.backup.<timestamp>` before running pending migrations. A separate **file-retention sweep** (distinct from the event-row sweep) runs daily and deletes `db.backup.*` files older than 30 days. Most recent 3 backups are always kept regardless of age, so a fresh install with a few migrations doesn't immediately lose them.
- **Migration failures**: refuse to start, log the error, surface it on the health endpoint. The user fixes it manually before retrying.

## Phases

The MVP is **Phase 3**: browser UI talking to a single-tenant hosted core on a box you own, managing many concurrent jobs across many repos. Everything before Phase 3 builds the substrate. Everything after is optional.

> **CLI-first dogfooding.** The CLI is the *first usable surface*, not just the power-user one. Phase 1 ships `codeless run --once`; Phase 2 ships the full multi-repo runtime over the CLI. AI agents (including Codeless dogfooding itself) and developers test against the CLI before the browser shell exists. The browser MVP arrives in Phase 3, but the product is already real and drivable from Phase 2.

### Phase 1 — Core skeleton + transport rule + thinnest possible run

- Stand up the full crate split from day one: `codeless-types`, `codeless-rpc`, `codeless-runtime`, `codeless-adapters-host`, `codeless-server` (empty stub), `codeless-mcp` (empty stub), `codeless-client`, `codeless-cli`, `codeless-tauri-desktop` (empty stub). Stubbing the optional shells now keeps the dependency direction honest, and stubbing `codeless-mcp` from Phase 1 forces the RPC trait to stay transport-agnostic (Phase 2 fills it in). (`codeless-adapters-desktop` is created later — see crate table.)
- Define the RPC trait and an in-process implementation
- Wire-type generation from Rust to TypeScript via `specta` (+ `tauri-specta` ready for Phase 5)
- SQLite schema with path-provider abstraction for **repos**, jobs, stages, tasks, sessions, events, reviews — event schema expressive enough for DAG state on day one
- Adopt `ai-runner` (vendored — see crate table) so `ClaudeRunner`, `CodexRunner`, `AnthropicRunner`, `OpenAIRunner` are available from day one — process-spawning runners gated behind a Cargo feature so future thin-client builds exclude them
- Port the Terax Rust modules (`pty`, `shell`, `fs`) into `codeless-adapters-host` behind the RPC
- Worktree manager: `git worktree add` / `remove` wired up, reaper on startup
- **Thinnest possible end-to-end run**: `codeless run --once --repo <repo> "<prompt>"` invokes a chosen runner once and streams its events to stdout. No state machine, no YAML, no review gate. This is the first dogfoodable surface — use it on Codeless itself from Phase 1 onwards.
- **Maintain `CLAUDE.md` at the repo root** (created during workspace bootstrap) capturing the rules from this scope that an AI agent (or human) must follow when touching code: the crate dependency-direction rule, the no-`@tauri-apps/api/core` rule, the no-`Foo.web.tsx`/`Foo.mobile.tsx` rule, and the `RpcClient`-only-import rule for UI modules. Without this file, every agent run starts from zero context and silently violates the rules that make the cross-platform plan work.
- **Testing baseline**: `cargo test --workspace` green on day one, with the state-machine unit tests, the in-memory SQLite + `MockRunner` integration harness, and the queue property tests in place. CI on GitHub Actions: test + clippy + fmt.
- **`tracing` baseline**: structured spans across job → stage → task wired through from Phase 1, JSON to stdout in hosted mode.
- **`sqlx::migrate!` baseline**: initial schema as a versioned migration, pre-startup backup, forward-only migration policy.
- **Secrets file scaffolding**: `codeless secrets set/get/rm/list` operating on a `chmod 600` TOML file.

### Phase 2 — Multi-repo, multi-job runtime (CLI)

- Repos as first-class entity: `codeless repo add/list/remove`
- Worktree-per-job execution
- Job template loader (YAML), referencing a repo and a runner choice (CLI wrapper or API)
- Job/stage/task state machine in `codeless-runtime` (plain `tokio` + `sqlx`, no ECS)
- **Scheduler + queue + concurrency caps** (global, per-repo, per-runner)
- End-to-end job run against both a CLI-wrapper runner (`ClaudeRunner`) **and** a direct-API runner (`AnthropicRunner`) — proves both coding modes work on the full stage/task structure
- Cost tracking from API runners; per-job cost cap that cancels via `CancellationToken` when hit
- Review approval flow, gated through CLI commands
- Resumable jobs after a core restart (worktree state + queue state both recover from SQLite)
- `codeless chat` / `codeless tail` / `codeless session attach` — the "drive from any terminal" surface
- **`codeless-mcp` server** (stdio + Streamable HTTP) exposing the full RPC surface as MCP tools/resources/prompts — so Claude Code, Codex, and any other MCP-capable agent can drive Codeless headless from Phase 2 onward, before the browser shell exists
- **Parity check in CI**: every RPC method has a CLI command *and* an MCP tool, or an explicit `opt_out_reason` in a registry that the test reads — failing this fails the build
- Outbound notification webhook (ntfy.sh / Discord / generic) on review-requested and job-failed
- Smoke endpoint for the SSE stream: delivers `job-queued`, `job-started`, `stage-started`, `task-completed`, `job-failed` events end-to-end (happy path *and* failure path)

### Phase 3 — Browser shell (THE MVP)

This is the phase that turns Codeless into the product you actually use.

- Fill in `codeless-server` (axum binary exposing the SSE + REST + WS surface)
- Stand up the single `codeless-ui` React + TS package; commit to the `RpcClient` boundary
- Bring across Terax UI components (editor, terminal, explorer, AI chat panel)
- Repo-grouped jobs dashboard — see all repos, all jobs, all states at a glance
- Per-job stage/task timeline with live event stream via SSE
- Review approval UI with diff + verify-output
- PTY terminal in-browser (xterm.js over WS)
- Ad-hoc `run --once` and `chat` surfaces in the UI, not just the CLI
- Cost display per job, daily total, daily/monthly cap configuration
- Same `CODELESS.md` project memory concept
- Responsive layout from the start (touch + keyboard, breakpoints not platform forks — so Phase 6 mobile is a packaging exercise)
- Auth: bearer token + HttpOnly session cookie for the browser, bearer for CLI / mobile. Full model in the Security section. OIDC deferred to Phase 7.
- **CLI-against-hosted**: `codeless --core https://… --token …` makes the CLI use `codeless-client` instead of in-process runtime
- Deployment: single binary + Caddy / Cloudflare Tunnel / Tailscale Funnel for HTTPS. Documented end-to-end.

**By the end of Phase 3 you have the product:** a hosted Codeless on your box that you drive from a browser, managing many jobs across many repos, with the CLI as the power-user surface.

### Phase 4 — Helper agents via Rig

- Add `RigHelperRunner` alongside existing runners
- Planner agent: user goal → proposed stages/tasks (user reviews before run)
- Reviewer agent: post-stage diff + verify-output summary
- Summariser: titles, commit messages, release notes
- Job-memory store (Rig + SQLite vector store) over past job summaries
- Helpers remain **strictly optional** — `codeless` must still run with none configured

### Phase 5 — Tauri desktop shell

- `codeless-tauri-desktop` ships: same `codeless-ui` React code, `TauriIpcClient` injected
- Local-mode runtime (no server hop) for users who want the GUI without running a hosted core
- Native file dialogs, OS-keychain secrets backend
- `codeless-adapters-desktop` fills in (created here, once it has more than one thing in it)
- Desktop is **not** the MVP. It exists because the UI is already cross-shell — building it is a packaging exercise, not a re-architecture.

### Phase 6 — Mobile shells (iOS + Android via Tauri 2)

- Stand up `codeless-tauri-mobile` (depends **only on** `codeless-types` + `codeless-client`)
- Same `codeless-ui` React code as browser and desktop
- Push notifications (APNs / FCM) for review-requested and job-completed events — this is the *required other half* of mobile event delivery, not an enhancement (see SSE rationale)
- Biometric unlock for token storage
- Responsive layout already in place from Phase 3 — mobile reuses, doesn't redesign
- No coding-CLI execution on device (would not pass App Store review and is hostile to Play Store policy); mobile is always a thin client to a hosted core

### Phase 7 — Multi-tenant hosted SaaS (only if it becomes the product)

If Codeless graduates from "personal tool" to "thing other people pay for," this phase exists. Everything in here is **out of scope until then**:

- Real auth (OIDC) replacing the single-bearer-token model
- Per-tenant sandboxed runners (container or microVM)
- KMS / vault for per-tenant secrets
- Hosted workspace model (workspaces in the core, not on the user's machine)
- Per-tenant quotas, billing, rate-limiting
- Distributed runner protocol (gRPC) for offloading job execution off the API host
- The hard open question of per-user `claude` / `codex` authentication in a multi-tenant sandbox

> Phase numbering note: there is no Phase 3.5. Helpers landed as Phase 4 because half-numbered phases tend to slip indefinitely. If helpers need to ship inside the browser MVP for a particular release, fold them into Phase 3's scope explicitly rather than introducing a half-phase.

## Appendix A — Phase 1 SQLite schema sketch

This is the schema implementers should land in Phase 1's first migration. Types are SQLite-native (`INTEGER`, `TEXT`, `BLOB`); the Rust side maps them via `sqlx`. **All IDs are ULID stored as TEXT** (sortable, URL-safe, no UUID-ordering pain in the queue). **Money is stored as `INTEGER` cents-USD** (no floats, no rounding surprises). **Timestamps are `INTEGER` Unix-millis UTC.**

```sql
CREATE TABLE repos (
  id              TEXT PRIMARY KEY,           -- ULID
  name            TEXT NOT NULL UNIQUE,       -- user-facing handle
  clone_url       TEXT NOT NULL,
  default_branch  TEXT NOT NULL,
  local_path      TEXT NOT NULL,              -- path to the shared .git: $XDG_DATA_HOME/codeless/repos/<name>/.git
  git_auth        TEXT NOT NULL,              -- JSON: { kind: "ssh"|"token"|"github_app", ... }
  concurrency_cap INTEGER,                    -- NULL → use global cap
  default_runner  TEXT,                       -- e.g. "claude" | "anthropic" | NULL
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
);

CREATE TABLE jobs (
  id                TEXT PRIMARY KEY,
  repo_id           TEXT NOT NULL REFERENCES repos(id) ON DELETE RESTRICT,
  status            TEXT NOT NULL,            -- queued|running|awaiting-review|completed|failed|stopped
  stop_reason       TEXT,                     -- e.g. user|cost-cap|wall-clock|runner-crash; NULL when running/completed
  template_yaml     TEXT,                     -- NULL for ad-hoc jobs
  prompt            TEXT,                     -- ad-hoc prompt; NULL when template-driven
  runner            TEXT NOT NULL,            -- the chosen Runner kind
  branch            TEXT NOT NULL,            -- e.g. codeless/job-<id>
  worktree_path     TEXT,                     -- NULL until started; preserved on crash
  cost_cap_cents    INTEGER NOT NULL,
  wall_clock_cap_ms INTEGER NOT NULL,
  cost_cents        INTEGER NOT NULL DEFAULT 0,
  started_at        INTEGER,
  ended_at          INTEGER,
  created_at        INTEGER NOT NULL
);
CREATE INDEX jobs_status_idx ON jobs(status);
CREATE INDEX jobs_repo_idx   ON jobs(repo_id, created_at);

CREATE TABLE stages (
  id          TEXT PRIMARY KEY,
  job_id      TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  ordinal     INTEGER NOT NULL,
  name        TEXT NOT NULL,
  status      TEXT NOT NULL,                  -- pending|running|awaiting-review|passed|failed
  verify_cmd  TEXT,                           -- shell snippet; NULL = no verify
  started_at  INTEGER,
  ended_at    INTEGER
);
CREATE INDEX stages_job_idx ON stages(job_id, ordinal);

CREATE TABLE tasks (
  id                TEXT PRIMARY KEY,
  stage_id          TEXT NOT NULL REFERENCES stages(id) ON DELETE CASCADE,
  ordinal           INTEGER NOT NULL,
  status            TEXT NOT NULL,            -- enqueued|running|completed|failed|cancelled
  depends_on        TEXT NOT NULL DEFAULT '[]', -- JSON array of TaskId; empty in linear mode
  lease_holder      TEXT,                     -- "<pid>:<startup-nonce>" or NULL when idle
  lease_expires_at  INTEGER,
  cost_cents        INTEGER NOT NULL DEFAULT 0,
  input_tokens      INTEGER NOT NULL DEFAULT 0,
  output_tokens     INTEGER NOT NULL DEFAULT 0,
  started_at        INTEGER,
  ended_at          INTEGER
);
CREATE INDEX tasks_stage_idx        ON tasks(stage_id, ordinal);
CREATE INDEX tasks_lease_expiry_idx ON tasks(lease_expires_at) WHERE status = 'running';

CREATE TABLE reviews (
  id            TEXT PRIMARY KEY,
  stage_id      TEXT NOT NULL REFERENCES stages(id) ON DELETE CASCADE,
  status        TEXT NOT NULL,                -- pending|approved|rejected|stopped|rerun-requested
  comment       TEXT,
  requested_at  INTEGER NOT NULL,
  resolved_at   INTEGER
);
CREATE INDEX reviews_status_idx ON reviews(status);

CREATE TABLE events (
  cursor      INTEGER PRIMARY KEY AUTOINCREMENT, -- monotonic per install; serves as Last-Event-ID
  job_id      TEXT,                              -- NULL for global events (repo-added, etc.)
  stage_id    TEXT,
  task_id     TEXT,
  type        TEXT NOT NULL,                     -- 'job-started' | 'task-enqueued' | ...
  payload     TEXT NOT NULL,                     -- JSON, schema per event type
  created_at  INTEGER NOT NULL
);
CREATE INDEX events_job_cursor_idx ON events(job_id, cursor);
CREATE INDEX events_created_at_idx ON events(created_at); -- for the retention sweep

CREATE TABLE pty_sessions (
  id              TEXT PRIMARY KEY,
  job_id          TEXT REFERENCES jobs(id) ON DELETE SET NULL,
  cwd             TEXT NOT NULL,                 -- resolved worktree path at open
  opened_by       TEXT NOT NULL,                 -- AuthedUser identifier
  opened_at       INTEGER NOT NULL,
  last_activity   INTEGER NOT NULL,
  closed_at       INTEGER
);
CREATE INDEX pty_idle_idx ON pty_sessions(last_activity) WHERE closed_at IS NULL;
```

Notes:

- **Cursor as `INTEGER AUTOINCREMENT`** is monotonic across the install and serves directly as `Last-Event-ID` over SSE. Per-stream filtering happens at query time, not at cursor allocation.
- **`tasks.depends_on` is JSON, not a join table.** It's read-mostly, the cardinality is tiny (typically 0–3 entries), and a join table would force a multi-row write per task creation. Revisit if/when topological execution lands and we need graph queries.
- **No down-migrations.** This schema is the base; later migrations are forward-only `ALTER`s.
- **Vendor-locked to SQLite syntax** (e.g. `INTEGER PRIMARY KEY AUTOINCREMENT`). Phase 7's Postgres rewrite is a translation pass, not a schema port.

## Reproduction — second job-detail tab right-pane blank (S)

### Steps to reproduce

1. Open any job → a job-detail tab mounts; job A's `JobPage` becomes active.
2. Navigate to a stage tab inside job A (e.g. click Stage 1 in the overview). The URL
   updates to `?tab=stage:<stageId-A>`.
3. Open a second job from the dashboard. A second job-detail tab appears;
   job B's `JobPage` mounts (inactive, hidden via Tailwind `hidden` class on
   its own root div).
4. Switch to job B's tab. Job A's `JobPage` root div becomes `display: none`
   (via `!active && "hidden"`). Job B's root div becomes visible.
5. **Observed**: job B's right pane is blank. No content visible. Switching
   back and forth between the tabs leaves job B always blank.

### Which pane is blank

The entire main content area of the second job-detail tab — tab bar renders,
page header renders, but the panel below the tab bar (where CHAT / SPEC /
Stages / StageDetail content would appear) shows nothing.

### Console errors

None. Both bugs identified below fail silently: the CSS layout issue is
purely visual; the URL-pollution issue causes a silent `null` from `Array.find`.

### Network / SSE state

- Two independent SSE connections are established, one per job (correctly keyed
  by `{ scope: "job", job_id: <id> }` in `joinSubscription`). Both are `live`.
- All HTTP RPC calls use the correct `job_id`. No failed network requests appear.
- The `list_stages` POST for job B returns HTTP 200 with job B's stages; the
  UI silently discards the result because it looks up the wrong `stageId`.

### Root cause — two bugs, one symptom

#### Bug 1 (primary) — `JobDetailStack` outer wrapper not hidden for inactive tabs

**File**: `src/modules/jobs/JobDetailStack.tsx`

```tsx
{jobTabs.map((t) => (
  <div key={t.id} className="h-full w-full">   // ← never hidden
    <JobPage jobId={t.jobId} active={t.id === activeId} />
  </div>
))}
```

`JobDetailStack` is mounted inside `<div className="absolute inset-0">` in
`App.tsx` (~line 970). The absolute container has a definite height. Each
outer wrapper div carries `h-full` (= `height: 100%`), which is an explicit
CSS property — the browser assigns the full height to the element regardless
of whether its child has `display: none`.

With two job tabs open:

- Wrapper div 1 (job A): `height: 100%` — occupies the top 100 % of the
  visible area. Child `JobPage` has `display: none` when job B is active, but
  that doesn't collapse the wrapper's layout footprint.
- Wrapper div 2 (job B): `height: 100%` — starts at 100 % of the visible
  area (below div 1) → entirely off-screen.

Result: the active job B is rendered but scrolled below the viewport. The user
sees a blank pane.

#### Bug 2 (secondary) — `JobPage` initialises `activeTab` from shared `window.location.search`

**File**: `src/modules/jobs/JobPage.tsx`, `useState` lazy initialiser (~line 70)

```ts
const [activeTab, setActiveTab] = useState<ActiveTab>(() => {
  const param = new URLSearchParams(window.location.search).get("tab");
  if (param?.startsWith("stage:")) {
    const stageId = param.slice("stage:".length);
    return { kind: "stage", stageId, stageName: stageId, pinned: false };
  }
  ...
});
```

`window.location.search` is a process-wide singleton shared by all mounted
`JobPage` instances. When job A navigates to `?tab=stage:<stageId-A>`, then
job B's `JobPage` mounts (inactive), it reads the URL and initialises its own
`activeTab` as `{ kind: "stage", stageId: "<stageId-A>", ... }`. When the
user switches to job B, `StageDetail` is rendered with
`jobId=B, stageId=<stageId-A>`. The `list_stages` call returns job B's stages;
the `find(s => s.stage.id === stageId-A)` returns `null`; `rollup` stays
null; the stage content area renders empty. This masks behind Bug 1 when the
layout is already broken, and surfaces independently once Bug 1 is fixed.

### Module-level state audit (Stage 2)

The job goal flags SSE subscription / chat store / `useJob` cache as suspects.
A read of `src/lib/rpc/hooks.ts`, `src/lib/rpc/http-sse-client.ts`,
`src/lib/rpc/provider.tsx`, `src/modules/ai/store/chatStore.ts`, and the
`JobPage` subtree (`JobPage`, `StagesOverview`, `StageDetail`, `RunPane`,
`JobChat`) shows that the rpc/SSE/chat-store singletons do **not** cross-
contaminate two `JobPage` instances. The only true cross-instance singleton
that does is `window.location` (Bug 2 above).

- **`SHARED_SUBSCRIPTIONS`** — `hooks.ts:237`, a
  `WeakMap<RpcClient, Map<string, SharedSubscription>>` keyed by
  `JSON.stringify({ filter, since })`. The filter includes `job_id`, so
  `JobPage A` (filter `{ scope: "job", job_id: A }`) and `JobPage B`
  (`job_id: B`) live under distinct map entries with distinct `buffer`,
  `listeners`, `stateListeners`, `lastStatus`, and `cancel`. Two
  EventSources, two replay buffers, no shared state. Sharing inside one
  jobId (e.g. `JobPage` + `StagesOverview` + `JobChat` for the same job all
  joining the same `(filter, since)` key) is the deliberate connection-
  pooling design and is correct. **Not the bug.**
- **`chatStore`** (`src/modules/ai/store/chatStore.ts`) — module-level
  `chats: Map<sessionId, Chat>`, `seedMessages: Map<sessionId, …>`,
  `pendingPersist: Map<sessionId, …>`, plus the zustand store. This is the
  global AI sidebar (Terax-inherited), keyed by AI chat-session id, not
  job id. `JobChat` in `RunPane.tsx` does **not** route through this store:
  it owns its own `useState` for history/streaming/liveItems and persists
  via `read_job_file`/`write_job_file` to `CHAT.md` inside the worktree.
  Two `JobPage` instances never collide on `chatStore` keys. **Not the
  bug.**
- **`useJob`** (`hooks.ts:111`) — no module-level cache. Each call owns its
  own `useState<QueryState<Job>>` plus a `tick` counter for refetch. Two
  `JobPage` instances issue two independent `get_job` RPCs and store the
  results in their own component state. **Not the bug.**
- **`useRepos` / `useAsyncOnce`** (`hooks.ts:20`) — same shape, per-component
  state. **Not the bug.**
- **`HttpSseClient`** (`http-sse-client.ts`) — stateless on the instance;
  each `subscribeWithState` call constructs a fresh `openManagedSse` closure
  with its own `EventSource`, stale timer, reconnect timer, cursor, and
  attempt counter. No module-level mutable state. **Not the bug.**
- **`window.location.search`** — the real module-level singleton.
  `JobPage`'s `useState<ActiveTab>` lazy initialiser (`JobPage.tsx:70-90`)
  reads `new URLSearchParams(window.location.search).get("tab")`
  unconditionally, both for active and inactive instances. With job A's
  URL parked at `?tab=stage:<A-stage>`, the freshly mounted (inactive)
  `JobPage` for job B initialises its own `activeTab` from that A-shaped
  stage id, asking `StageDetail` to render `jobId=B, stageId=<A-stage>` —
  the rollup lookup never matches and the content area renders empty.
  This is the Bug 2 already captured above; it is the only cross-instance
  module-level state coupling the two pages.

### Fix direction (for Stage 2+)

- **Bug 1**: add `cn("h-full w-full", t.id !== activeId && "hidden")` to the
  outer wrapper div in `JobDetailStack`. `hidden` = `display: none` keeps the
  React tree (and SSE subscription) mounted while removing the layout
  footprint.
- **Bug 2**: guard the `useState` initialiser with `if (active)` — inactive
  `JobPage` instances should not read the URL. Only the active page at any
  moment owns `window.location.search`.

## Out of scope (for now)

- Real-time multi-user collaboration on the same job
- Plugin / extension marketplace
- On-device coding-CLI execution on mobile (forbidden by platform policy, not just our choice)
- Multi-tenant SaaS deployment (deferred to Phase 7 — may never happen)
- Anything that requires forking the Terax desktop app cleanly back into upstream — we are not maintaining upstream compatibility

## Open questions

- **Default coding runner**: when a job's YAML doesn't name one, do we default to the CLI wrapper (free, requires login) or the API runner (metered, requires key)? Probably CLI wrapper if `claude` is on PATH, otherwise API runner if a key is configured.
- **Rig vector-store backend**: SQLite (matches our SQLite-first rule) vs. LanceDB/Qdrant (better at scale). Default to SQLite; revisit if memory search gets slow.
- **Tauri-mobile maturity** (Phase 6): if a specific feature genuinely doesn't work via Tauri 2 mobile (e.g. background execution), add a thin native Tauri plugin — do not fork to a second UI framework. Verify push notifications, biometric unlock, and SSE-over-HTTP behaviour on both iOS and Android during Phase 6.
- **Multi-tenant Claude Code login** (Phase 7, if it ever happens): each sandbox needs its own `claude` authentication, and no good solution exists today. This is the open question that may permanently keep Phase 7 out of scope. We do **not** anticipate a "user's desktop acts as the runner for their phone" peer-to-peer architecture — mobile is a thin client to a hosted core, full stop. Adding peer-to-peer later would be a re-architecture, not an extension, and the crate split deliberately does not anticipate it.
- **gRPC for distributed runners** (Phase 7): only earns its place if we ever distribute the runtime. Until then, in-process is fine.

### Settled (kept here for posterity, do not re-litigate)

- **LLM library**: Rig — see "Why Rig over AutoAgents / ADK-Rust" above.
- **Wire-type generator**: `specta` + `tauri-specta` — see Rule 1.
- **`codeless-runtime` mobile reach**: host-only — mobile is always a thin client. See crate table.
- **`codeless-adapters-desktop` timing**: created in Phase 5 when it has more than one thing in it. Until then it lives inside `codeless-tauri-desktop`.
- **Multi-repo dev setup**: `mani` workspace.
- **`ai-runner` adoption shape**: vendored into `ai-runner/` at the
  workspace root, no `.git` of its own, patched in-place. Every
  patch logs a row in `ai-runner.PATCHES.md` and leaves a
  `// codeless-patch-NNN` marker in source.
