# JOB-WORKFLOW — turning a Job into an editable, iterable artifact

> Companion to [`JOB-MODEL.md`](./JOB-MODEL.md). JOB-MODEL.md
> establishes the **file contract** (`.codeless/jobs/<name>.yaml`,
> `runs/<name>/handover.md`, `runs/<name>/log.md`). This doc covers
> the **interaction contract** — what the user can edit, what they
> can re-run, what flows feedback from one run to the next. JOB-MODEL
> describes the artifacts; JOB-WORKFLOW describes the loops the user
> closes with them.
>
> The conversational counterpart to this doc is
> [`JOB-CHAT.md`](./JOB-CHAT.md) — one chat thread per Job shared
> across web / Telegram / Slack, plus a per-Run supervisor agent
> that watches the run and replies in that thread. Where this doc
> covers the **iterate loop** (edit spec, re-run, resume), JOB-CHAT
> covers the **conversation loop** (ask the run what it is doing,
> tell it to stop). The two compose: the supervisor's tool surface
> in JOB-CHAT calls the RPCs introduced here
> (`add_job_note`, `pause_after_stage`).
>
> Where this doc disagrees with `JOB-MODEL.md`, **JOB-MODEL wins** —
> raise it as an issue, update both files together.

## The problem

Today the lifecycle of a job in the UI is a fire-and-forget arrow:

```
[ submit ] -> [ runs to terminal ] -> [ done, look at the diff ]
```

The user has no way to:

- Edit the template **after** they realise stage 2 should have been
  worded differently.
- Edit the handover between runs to inject "you were wrong because X".
- Re-run from stage N — only "re-run the whole thing from stage 1".
- Give feedback at re-run time without rewriting the whole prompt.

The result is that codeless feels like "a textbox you submit and watch"
rather than "a workflow you drive." The screenshots in the recent
chat with `ap@nube-io.com` make this exact point: the user is asking
for the **iterate** half of the loop, which doesn't exist yet.

## What "good" looks like

A Job in the UI behaves like a **document with a run history**:

- The **template YAML** is the spec the user keeps refining.
- The **handover** is the inter-session knowledge transfer (already
  half-real today, per JOB-MODEL.md).
- Each **run** is one attempt at the spec. Runs keep their own
  worktree, branch, events, diff.
- The user freely edits the spec / handover between runs, re-runs
  with optional feedback, and resumes from a chosen stage.

Concretely, every job page exposes these affordances:

| Affordance | What it edits | Persists to | Used by |
|---|---|---|---|
| Edit `template.yaml` inline | template spec | `<repo>/.codeless/jobs/<name>.yaml` (committed) | next run |
| Edit `handover.md` inline | hand-off contract | `<worktree>/runs/<job_id>/handover.md` (committed) | next run's prompt-prefix |
| Add ad-hoc note (`runs/<name>/notes/<file>.md`) | free-form context | `<worktree>/runs/<job_id>/notes/` (committed) | next run's prompt-prefix |
| Open YAML / handover / note in the editor tab | same file, full editor | same as above | same as above |
| Re-run | clone the job for a fresh attempt | new Job/Run row | starts another run |
| Re-run **from stage N** | resume mid-template | new Run, frozen-from-stage-N | starts a run skipping prior stages |
| Re-run **with feedback** | the user's "try X this time" | prompt prefix on the next run | model sees the note before stage 1 |

## The data-model question

The above implies a real split that the current schema collapses.
Today:

```
Job (1) ── (N) Stage ── (N) Task
       ── (N) Event
```

`Job` is "the thing the user submitted" **and** "the one attempt"
fused into one row. `Job.template_yaml` is captured once at submit
time. Re-run = `INSERT INTO jobs` with a fresh `id` and starts from
stage 1.

The target shape:

```
Job (1) ── (N) Run (1) ── (N) Stage ── (N) Task
       ── (1) handover.md (mutable)
       ── (1) template.yaml (mutable)
                     │
                     └── Run also carries (N) Event
```

Where:

- `Job` is the **template instance**: a name, a repo, a goal, a
  current template, a current handover, a list of runs. Long-lived.
- `Run` is **one attempt**: started_at / ended_at / cost / branch /
  worktree / stop_reason / a **frozen-at-submit-time** template
  snapshot / a "resumed_from_stage" pointer / a terminal status.
  Multiple per Job. Each Run owns its events, stages, tasks, reviews.
- The mutable `handover.md` and `template.yaml` live on the **Job**,
  not the Run. Each new Run reads the current versions; the Run's
  frozen snapshot tells the future "what did this run actually see?"

This is a real schema change. SQLite migration. Wire types change.
The `submit_job` / `rerun_job` / `get_job` / `list_jobs` RPCs all
gain a notion of `Run` alongside `Job`.

## Recommended sequencing — (A) first, then (B)

The full Job/Run split is correct but expensive. Several decisions
inside it are easier to get right with concrete user behaviour in
front of us. So:

### (A) Half-step — edit + iterate without a schema change

Scope (one focused session of work):

1. **`update_job_template` RPC.** Writes `<repo>/.codeless/jobs/<name>.yaml`
   (creates `.codeless/jobs/` if missing), commits with message
   `update template: <name>`. The Job row's `template_yaml` column
   stays as the historical record of what was submitted; the
   committed file is the current spec the next run will read.

2. **`update_job_handover` RPC.** Writes
   `<worktree>/runs/<job_id>/handover.md`, commits with message
   `update handover: <name>`. Next run's `find_latest_handover`
   picks it up via the existing pickup path (commit b67f111).

