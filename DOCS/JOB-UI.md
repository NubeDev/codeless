# JOB-UI — the single-job page

The job page is the surface the user lives in once they've kicked off
a job. It answers three questions at a glance:

1. **Where is the agent right now?** Which stage, which tick.
2. **What's green, what's red?** Per-stage and per-tick pass/fail.
3. **Can I talk to it?** Yes — per stage, on a still-warm session.

This doc is the load-bearing UX spec. Cross-references:

- Stage / tick / session semantics: [`SCOPE.md` hard rule #1](./SCOPE.md#hard-rules-for-the-coding-runner)
- Event schema feeding this UI: [`SCOPE.md` Repo → Job → Stage → Task table](./SCOPE.md#what-each-level-means)
- Run-log primitives (lifecycle dividers, error cards): [`JOBS-UX.md`](./JOBS-UX.md)
- Multi-job dashboard (out of scope here): a future doc

## The page is one page, not many

There is **one** job page. Everything — overview, spec, per-stage
detail, chat — happens inside it as **tabs**. No separate routes for
"stage detail page" or "chat page". Tabs are the unit of navigation,
they're pinnable and closable, and the open set survives reload.

```
┌─ Job: add-auth-middleware ──────────────── [● running]  1h12m/8h  $1.83/$10 ─┐
│ repo: codeless   branch: codeless/add-auth   runner: claude-code             │
├──────────────────────────────────────────────────────────────────────────────┤
│ [ CHAT ● ] [ SPEC ] [ Stages ◀ ] [ Stage-3: auth ✕ ] [ Stage-1 📌 ✕ ]   [+] │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  (active tab content)                                                        │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Tab kinds

| Tab          | Pinned by default | Closable | What it shows                                           |
|--------------|-------------------|----------|---------------------------------------------------------|
| `CHAT`       | yes               | no       | Job-level chat: planner, supervisor, cross-stage talk.  |
| `SPEC`       | yes               | no       | Job YAML: goal, stages, acceptance, caps, runner.       |
| `Stages`     | yes               | no       | Stage overview with ticks expanded inline (default).    |
| `Stage-<N>`  | no                | yes      | One stage's detail page — gates, log, live chat.        |

User-opened tabs (`Stage-N`) can be **pinned** (📌) to survive page reload
without being explicitly reopened. The `+` menu reopens any closed tab
(closed ≠ destroyed — the stage still exists, it's just not in the bar).

### Tab indicators

The tab label carries a single status glyph so the user can glance at
the bar and see which open surfaces need attention:

| Glyph | Meaning                              |
|-------|--------------------------------------|
| `●`   | running / unread activity            |
| `!`   | failed verify or error               |
| `⟳`   | awaiting review                      |
| `⏸`   | paused                               |
| (none)| idle, completed, or queued           |

`CHAT ●` means "the agent posted something in job chat you haven't read."
`Stage-3 !` means "this stage failed and is open in a tab."

## The `Stages` tab — the overview

This is the default tab. It shows every stage in the job, and within
each stage, every **tick** and **test** as inline children. The user
should be able to scan this list and understand the whole job without
opening a stage tab.

```
STAGES

✓ 1  api                                                       3m   $0.18
     ✓  tick 1   scaffold /users routes
     ✓  test     cargo test --test http::users
     ✓  tick 2   add request validation
     ✓  test     cargo test --lib validators

✓ 2  routes                                                    8m   $0.51
     ✓  tick 1   wire router
     ✓  final    cargo check + cargo test --lib

● 3  auth                                                     14m   $0.42
     ✓  tick 1   add bearer middleware
     ✓  test     cargo test --lib auth
     ✓  tick 2   wire middleware into router
     !  final    cargo test --test http      2 failed   [ restart ▾ ]
                                                         ├─ rerun now
○ 4  tests                                                   └─ new session
○ 5  docs                                                      + handover
```

### Tick rows

Ticks are runner invocations within a stage (per SCOPE: "smallest
re-runnable invocation unit"). The overview surfaces them so the user
can see the agent's *progress within* a stage, not just the stage's
boolean state. Each tick row carries:

- **Status glyph** — `✓` passed, `●` running, `○` queued, `!` failed
- **Kind** — `tick N` (work invocation) or `test` / `final` (verify step)
- **One-line summary** — what the agent set out to do, or what command ran
- **Inline action** — `[ restart ▾ ]` appears on failed rows only

A `test` row is a single verify step. A stage may have many — one
after each tick that asked to verify, plus a `final` gate before the
stage closes. This matches the **layered verify** model
(`verify: Vec<VerifyStep>`) called out in the next-steps section below.

### The `[ restart ▾ ]` menu

Failed rows offer two restart modes, no others:

- **`rerun now`** — re-fire the failed step against the **same warm
  session**. The agent already knows what it tried; this is the
  cheapest fix path. Maps to `--continue <session_id>`.
- **`new session + handover`** — archive the current session, start a
  fresh agent with the stage's `handover.md` as seed context. For
  when the conversation has gone sideways and a clean re-onboard is
  cheaper than continuing.

These two options correspond exactly to the two sides of SCOPE hard
rule #1 (interactive resumption vs. fresh-session reset). The user
picks; codeless does not silently pick for them.

## The `Stage-<N>` tab — detail + live chat

Opening a stage gives you the full picture for that one stage: goal,
gates, log, and a **live chat with the stage's session**.

```
Stage 3: auth                                                  ! final failed

GOAL       Wire bearer-token middleware into the axum router
TICKS      ✓ tick 1   ✓ test   ✓ tick 2   ! final

FAILURE
  test_login_returns_token:  expected 200, got 401
  test_me_requires_auth:     middleware not wired on /me

[ rerun now ]   [ new session + handover ]   [ open review ]   [ stop ]

─── stage chat ────────────────────────────────────────────────────
agent  middleware is registered on the auth subrouter but the parent
       router never .merge()s it — that's why /me bypasses it.
you    can you check if /logout has the same problem?
agent  yes, same root cause. want me to fix both in one patch and
       add a regression test?
you    yes
agent  on it.
       > editing src/router.rs (+4 -1)
       > adding tests/http/middleware_coverage.rs
       > running cargo test --test http ...
───────────────────────────────────────────────────────────────────
[ ask a question or paste a hint... ]                      [ send ]
```

### What "live chat" means

This is the load-bearing UX claim of this doc: **the stage chat is a
real conversation with the same session that just ran**, not a
transcript viewer with a "rerun with feedback" button.

- The user types → the message goes to the stage's still-warm session
  via `--continue <session_id>`.
- The agent replies, may call tools, may edit files, may run verify
  steps. The reply streams into the chat exactly the way live tokens
  stream during an autonomous tick.
- No restart confirmation needed for conversational fixes. Typing
  *is* the rerun.

When the session is warm: the user is **continuing the same agent**.
When the session has timed out (per `session_idle_timeout`, default
30 min): the next user message becomes a `new session + handover`
transparently. The chat keeps working; the first reply is slower
because the new session has to re-onboard.

### What the three buttons are for

- **`rerun now`** — re-fire the failed verify step without typing
  anything. Useful for flaky tests.
- **`new session + handover`** — explicit hard reset. Use when the
  warm session is confused or polluted.
- **`open review`** — escalate this failure to a `Review` row so
  another human (or a future cron-triggered review pass) handles it.
- **`stop`** — terminate the job. Stage stays at `failed`.

Conversational fixes via the chat input are the *normal* path. The
buttons exist for the edge cases where you don't want to talk.

## The `CHAT` tab — job-level

`CHAT` is **not** the same as a stage's chat. It is a higher-level
conversation scoped to the whole job:

- Useful for: questions to the planner before/between stages, asking
  for a stage to be re-planned, asking for a new stage to be inserted,
  retros after the job finishes.
- Backed by: a **separate session** (probably a Rig-helper, since
  this is the planner/supervisor role, not the coding role).
- Not backed by: any individual stage's session.

Two chats, two purposes. The stage chat talks to the agent that did
(or is doing) the work. The job chat talks to the agent that plans
the work.

## The `SPEC` tab

Read-mostly view of the job's YAML — goal, stages with their goals
and acceptance criteria, caps, runner choice. Editable while the job
is queued or paused; read-only while a stage is running. Edits to
acceptance criteria or stage goals propagate to the `Stages` tab on
save.

## State that drives this UI

All values come from existing event types (see SCOPE Repo→Job→Stage
table). Nothing new on the wire for the overview itself:

- **Stage status glyph** — derived from `stage-started`, `verify-passed`,
  `verify-failed`, `stage-completed`, `review-requested`.
- **Tick status glyph** — derived from `task-started`, `task-completed`,
  `task-failed`.
- **Cost / time / tool-call counters** — summed from event payloads
  per SCOPE "Cost — visible and cappable from day one".
- **`● ` unread indicator on tab labels** — driven by a per-tab
  read-cursor stored client-side; advances when the tab is focused.

## What's new on the schema side

Three additions are load-bearing for this UI to render with real
content. None are wire-breaking; all are additive.

### 1. `stage.goal` and `stage.acceptance`

Stages need a one-sentence goal and a list of acceptance bullets to
display in the detail tab. Both authored in the job YAML or produced
by the planner.

```yaml
stages:
  - name: auth
    goal: Wire bearer-token middleware into the axum router
    acceptance:
      - cargo check passes
      - cargo test --lib auth passes
      - cargo test --test http passes
      - cargo clippy -- -D warnings clean
```

### 2. `verify: Vec<VerifyStep>` (layered verify)

Single-string `verify_cmd` becomes a list. Each step is its own gate
row in the UI and emits its own `verify-step-passed` / `verify-step-failed`
event. The stage passes only when every step passes; on first failure
later steps are marked `skipped — prior gate red`.

```yaml
verify:
  - name: check
    run: cargo check
  - name: unit
    run: cargo test --lib
  - name: http
    run: cargo test --test http
  - name: lint
    run: cargo clippy -- -D warnings
```

`verify_cmd: String` stays as a sugar for a single-step list, for
backward compatibility with existing YAML.

### 3. `session_idle_timeout`

Job-level config, default 30 min. After this much idle time on a
warm session, the runtime archives the session and the next user
message becomes a `new session + handover`. Surfaced to the user
only as a footnote ("session was archived after 30 min — your next
message will start a fresh agent with handover context").

## What this UI is **not**

- **Not a file tree or code editor.** The job page is about the
  agent's work, not about manually editing the workspace. That lives
  in the Terax-derived UI surface around it, not in any of these tabs.
- **Not the multi-job dashboard.** Cross-job overview, queue depth,
  global cost burn — different page, different doc.
- **Not a generic terminal.** The stage chat is scoped to the
  stage's session. A "talk to a runner detached from any stage"
  surface, if we ever build one, is separate.

## Implementation order

This is the rough sequence; the granularity track is
[`PROGRESS.md`](./PROGRESS.md) and individual items will get their
own letter-codes there.

1. **Schema additions** (`stage.goal`, `stage.acceptance`,
   `verify: Vec<VerifyStep>`) — small migration, unlocks every UI
   item below.
2. **`Stages` overview tab** with stages + ticks + status glyphs.
   Read-only render of existing events; no new runtime behaviour.
3. **`Stage-<N>` detail tab** with goal / gates / failure summary
   and the three buttons (`rerun now`, `new session + handover`,
   `stop`). No live chat yet — buttons-only.
4. **Live stage chat** — wire the chat input to
   `--continue <session_id>` on a warm session. This is where SCOPE
   hard rule #1's interactive-resumption clause lights up.
5. **`session_idle_timeout`** + the transparent fallback to
   `new session + handover` when the session has been archived.
6. **`CHAT` tab** (job-level, planner-backed). Independent of the
   stage-chat work; lower priority for first usable cut.
7. **`SPEC` tab** edits while queued/paused.

Step 4 is the user-visible payoff. Steps 1-3 are the prerequisites
that make step 4 not feel like magic.
