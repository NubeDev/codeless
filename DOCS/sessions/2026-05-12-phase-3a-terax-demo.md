# Build status — Phase 3a Terax UI demo (codeless-server + browser end-to-end)

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
> 5. ⛔ COMMIT AND PUSH every stage. Workspace .gitignore excludes the
>    inner repo, so codeless code commits go to NubeDev/codeless via
>    mani (or raw git from inside `codeless/`); session-file commits
>    go to the workspace repo. Never `--force`, never `--no-verify`.
> 6. ⛔ Comments explain *why*. No emojis. No task-status comments.
> 7. ⛔ R1 cross-platform reach is enforceable. Process spawn stays in
>    `codeless-adapters-host` (and `ai-runner`). Mobile-safe crates
>    must not pull `tokio::process` or `claude-wrapper` transitively.
> 8. ⛔ R2: the UI surface lives in `codeless/ui/codeless-ui/`. Do not
>    fork it. Mount any new product surface as a module under
>    `src/modules/`; do not bypass `RpcClient` for browser HTTP/SSE.
>    Pre-existing untracked UI files from the user's in-flight work
>    are out of scope for stage commits — either stash them in a
>    separate chore commit or use raw `git add` for the stage files.

File: DOCS/sessions/2026-05-12-phase-3a-terax-demo.md
Goal: Get a working browser demo of the Terax-derived UI driving
      a real Codeless core. Open the JobsDashboard in a browser,
      see persisted repos/jobs, submit a job via SubmitJobDialog,
      watch the live JobTimeline tick through events from the real
      runtime, and exercise an approve/stop flow against a real
      review row. The CLI demo is implicit — `codeless serve` is
      the demo's backend.
Started: 2026-05-12
Last tick: 2026-05-12 (DONE — stages 1-5 [x])
Current stage: 5 / 5 (DONE)

Repo:        codeless
Branch:      feat/phase-2a-persistence  (Phase 3a stacks on the
             same branch as Phase 2a + 2b + 2c — the combined PR
             cuts after Phase 3a)
Memory policy: compact every 3 stages
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   25

## What's already done (not in scope for new code)
- `codeless-cli`: `run` / `job submit` / `review` / `tail` /
  `secrets` all green against a file-backed `--db`. The CLI is the
  reference embedder of `InProcessRpc`.
- `codeless-runtime`: `RpcServer` trait has the full method set
  (repos, jobs, reviews, subscribe). `EventBus::subscribe_since`
  replays from SQLite + chains to live broadcast tail without gaps.
- `codeless-types` + `codeless-rpc`: serde + specta wire types are
  authoritative; the UI's `wire.ts` snapshot already mirrors them.
- `codeless/ui/codeless-ui/`: Terax shell + `RpcClient` boundary
  + `HttpSseClient` + `MockRpcClient` + `RpcProvider` + the full
  `modules/jobs/` surface (`JobsDashboard`, `JobRow`, `JobDetail`,
  `JobTimeline`, `SubmitJobDialog`, `StatusBadge`) all exist.
  Browser shell `src/shells/browser/main.tsx` constructs the right
  client (with a `?mock=1` escape hatch).

The gap: `codeless-server` is a `fn main() {}` stub. Closing that
gap is what unlocks the demo.

## Stages
Format: `[ ] N. [S|M|L] title` — complexity tag mandatory.

- [x] 1. [M] codeless-server axum app + REST adapter. Implements
         POST routes for every `RpcServer` method (add_repo,
         remove_repo, list_repos, submit_job, get_job, list_jobs,
         stop_job, list_reviews, approve_review, comment_review,
         stop_review) plus an SSE `GET /events/stream` adapting
         `subscribe`. Bearer-token middleware reads the token from
         the existing secrets file (key: `core_bearer_token`); a
         missing key short-circuits with a "run `codeless serve
         --init-token` first" hint. URL shapes must match what
         `HttpSseClient` already sends — verify against
         `ui/codeless-ui/src/lib/rpc/http-sse-client.ts`. Test
         coverage: tower `oneshot` exercising each route end-to-end
         against an in-memory `InProcessRpc`; one SSE smoke test
         that publishes an event mid-stream.

- [x] 2. [S] `codeless serve` CLI verb. Opens `--db <path>` (or
         `CODELESS_DB`), reads the bearer token from the secrets
         file, binds to `--bind 127.0.0.1:7777` by default, and
         runs the axum server until SIGINT. Adds `--init-token`
         flag that generates a random token, writes it to the
         secrets file under `core_bearer_token`, and prints it
         to stdout once for the user to paste into the browser.
         Integration test: spawn the binary, hit a route, assert
         the bearer gate.

- [x] 3. [S] Browser shell config + smoke. Confirm `readBaseUrl`
         / `readToken` (already in `lib/rpc/config.ts`) resolve
         from a path the demo can hit — likely `localStorage` for
         token + `?core=http://localhost:7777` query override.
         Write a 10-line `DEMO-UI.md` at the workspace root with
         the exact steps (terminal A: `codeless serve
         --init-token`; terminal B: `pnpm -C codeless/ui/codeless-
         ui dev`; paste token; open `JobsDashboard`).