3. **`add_job_note` RPC.** Writes `<worktree>/runs/<job_id>/notes/<filename>.md`,
   commits. The orchestrator's prompt-prefix builder concatenates
   every note in `notes/` after the handover when the next run
   starts. "Drop a markdown file with what to fix" becomes a
   first-class flow.

4. **UI: inline editors** for `template.yaml` and `handover.md`.
   CodeMirror with YAML / markdown highlighting in the existing
   "Template" and "Handover" panes. `[edit]` toggle, save button,
   discard-changes button.

5. **UI: "open in editor tab" buttons** on every editable surface.
   Inline edit = convenient; editor tab = powerful (autocomplete,
   multi-cursor, the whole point of CodeMirror).

6. **UI: re-run dialog with a feedback textarea.** The text the user
   types is written to a new `notes/<timestamp>-feedback.md` before
   the new run starts, so the orchestrator picks it up via (3).
   "Try X this time" without rewriting the prompt.

7. **UI: notes panel.** New section on the job page lists every
   `notes/*.md` file with click-to-edit. The user can drop ad-hoc
   context any time and watch it accumulate.

What (A) does **not** give us:

- "Re-run from stage 3" — a fresh Job still starts at stage 1.
  The user can simulate it by editing the template to start at
  what was stage 3, but that loses the original spec.
- A clean "run history" surface — re-runs are sibling Job rows
  that happen to share a name.
- Per-run worktree retention policy. A new Job means a new worktree;
  the prior runs' worktrees stay on disk per ux-1 but they're not
  navigable as a list under the Job.

What (A) gives us **for free** that (B) won't:

- No migration. No wire-type churn. No risk of getting the schema
  wrong on the first cut.
- Fast feedback on which loops the user actually uses. If "edit
  template + re-run" is 90% of the value, (B)'s extra surface area
  is over-engineering.

### (B) Full step — split Job and Run

Once (A) has been in your hands for a few weeks and you've felt the
specific friction of "every re-run is a new Job", commit to the
split. The shape, in concrete schema terms:

```sql
-- mutable surface; one per "thing the user is building"
CREATE TABLE jobs (
    id            TEXT PRIMARY KEY,    -- ULID
    repo_id       TEXT NOT NULL,
    name          TEXT NOT NULL,        -- the YAML's `name:`
    template_yaml TEXT,                 -- current spec; mutable
    handover_md   TEXT,                 -- current handover; mutable
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE (repo_id, name)
);

-- immutable per-attempt record
CREATE TABLE runs (
    id                  TEXT PRIMARY KEY, -- ULID
    job_id              TEXT NOT NULL REFERENCES jobs(id),
    ordinal             INTEGER NOT NULL, -- 1-based
    template_snapshot   TEXT NOT NULL,    -- frozen at submit time
    handover_snapshot   TEXT,             -- the handover the run started with
    runner              TEXT NOT NULL,
    branch              TEXT NOT NULL,    -- codeless/<name>-r<ordinal>
    worktree_path       TEXT,
    status              TEXT NOT NULL,
    stop_reason         TEXT,
    started_at          INTEGER,
    ended_at            INTEGER,
    cost_cap_cents      INTEGER NOT NULL,
    wall_clock_cap_ms   INTEGER NOT NULL,
    cost_cents          INTEGER NOT NULL DEFAULT 0,
    resumed_from_stage  TEXT,              -- StageId of the prior run's
                                           -- stage we picked up from, or NULL
    created_at          INTEGER NOT NULL,
    UNIQUE (job_id, ordinal)
);

-- Existing stages / events / tasks / reviews keyed by run_id instead
-- of job_id. The migration rewrites every row with a synthetic
-- ordinal=1 run for the old Job's data.
```

Wire-type implications (`codeless-types`):

- New `Run` struct mirrors the table.
- `Job` shrinks: no more `runner`, `branch`, `worktree_path`,
  `cost_cents`, `started_at`, `ended_at`, `cost_cap_cents`,
  `wall_clock_cap_ms`, `template_yaml`. Those become fields on
  `Run`. `Job` keeps `template_yaml` and `handover_md` as the
  **mutable current** versions.
- `submit_job` returns a `Run`, not a `Job` (or both — the Job is
  created or reused, and a fresh Run is returned).
- New RPCs: `rerun_job(job_id, resume_from?: StageId)`,
  `update_job_template(job_id, yaml)`, `update_job_handover(job_id, md)`,
  `list_runs(job_id)`, `get_run(run_id)`.
- `Event::JobCompleted` etc. become `Event::RunCompleted`. The
  envelope's `job_id` becomes `run_id`. Migration of the events
  table needs a script that fills `run_id` from each event's
  `job_id` resolved through the synthetic-ordinal-1 row.

UI implications:

- The **job page** becomes a header (template + handover + notes —
  the mutable surface) plus a list of **runs**. Clicking a run opens
  the run-detail surface (today's job-detail page, basically).
- The "re-run" button gets richer: from-stage picker, feedback
  textarea, runner picker.
- Cost / elapsed roll-ups: the Job page shows lifetime totals across
  all runs; each Run page shows per-run.

Open questions for (B), to revisit when we get there:

- What does "re-run from stage 3" mean if the template changed
  between Run 1 and Run 2? Two valid answers: (i) freeze the
  template at the moment of re-run (current proposal — `Run.template_snapshot`),
  (ii) always run the current template's stage 3 onward. Pick (i)
  for predictability.
- Worktree retention across runs: keep N most recent, garbage
  collect older? Or keep all forever until the user GCs?
- If a REVIEW stage in Run 1 was approved, and Run 2 resumes from
  the stage after it, do we carry the approval forward? Probably
  yes — the review approved the **work** at that stage, and re-doing
  earlier stages does not re-do the review.

## Editing surfaces — UX details

These apply equally to (A) and (B).

**The template pane** (`Template` in the sub-rail):

- Default view: render the YAML as a syntax-highlit read-only block
  (today's behaviour).
- `[edit]` toggle → switch to a CodeMirror editor with YAML mode.
- `[save]` → POST to `update_job_template`. The save commits the
  file in the source repo with `update template: <name>` so the
  diff is visible.
- `[open in editor tab]` → opens
  `<repo>/.codeless/jobs/<name>.yaml` in a regular editor tab. The
  user gets autocomplete, multi-cursor, and saves through the same
  `fs_write_file` path the editor already uses.
- Inline edit and editor-tab edit are **the same file on disk**.
  The inline editor reloads on focus to pick up out-of-band changes.

**The handover pane** (`Handover` in the sub-rail):

- Default view: rendered four-section structured view (today).
- `[edit]` toggle → CodeMirror, markdown mode. Same shape as template.
- Save commits the file with `update handover: <name>`.
- The model's `Done / Next / What you need to know / Open questions`
  sections stay visible while editing — they are headings in the
  source, not a separate format.

**The notes pane** (`Notes`, new section in the sub-rail):

- Lists files in `<worktree>/runs/<job_id>/notes/`.
- `[+ note]` button → opens a new file with a default name like
  `feedback-<timestamp>.md` and a placeholder body.
- Notes are markdown; the convention is "one note per topic". The
  orchestrator concatenates them all into the next run's prompt
  prefix, ordered by filename, after the handover.

**The runs pane** (B-only):

- List of `Run` rows for this Job. Status pill, ordinal,
  started/ended timestamps, branch, cost, "resumed from" link if
  applicable.
- Click a run → opens the per-run detail (today's job-detail page).
- `[+ new run]` → re-run dialog.

**The re-run dialog**:

- Optional `resume from` dropdown (B-only): pick a stage from the
  most recent run; the new run skips the prior stages.
- Optional `feedback` textarea: what to do differently. Saved as a
  note (A) or as the new run's seed prompt (B).
- `[run]` queues the new run; on success, navigates to the new
  run's page.

## How feedback flows through the prompt assembler

This is the same path the existing handover-pickup uses, extended:

```
[ stage prompt ] is built from, in order:
  1. # Prior session handover
       <— current handover.md content
  2. # Notes from the user
       <— concatenated notes/*.md, ordered by filename
       <— each note prefixed with its filename so the model can
          tell what comes from where
  3. # Job goal
       <— template's `goal:` field
  4. # Stage N of M
       <— the stage title from the template
  5. # What to do now
       <— the orchestrator's instruction to commit and stop
```

Today (1) exists (commit b67f111). (A) adds (2) and the "edit
handover" path that (1) reads from. (3)/(4)/(5) already work.

For re-run-from-stage-N (B-only), the orchestrator skips the first
N-1 stages and starts at N. Stage prompts after the resume point
include both the prior run's handover AND any new notes — the model
needs both the "what landed last time" context AND the "what's
different this time" instruction.

## Migration plan, if/when we commit to (B)

1. **Add `runs` table** in a new migration. Backfill: every existing
   `Job` row gets a synthetic Run with `ordinal=1`, copying
   `runner` / `branch` / `worktree_path` / etc.
2. **Add `run_id` columns** to `events`, `stages`, `tasks`,
   `reviews`. Backfill from each row's `job_id` through the
   synthetic Run.
3. **Drop migrated columns from `jobs`** in a follow-up migration.
   Keep `template_yaml` and `handover_md` on `jobs` (the mutable
   versions), drop the rest.
4. **Wire types**: ship the new `Run` struct, shrink `Job`, regen
   the TS bundle. Bump the wire `EventCursor` semantics if needed —
   probably not, the cursor is a stream-position not a row-id.
5. **RPC surface**: add `list_runs`, `get_run`, `update_job_template`,
   `update_job_handover`; rewrite `submit_job` to return both Job
   and Run; rewrite `rerun_job` to take `(job_id, resume_from?)`.
6. **UI**: split the current job page into the Job header (mutable
   surfaces + runs list) and the Run page (today's detail layout).

This migration is not reversible in practice — the wire-type
churn means clients pinned to the old shape will fail. Accept that
or design a compatibility shim that translates new envelopes to old
shapes; not worth it for a single-user MVP.

## What lands in code first (A's full punch list)

For each item: where the change lives, what the wire impact is,
roughly how big.

| # | Change | Crate / module | Wire impact | Size |
|---|---|---|---|---|
| 1 | `update_job_template` RPC | `codeless-rpc` + `codeless-runtime/rpc.rs` | new method, args/result types | S |
| 2 | `update_job_handover` RPC | same | new method | S |
| 3 | `add_job_note` RPC | same | new method | S |
| 4 | Notes accumulator in `TemplateRunner` prompt builder | `codeless-runtime/template_runner.rs` | none | S |
| 5 | UI: inline YAML editor in Template pane | `codeless-ui/modules/jobs/JobPage.tsx` (+ a new `TemplateEditor.tsx`) | none | M |
| 6 | UI: inline markdown editor in Handover pane | same | none | M |
| 7 | UI: notes pane + new-note dialog | new `NotesPane.tsx` | none | M |
| 8 | UI: re-run dialog with feedback textarea | replace today's bare re-run button | none | S |
| 9 | UI: "open in editor tab" buttons everywhere | `JobPage.tsx`, `TemplateEditor.tsx`, etc. | none | S |
| 10 | DEMO-UI section on the iterate loop | `DEMO-UI.md` | none | S |

Estimate: one focused session. The CodeMirror integration is the
chunkiest piece, but the editor module is already wired for general
file editing — we just instantiate it in-place inside the job page.

## What lands later (B's punch list)

Listed for completeness; do not start until (A) has been in real
use and we know which (B) decisions to firm up.

- Schema migration: `runs` table, backfill, foreign keys.
- Wire types: `Run`, shrink `Job`, regen TS.
- RPC: `list_runs`, `get_run`, `submit_job` returns Run, `rerun_job(job_id, resume_from?)`.
- Orchestrator: `resumed_from_stage` support in `TemplateRunner`.
- UI: Job page = template + handover + notes + runs list; Run page = today's layout; re-run dialog with from-stage picker.
- Worktree retention policy.
- Review carry-forward when resuming past an already-approved stage.

## Decision: how the user signals "I want to edit"

A small thing that's worth picking up-front so we don't accidentally
build two UX patterns:

**Option 1:** every editable surface has an `[edit]` toggle that
swaps the rendered view for an inline editor. Save / discard buttons
explicitly persist. **The default state is read-only.**

**Option 2:** every editable surface is **always** an editor, with
the rendered view hidden behind a `[preview]` toggle. Save is
debounced on blur.

Pick option 1. The default state is "read what's there"; editing is
opt-in. Matches user expectations from GitHub / Linear / every
JIRA-style ticket system. Saves us the debounce-on-blur edge cases.

## Decision: do we commit user edits?

When the user saves an edit to the template / handover / a note, do
we `git commit` it in the source repo, or just write the file?

**Commit it.** Reasons:

- The audit trail matters. JOB-MODEL.md is explicit that the
  inter-session contract is **committed**; an uncommitted handover
  is by definition outside the contract.
- `git diff` is the user's other tool for understanding what
  changed. Uncommitted edits don't show up there.
- Reverting is `git revert`; the user gets that for free.

Commit-on-save is what the (A) plan above assumes. The downside is
small noise in `git log`; the upside is a real history of how the
spec evolved.

## Job chaining — the "next job after this one" loop

Everything above is about iterating **one** Job better. This section
covers the orthogonal problem: stringing **multiple** Jobs together
so finishing job-1 triggers job-2 with no human in the loop.

These are distinct concepts that happen to share the word "workflow."
To keep the codebase legible, this doc fixes the names now:

| Term | Meaning |
|---|---|
| **Job** / **Run** | One spec, many attempts. The subject of the sections above. |
| **Plan** | An ordered sequence (later: DAG) of Jobs with transition rules. The subject of this section. |

"Plan" beats "Workflow" / "Pipeline" / "Chain" because (i) it matches
the user's mental model ("planner"), (ii) it doesn't collide with
JOB-WORKFLOW's existing usage, and (iii) it composes naturally with
the scheduler ("on Monday 8am, run Plan X").

### The problem

Today the user can submit a Job and watch it run. They cannot say:

- "When job-1 finishes successfully, start job-2."
- "When job-1 fails, start job-recover-from-failure."
- "Every Monday at 08:00, run the release-prep Plan: lint → test →
  changelog → publish."
- "Run job-A and job-B in parallel; when both finish, run job-C."

The cheap workaround is a tool the LLM calls inside job-1 (e.g.
`codeless.job.then`). That works until job-1 dies before reaching
the call, or the user wants to see the planned chain *before* it
runs. Chains-as-tool-calls are scattered across prompts; chains-as-
data are introspectable, persistent, and reusable.

### What "good" looks like

A Plan is a **document with a run history**, mirroring how Job will
become one under (B):

- The **Plan spec** is what the user keeps refining (a YAML/JSON
  document listing steps and transitions).
- Each **PlanRun** is one execution of the spec — which Jobs were
  spawned, in what order, with what outcome.
- A Plan can be triggered three ways: manually from the UI / CLI,
  by the scheduler ([`codeless-tools::schedule`](../crates/codeless-tools/src/schedule/)),
  or by another Plan finishing.

```
Plan (1) ── (N) PlanStep
        ── (N) PlanRun (1) ── (N) PlanRunStep ── (1) Job (1) ── (N) Run
```

`PlanStep` is the *template* ("after step-1 succeeds, run a Job using
this template, in this repo, with this policy"). `PlanRunStep` is the
*attempt* (which Job/Run actually got spawned, what its terminal
state was). The shape intentionally rhymes with Job/Run from (B):
mutable spec vs. immutable execution record.

### Integration with what already exists

The engine is small because three of the four pieces are already
built:

| Piece | Where it lives | What Plans use it for |
|---|---|---|
| Job state-machine terminal events (`JobFinished` / `JobFailed` / `JobStopped`) | `codeless-runtime` event bus | The Plan engine subscribes; each terminal event triggers the next `PlanStep` (or marks the PlanRun done). |
| `codeless-bot-core::outbound` pattern | `codeless-bot-core` | The same `EventSource` abstraction the bots use — Plan engine is a second consumer, no new bus. |
| `codeless-tools::schedule::Scheduler` | `codeless-tools/src/schedule/` | `Schedule` fires → `Action` calls `start_plan_run(plan_id)`. Recurring chains become a one-liner. |
| SQLite as the source of truth | `codeless-runtime` migrations | `plans`, `plan_steps`, `plan_runs`, `plan_run_steps` tables; same migration discipline as `runs` in (B). |

The new code is: the `plan_engine` module that owns the state machine
("PlanRun X is on step 3; step 3's Job just finished successfully;
look up step 3's `on_success` transition; spawn step 4's Job; record
the new `PlanRunStep`"), the four tables, and the RPC / UI surface.

### Minimal transition vocabulary

Resist the urge to ship a full DAG with `when:` predicates on day
one. The 80% case is a linear chain with two branches per step:

```yaml
name: release-prep
steps:
  - id: lint
    job_template: lint
    on_success: test
    on_failure: stop

  - id: test
    job_template: test
    on_success: changelog
    on_failure: notify-and-stop

  - id: changelog
    job_template: changelog
    on_success: publish

  - id: publish
    job_template: publish

  - id: notify-and-stop
    job_template: notify-failure
    on_success: stop
```

Three transition targets: a step id, `stop` (terminate PlanRun
successfully), or omitted (= same as `stop`). `on_failure` defaults
to `stop`, so the common case stays terse. Parallel (`fan_out:`) and
join (`fan_in:`) ship in a follow-up once the linear case has been
in real use.

### Recommended sequencing — (P1) → (P2) → (P3)

Same phased discipline as (A) → (B) above. Each phase ends with
something a user can drive end-to-end.

**(P1) — Reusable Plan library + tool, no UI.** ~3 days. **Landed.**

Per-bullet, the modules that actually shipped:

- [x] Pure data — [`codeless-tools/src/plan/spec.rs`](../crates/codeless-tools/src/plan/spec.rs).
  `PlanSpec`, `PlanStep`, `StepId`, `Transition`, `PlanSpecError`;
  serde + validation (unique step ids, every transition target
  exists or is `stop`). Mirrors the `email/` and `schedule/` layout.
- [x] In-memory engine — [`codeless-tools/src/plan/engine.rs`](../crates/codeless-tools/src/plan/engine.rs).
  `PlanEngine` + injected `JobSpawner` trait; one PlanRun = one
  state machine that on each terminal `Event::JobCompleted` /
  `JobFailed` / `JobStopped` looks up the current step's transition
  and either spawns the next Job or marks the run done. No SQLite,
  no tokio runtime handle — the engine is a function of "the event
  bus" + "a spawn callback."
- [x] Schedule → Plan composition — [`codeless-tools/src/plan/dispatch.rs`](../crates/codeless-tools/src/plan/dispatch.rs).
  `StartPlanRunAction` is a `codeless-tools::schedule::Action` that
  calls `PlanEngine::start_run(plan_id)` when a `Schedule` fires
  with the `start-plan-run` payload kind (`START_PLAN_RUN_KIND`).
  That one file is the boundary proof — recurring chains are a
  one-liner per the table above.
- [x] LLM-callable tool surface — [`codeless-tools/src/tools/plan_tool.rs`](../crates/codeless-tools/src/tools/plan_tool.rs).
  Four `Tool` impls registered in
  [`codeless-tools/src/tools/mod.rs`](../crates/codeless-tools/src/tools/mod.rs):
  - `codeless.plan.create` — register a `PlanSpec`, returns `plan_id`.
  - `codeless.plan.start` — start a `PlanRun` from a registered plan.
  - `codeless.plan.list` — snapshot of plans and in-flight / terminal runs.
  - `codeless.plan.cancel` — mark an in-flight run cancelled.
- [x] Boot wiring — [`codeless-mcp/src/main.rs`](../crates/codeless-mcp/src/main.rs).
  Single `Arc<PlanEngine>` constructed once at MCP startup;
  registered with the tool registry and with the scheduler's
  `PayloadDispatcher` under `START_PLAN_RUN_KIND`. The engine is
  not wired into `codeless-runtime`'s event bus yet — P1 ships it
  inside the MCP process only; the runtime-side `EventSource`
  subscription comes in P2 alongside persistence.

Known limits, surfaced here so P2 doesn't forget them:

- **In-memory only.** `PlanEngine` holds plans and runs in
  `HashMap`s behind a `Mutex`. There is no SQLite, no migrations,
  no `template_snapshot`. Restarting the MCP / runtime process
  drops every registered plan and every in-flight `PlanRun`. P2 is
  where that becomes durable.
- **MCP-process scope.** The engine constructed in
  `codeless-mcp/src/main.rs` is independent from any engine
  `codeless-server` might construct; there is no shared state
  across processes. P1's job is to prove the boundary, not to
  share it.
- **Linear chains only.** Transitions are `on_success` / `on_failure`
  → step id or `stop`. `fan_out:` / `fan_in:` / `when:` predicates
  are deferred to P3.

**(P2) — Persistence + RPC surface.** ~3 days.

- Migrations: `plans`, `plan_steps`, `plan_runs`, `plan_run_steps`.
  PlanRun rows carry a `template_snapshot` of the spec at start
  time, same reason JOB-WORKFLOW Run carries one — re-runs and
  retries should not silently behave differently because the spec
  changed.
- RPCs: `create_plan`, `update_plan`, `start_plan_run`,
  `cancel_plan_run`, `list_plans`, `list_plan_runs`, `get_plan_run`.
- Event bus emits `PlanRunStarted` / `PlanRunStepStarted` /
  `PlanRunStepFinished` / `PlanRunFinished` so the existing bots and
  the UI can subscribe with no new transport.
- Restart recovery: on boot, scan for PlanRuns in `running` state,
  resubscribe to the in-flight Job's events, resume the state
  machine.

**(P3) — UI + DAG.** ~1 week.

- Plan page: spec editor (same CodeMirror pattern as JOB-WORKFLOW),
  list of PlanRuns, manual `[run]` button.
- PlanRun page: visual graph of steps with live status pills,
  click-through to the Job/Run that step spawned.
- DAG primitives: `fan_out: [step-a, step-b]`, `fan_in:` waits on
  named predecessors. Predicate transitions (`when: outputs.x > 0`)
  if the data is there to support them — defer if not.

### What stays out of scope (deliberately)

- **Cross-repo Plans.** A PlanStep's Job runs in the repo the step
  names. The engine doesn't try to coordinate worktrees across
  repos — that's `mani`'s job, and a Plan can call `mani` through
  the existing shell tool if it needs to.
- **Conditional re-runs of the same step.** A Plan does not loop
  back to an earlier step. Retries are a per-Job affair, owned by
  JOB-WORKFLOW's re-run flow.
- **Distributed execution.** Single-tenant MVP per R5; the engine
  runs in the same process as the runtime.
- **Plan-of-Plans.** A PlanStep spawns a Job, not a Plan. If you
  want a sub-Plan, expose a "start this Plan" action via the tool
  surface and have a step call it — keeps the engine's vocabulary
  one-level.

### Naming inside the code

To make the JOB-WORKFLOW vs. Plan split obvious from filenames
alone:

- `codeless-runtime/src/plan/` — engine, state machine, persistence.
- `codeless-tools/src/plan/` — pure data + tool wrappers.
- Wire types: `Plan`, `PlanStep`, `PlanRun`, `PlanRunStep` — never
  `Workflow*`.
- Event variants: `PlanRunStarted`, never `WorkflowStarted`.

`Job` / `Run` / `Stage` / `Task` remain JOB-MODEL / JOB-WORKFLOW's
vocabulary, untouched.

### Open questions for Plan, to revisit at (P2)

1. **What does cancelling a PlanRun do to the in-flight Job?**
   Probably stop it — the Plan is the user's intent envelope, and
   cancelling the envelope cancels the work. Confirm at (P2).
2. **What does editing a Plan spec do to running PlanRuns?** Same
   answer as JOB-WORKFLOW's question 5: nothing. Running PlanRuns
   have their `template_snapshot`; edits apply to the next run.
3. **How does a Plan surface its history in chat (Slack / Telegram)?**
   The bot adapters already render Job terminal events; a Plan
   probably renders as a single thread with one message per
   PlanRunStep transition. Belongs in `codeless-bot-core::notify`,
   not the engine. The per-Job chat substrate this rides on is
   specified in [`JOB-CHAT.md`](./JOB-CHAT.md); a Plan-level chat
   surface is deferred until Plans get UI (P3).
4. **What's the failure-cascade default?** A step that fails with no
   `on_failure` falls through to `stop`. The PlanRun's terminal
   status is `failed`, not `success`. The handler-step pattern
   (`notify-and-stop` above) is the explicit recovery path.

## Open issues / non-decisions

Things I don't have a confident answer for, listed so they don't get
quietly settled by the first PR:

1. **What does "discard changes" mean for the inline editor?**
   Revert the inline buffer to the last-saved version (= last
   committed). Don't `git restore` — the user might have edited
   the file out-of-band in another editor tab.
2. **What happens if the inline editor and the editor-tab editor
   disagree?** The inline editor reloads on focus. The editor tab
   reloads when its file is touched on disk. Last write wins; the
   user gets a "this file changed on disk, reload?" prompt either
   way. We already have this for the editor tab; mirror it in the
   inline editor.
3. **Are notes auto-applied or opt-in?** Auto. The model needs the
   full context. If a note becomes stale, the user deletes the file.
4. **Where do notes live for ad-hoc (non-template) jobs?** Probably
   nowhere — ad-hoc jobs are by definition one-shot. The Notes pane
   only shows up when `template_yaml` is set.
5. **(B-only) When a Run is mid-flight and the user edits the
   template, does the running Run see it?** No — the running Run
   has its own `template_snapshot`. Edits apply to the **next** Run.
   Make this visible in the UI: "your edits will apply on the next
   run."

## TODO — user-initiated pause at a stage or todo boundary

> Status: **not designed yet**. This section captures the shape of
> the problem so the next agent picking it up doesn't start from a
> blank page. Do not start work here until the (A) punch list has
> landed — pausing is only valuable once the iterate loop exists,
> because pause-without-edit is just "wait."

### The problem

Today the user has two coarse controls: let the run finish, or
cancel it. There is no in-between. Specifically the user cannot:

- "Stop after the current stage commits — I want to look at the
  diff before stage N+1 starts."
- "Stop after the current todo finishes — I want to read
  `handover.md` before the agent moves to the next todo in the
  same stage."
- "I'm watching the closing trio tick over; pause after `checks`
  so I can run the test suite myself before `docs` rewrites the
  handover."

REVIEW gates cover the **planned** boundaries — the ones the
template author knew were risky. The pause control covers the
**ad-hoc** boundaries — the ones the user only realises matter
once they see the run in flight.

### What "good" looks like

Two pause modes, both **soft** (the runner finishes the in-flight
unit of work before halting; no killing mid-edit):

| Mode | Boundary the runner halts at | UI affordance |
|---|---|---|
| `pause-after-stage` | After the current stage's `git` closing-trio commit + push | `[pause after stage]` button on the run page |
| `pause-after-todo` | After the current todo's commit (or, for non-committing todos, after the runtime records the todo as `[x]`) | `[pause after todo]` button on the active stage card |

Both surface as a sticky banner on the run page once armed:
"Will pause after `<stage|todo>` finishes." The user can
disarm before the boundary fires (the button becomes
`[cancel pause]`). Once the boundary fires, the run enters a new
terminal-ish status — `paused` — that is **resumable** without
restarting the worktree.

The two-mode split matters because stages are slow (minutes) and
todos are fast (seconds-to-a-minute). `pause-after-todo` is the
fine-grained control that makes "watching the closing trio" a
real loop; `pause-after-stage` is the coarse one that lets the
user gate every stage transition by hand, effectively turning every
stage into an ad-hoc REVIEW.

### Resume semantics

Resume is **not** a re-run. A resumed run:

- Keeps the same worktree, branch, run id, event stream, cost
  meter, wall-clock meter. The pause window does **not** count
  against the wall-clock cap.
- Reads `handover.md` and any `notes/*.md` the user added during
  the pause — this is where the iterate loop ((A)) composes with
  pause. "Pause, edit handover, drop a feedback note, resume" is
  the canonical use.
- Re-enters the runner at the next unit of work after the boundary
  it halted at (next stage for `pause-after-stage`; next todo in
  the same stage for `pause-after-todo`).
- Cannot resume **across** a process restart in P1 — pausing
  requires the runtime to hold the runner's in-memory state. P2
  parity with the persistent run table (JOB-WORKFLOW (B)) is what
  makes restart-survivable pause possible.

A resumed run that hits a pause boundary again pauses again.
Multiple consecutive pauses are explicitly supported — the user
might pause, look, resume, look more, pause again.

### Composition with REVIEW gates

REVIEW gates and pauses are different mechanisms but visually
indistinguishable to the user. The unified status should be:

- `paused` — runner is halted at a boundary, awaiting either
  user resume (pause) or user approval (REVIEW). The reason
  (`pause-after-stage`, `pause-after-todo`, `review-gate`) is the
  detail; the surface action (a `[resume]` / `[approve]` button) is
  the same.
- `paused (with edits)` — there are uncommitted edits in the
  worktree from the user (template / handover / notes). Resume
  commits them as a `pause-edit:` commit before re-entering the
  runner, so the next stage starts with a clean tree and the
  edits are visible in `git log`.

### Open questions for this TODO

1. **Mid-stage pause** — is there a third mode that halts the
   runner *inside* a stage, before the next todo starts, by
   sending a signal mid-`Tool::call`? Probably no for V1: tool
   calls are atomic units in the runner's mental model, and
   pausing inside one risks half-applied edits the agent can't
   reason about on resume. Halt at todo boundaries only.
2. **Pause vs. cancel** — what happens if the user clicks `cancel`
   on a paused run? Same as cancelling a running run: the runner
   exits, the worktree stays, the run's terminal status becomes
   `cancelled`. The pause was just a deferral, not a state
   change.
3. **Pause and the cost/wall-clock caps** — the wall-clock cap
   pauses with the runner (does not count). The cost cap is
   already-spent money; it carries through unchanged. If the user
   pauses to add notes and then runs into the cap on resume, the
   runner stops with `cost-cap` like any other run.
4. **Concurrent pauses across jobs** — pause is per-run; multiple
   runs can be paused at once. The runtime already supports
   multiple in-flight runs (concurrency cap), so this falls out
   for free.
5. **Telegram / Slack pause control** — should the bot adapters
   expose a `/pause <job>` command? Probably yes once the in-
   product UI is right, but design that *after* the UI affordances
   ship — bot commands tend to ossify the surface they wrap.
6. **What "boundary" means for the closing trio specifically** —
   `pause-after-todo` while the runner is between `checks` and
   `docs` is the high-value case. Confirm the runtime emits a
   todo-completed event *before* dequeueing the next todo, so the
   pause check has somewhere to fire.

### Sequencing — where this lands in the (A) → (B) plan

Sequence as **(A.5)**, between (A) and (B):

- (A) ships the edit + iterate loop. Pause is useless without it
  (the user has nothing to do during the pause).
- (A.5) ships pause-after-stage. One new RPC (`pause_run`), one
  new run status, one new UI button, one `pause-edit:` commit
  path. Pause-after-stage only — pause-after-todo waits for B's
  per-todo event surface.
- (B) ships the Job/Run split + the per-todo event surface
  pause-after-todo needs.
- (A.5b), after (B): pause-after-todo + the bot-command surface.

This sequencing means **do not** wire pause into the runner before
(A) — the temptation to ship `pause` as a sibling of `cancel` is
real, but a pause control with no edit affordances is a worse UX
than no pause control at all (the user pauses, has nothing to do,
resumes, and is annoyed).

### What does *not* belong in this TODO

- Auto-pause heuristics ("pause when cost > N", "pause when a
  test fails"). Those are policy on top of the mechanism; design
  the mechanism first, layer policy in a separate doc.
- A queue of pre-armed pauses ("pause after stage 3 *and* stage
  5"). One armed pause at a time; if the user wants multiple,
  they re-arm after each resume. Keeps the UI honest.
- Pause across Plans. A PlanRun pauses by pausing its current
  step's Run — no first-class "pause Plan" yet. Revisit once
  Plans have UI (P3).

## TODO — precheck rules reference

> Status: **rules exist in code, not in this doc**. Today the
> precheck that runs at REVIEW gates auto-fails handovers for
> reasons the template author cannot see anywhere in JOB-WORKFLOW,
> JOB-MODEL, or JOB-DIR. The next agent debugging an auto-fail
> ends up reading the runtime source to find out why. Document
> the rules here so the contract is visible to the people writing
> against it.

### Why this matters

The handover contract today is described as a **format** — the
four sections (`Done` / `Next` / `What you need to know` /
`Open questions`). The precheck enforces a **semantics** layer on
top of that format, and the gap between the two is where surprise
auto-fails come from. The fix is not to weaken the precheck — it
is to make the rules first-class.

### Rules to write up (best understood from observed behaviour)

At minimum, this section needs:

1. **`Done` ↔ diff cross-check.** Every path-shaped token under
   `Done` must appear in the stage's git diff. The rule exists to
   stop the agent from claiming work it didn't do. A survey stage
   that reads `crates/codeless-tools/src/schedule/` and lists it
   under `Done` auto-fails — those paths are not in the diff
   because nothing was written there.
2. **Section presence.** All four sections must be present, in
   order, with the exact headings. An empty section is fine; a
   missing one is a fail.
3. **`Done` for read-only stages.** Stages that produced zero
   source diff (survey, design, REVIEW-prep) put **the docs they
   wrote** under `Done`, and **the things they read** under
   `What you need to know`. The closing-trio `git` todo for these
   stages is `committed handover.md only` — not `skipped — no
   diff`, because the handover itself is the diff.
4. Other rules the precheck enforces today that aren't in this
   list yet — the next agent writing this section should grep the
   runtime for the precheck implementation and lift the full set,
   then come back and update this list.

### What lands in code

Nothing new — this is a docs-only TODO. The runtime already
enforces these rules. The deliverable is a `### Precheck rules`
subsection inside the existing "How feedback flows through the
prompt assembler" section (or a sibling section if it grows past
~30 lines), with each rule numbered so a precheck failure can
reference `precheck rule #1` and the user / agent can look it up.

### Out of scope for this TODO

- Changing what the precheck enforces. The semantics are right;
  only the documentation is wrong.
- Per-rule failure messages in the UI. Worth doing but separate —
  a precheck failure should link to the rule's anchor in this
  doc; the wording lives in the runtime.

## TODO — handover schema for read-only stages

> Status: **convention exists by accident, not by design**. Survey,
> investigation, and design stages produce no source diff but
> still have to satisfy the `Done` / `Next` / `Know` /
> `Open questions` contract. The current shape of `Done` ("things
> I produced") doesn't fit them naturally, which is how the
> auto-fail above happens.

### The shape to fix

For stages whose deliverable **is** the handover:

- `Done` lists the handover sections written and the commit that
  landed them. **No source paths.** If the stage wrote a long
  worktree-root `handover.md` alongside the runtime-managed run
  handover, list both.
- `What you need to know` carries the actual content — what was
  read, what was decided, what the next stage must not re-derive.
  This is the part the next stage's prompt prefix is built from,
  so it has to be where the substance lives.
- `Next` names the next stage's first concrete unit of work
  (file path + what to do), not "stage 2 should start." The
  next-stage prompt is generic enough that "start on
  `crates/x/src/y.rs` adding `Foo`" is more useful than the
  stage title repeated.
- The `git` closing-trio item is `committed handover.md only`
  with the commit short SHA. The current "skipped — no diff"
  wording is wrong for these stages because there *is* a diff —
  the handover.

### Sequencing

Land this **at the same time** as the precheck-rules TODO above —
the two reinforce each other. The precheck rules describe what
fails; the handover-schema convention describes the shape that
won't trip them.

## TODO — commit message conventions

> Status: **vocabulary exists but is undocumented**. Today the
> codebase has at least these commit prefixes:
> `stage N: ...`, `update template: ...`, `update handover: ...`,
> `scaffold job: ...`, `update job-file: ...`. With the (A) /
> (A.5) / (B) work above this grows by `pause-edit:` and likely
> `plan-run: ...`. The bots and the UI will both want to render
> these consistently.

### What to write

A short table at the top of JOB-MODEL.md or here:

| Prefix | Source | Meaning |
|---|---|---|
| `stage N: <title>` | Job runner closing-trio `git` | Stage N landed. |
| `update template: <name>` | `update_job_template` RPC | User edited the template between runs. |
| `update handover: <name>` | `update_job_handover` RPC | User edited the handover between runs. |
| `scaffold job: <name>` | `submit_job` | Initial seed of a job dir. |
| `update job-file: <name>/<file>` | `write_job_file` RPC | Overlay of SCOPE / WORKFLOW / etc. |
| `pause-edit: <name>` | Resume from `paused (with edits)` | User edits absorbed at resume time. |

### Why this is worth doing now

The bot adapters render commit messages to the user (Slack /
Telegram). Without a documented prefix vocabulary they will
either ignore the structure (bad) or invent their own rendering
per prefix (worse — drift). Documenting first means the bot
adapters can render by lookup instead of pattern-matching.

### Out of scope

- Enforcing the prefix in a pre-commit hook. The runtime writes
  these directly; user-driven commits via `mani` follow the
  convention by habit. A hook is overkill until we see drift.

## TODO — pause × Plan composition

> Status: **one-sentence note**, listed here so it doesn't get
> forgotten when pause and Plans both land. Stays small.

A PlanRun pauses by pausing its current step's Run. The Plan
engine sees the step's Run enter `paused`; it does **not** spawn
the next step until the Run reaches a terminal status
(`completed` / `failed` / `cancelled`). Resuming the step's Run
resumes the PlanRun implicitly. There is no first-class
"pause Plan" verb — that would mean pausing between steps, which
is what `on_success`-pointing-to-`stop` already gives you if the
template author wants it explicit.

Revisit when Plans get UI (P3) — at that point a `[pause plan]`
button on the PlanRun page might be worth the extra surface, but
not before.
