# JOB-WORKFLOW — turning a Job into an editable, iterable artifact

> Companion to [`JOB-MODEL.md`](./JOB-MODEL.md). JOB-MODEL.md
> establishes the **file contract** (`.codeless/jobs/<name>.yaml`,
> `runs/<name>/handover.md`, `runs/<name>/log.md`). This doc covers
> the **interaction contract** — what the user can edit, what they
> can re-run, what flows feedback from one run to the next. JOB-MODEL
> describes the artifacts; JOB-WORKFLOW describes the loops the user
> closes with them.
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