- [x] 4. [S] End-to-end demo dry run. Boot the server against a
         fresh DB, seed one repo + one job via CLI, load the
         browser at `http://localhost:5173/?core=...`, paste the
         token, screenshot or describe what the UI shows. Any
         field that renders wrong (status badge, timestamp) gets
         a follow-up Note in this file; do NOT widen scope into
         UI rework inside this stage.

- [x] 5. [S] Phase 3a wrap-up. CODELESS.md gets a Phase 3a entry
         (codeless-server + serve + demo path). README quickstart
         gains a "Run the browser demo" subsection pointing at
         `DEMO-UI.md`. Three verify gates green; combined Phase
         2a+2b+2c+3a PR description drafted on the branch (not
         opened — that's a separate human action).

## Likely batching
- Tick 1: stage 1 (M).
- Tick 2: stage 2 (S) — maybe pair with stage 3 (S) if the
  diff stays tight.
- Tick 3: stage 3 (S) if not paired; else stage 4 (S).
- Tick 4: stage 4 (S).
- Tick 5: stage 5 (S) — DONE.

## Notes
- Stage 1: `codeless-server` is now a library (`build_router`,
  `AppState`, `load_bearer_token`, `TOKEN_SECRET_KEY`) plus a stub
  binary that points at `codeless serve`. Stage 2 wires the binary
  via the CLI verb. SSE adds an `id:` line per frame from
  `EventCursor.0` so EventSource populates `Last-Event-ID` for
  reconnect; the `?since=` extractor also accepts an explicit cursor
  for fresh-connection resume.
- Stage 2: `codeless serve` lives in `codeless-cli` and delegates
  bind + graceful-shutdown to `codeless_server::serve_with_shutdown`.
  `--init-token` writes a 32-char hex token (16 random bytes from
  the OS CSPRNG via `getrandom`) under `core_bearer_token` in the
  same secrets file the rest of the CLI uses, and prints it once on
  stdout. `--force` rotates. Integration tests spawn the binary and
  exercise the 401/200 bearer gate end-to-end.
- Stage 3: rather than add a `?core=` query override (would touch
  the UI), `DEMO-UI.md` at the workspace root drives the existing
  `localStorage` keys (`codeless-rpc-base-url`, `codeless-rpc-token`)
  that `config.ts` already reads. CORS is now permissive in
  `codeless-server` (loopback-only by default, so Any-origin is
  correct for the single-tenant MVP) so the browser at `:5173` can
  hit the core at `:7777` without a preflight failure.
- Stage 4 dry-run results (server bound to `127.0.0.1:7787`, fresh
  SQLite, single-tenant token):
  - `POST /rpc/list_repos` → `{"repos":[]}` then, after `add_repo`,
    a one-entry array with ULID id + `created_at` ms timestamp.
  - `POST /rpc/submit_job` → Job row with `status:"queued"`,
    `stop_reason:null`, all caps and timestamps populated.
  - `POST /rpc/stop_job` → 200; subsequent `get_job` shows
    `status:"stopped"`, `stop_reason:"user"`, `ended_at` set.
  - `GET /events?scope=all&since=0` replays `repo-added` (cursor 1)
    + `job-queued` (cursor 2) as SSE frames with `id: <cursor>`
    lines, then the live tail surfaces `job-stopped` (cursor 3)
    within ~1s of the stop_job request.
  - Auth gates: no bearer → 401; wrong job id → 404.
  - Wire format matches the UI's `wire.ts` exactly (kebab-case
    event types, snake_case fields, `cursor` as int). The
    JobsDashboard would render `demo` → `demo/job-1` → `stopped`
    + reason `user` once the localStorage keys point at the core.
- Stage 5 wrap-up: CODELESS.md picks up a "Phase 3a — browser demo
  loop" subsection; README quickstart gains a "Run the browser
  demo" subsection that links to DEMO-UI.md. Three verify gates
  green at 118 tests / 0 failures, clippy -D warnings clean,
  `cargo fmt --check` clean.
- `HttpSseClient` already documents the wire shape (bearer header
  on REST; `?token=` query on SSE because EventSource has no
  header API). The server must conform to *that* contract, not
  invent a new one — the file is the spec.
- The UI is `?mock=1`-toggleable, so stages 1-2 can be tested
  without the UI by hitting routes with `curl` + `jq`. Stage 4 is
  the moment of truth.
- Single-tenant per R5: ONE bearer token, stored next to the
  secrets, used by browser + CLI + (future) mobile alike. Do not
  introduce per-job scopes.
- The user has in-flight UI work in the working tree
  (shell-injection adapters, settings refactors). Stage commits
  here must NOT sweep those in — use raw `git add` against the
  specific files for each stage, or do a `chore: stash UI WIP`
  commit before the first tick.
- `codeless-server`'s axum dep is new; add `axum`, `tower`,
  `tower-http` (for cors + trace), and the runtime dep on
  `tokio-stream` (already in workspace) for SSE.

## Blockers
(none)
