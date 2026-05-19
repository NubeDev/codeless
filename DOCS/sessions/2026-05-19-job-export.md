# 2026-05-19 — job-export

Job [`job-export`](../../.codeless/jobs/job-export/SCOPE.md) implements
the `.codeless-job` bundle (E1 of
[`DOCS/SCOPE-JOB-EXPORT.md`](../SCOPE-JOB-EXPORT.md)): export one Job
with its full per-Run history, import into another workspace, hit
run.

## Stage 1 — survey + precondition check  `[!]` halted

Stage 1 is "survey current Job/Run schema and confirm JOB-WORKFLOW
(B) precondition status; list every table column the bundle must
serialize."

The job's own
[`WORKFLOW.md`](../../.codeless/jobs/job-export/WORKFLOW.md#precondition-check-stage-1)
declares this as a hard halt:

> Stage 1's first action: confirm JOB-WORKFLOW (B) — the Job/Run
> split — is merged. Grep for the `runs` table migration; if it does
> not exist, halt the stage with `[!]` in the session doc and write
> a handover explaining that this job is blocked on (B). Do not try
> to scaffold around it; the bundle layout depends on immutable Run
> rows.

### Precondition check — result: **NOT MERGED**

`ls crates/codeless-runtime/migrations/` lists 27 migrations (0001
through 0028, no 0018). None of them creates a `runs` table. A grep
across the migration set for `runs`, `template_snapshot`, and
`handover_snapshot` returns only stray prose comments — no DDL:

```
$ grep -i "runs\|template_snapshot\|handover_snapshot" crates/codeless-runtime/migrations/*.sql
0009_job_system_prompt.sql:-- the row so reruns and resumes reproduce the prompt the user picked.
0011_personas.sql:     'General-purpose coding assistant. Writes, edits, and runs.',
0016_precheck_override_once.sql:-- the runner just before the pre-check runs. Distinct from
0026_supervisor_goals.sql:-- table load-bearing — the "if it runs >1h, stop it" example only
```

The `jobs` table still carries the per-attempt columns that
[JOB-WORKFLOW.md (B)](../JOB-WORKFLOW.md#b-full-step--split-job-and-run)
hoists onto `runs`: `runner`, `branch`, `worktree_path`,
`cost_cap_cents`, `wall_clock_cap_ms`, `cost_cents`, `started_at`,
`ended_at`, `template_yaml`, `prompt`. There is no immutable
per-attempt record, no `handover_md` column on jobs, no
`(job_id, ordinal)` uniqueness.

The codebase is on phase **(A)** of JOB-WORKFLOW. The `runs/NNNN/`
layout this bundle is built around therefore cannot be cleanly
constructed — there is exactly one implicit run per Job, with its
state co-mingled into the mutable `jobs` row. Exporting it as a
"frozen Run with `template_snapshot` / `handover_snapshot`" would
require either (i) fabricating snapshots at export time (which is
not what (B) promises and breaks resume-from-stage semantics in
imported history) or (ii) inventing a different bundle shape that
the importer in a post-(B) world will then have to translate. Both
violate `SCOPE.md`'s precondition and `WORKFLOW.md`'s "do not
scaffold around it."

This job is therefore **blocked on JOB-WORKFLOW (B)** and stage 1
halts with `[!]`. No source files were edited.

## Survey — current table columns the bundle would need

Captured here so the operator can compare what (B) ships against
what the bundle serializer will need to read. Sourced from
`crates/codeless-runtime/migrations/*.sql` (0001 initial plus every
`ALTER TABLE` since).

### `jobs` — today (pre-(B)), per `0001_initial.sql` + later ALTERs

| column | type | introduced |
| --- | --- | --- |
| `id` | TEXT PRIMARY KEY | 0001 |
| `repo_id` | TEXT NOT NULL → `repos(id)` | 0001 |
| `status` | TEXT NOT NULL | 0001 |
| `stop_reason` | TEXT | 0001 |
| `template_yaml` | TEXT | 0001 — moves to `runs.template_snapshot` under (B) |
| `prompt` | TEXT | 0001 |
| `runner` | TEXT NOT NULL | 0001 — moves to `runs` under (B) |
| `branch` | TEXT NOT NULL | 0001 — moves to `runs` |
| `worktree_path` | TEXT | 0001 — moves to `runs` |
| `cost_cap_cents` | INTEGER NOT NULL | 0001 — moves to `runs` |
| `wall_clock_cap_ms` | INTEGER NOT NULL | 0001 — moves to `runs` |
| `cost_cents` | INTEGER NOT NULL DEFAULT 0 | 0001 — moves to `runs` |
| `started_at` | INTEGER | 0001 — moves to `runs` |
| `ended_at` | INTEGER | 0001 — moves to `runs` |
| `created_at` | INTEGER NOT NULL | 0001 |
| `model` | TEXT | 0002 |
| `permission_mode` | TEXT | 0002 |
| `effort` | TEXT | 0002 |
| `workspace_mode` | TEXT NOT NULL DEFAULT 'in-repo' | 0004 |
| `system_prompt` | TEXT | 0009 |
| `persona_id` | TEXT → `personas(id)` | 0010 / 0012 (FK promoted in 0012) |
| `auto_bypass_policy` | TEXT | 0014 |
| `pending_operator_comment` | TEXT | 0015 |
| `precheck_override_once` | (per `0016_precheck_override_once.sql`) | 0016 |

Plus the table was recreated in `0012_stage_persona_id_and_jobs_persona_fk.sql`
to promote `persona_id` to an FK. Migration 0018 is intentionally
absent (a numbering skip; verified by `ls`).

Under (B), the bundle's `runs/NNNN/run.json` carries the
attempt-state columns; the bundle's `template.yaml` + `handover.md`
come from the mutable Job row (`jobs.template_yaml`,
`jobs.handover_md` once (B) adds it). The denylist in `SCOPE.md`
(`*token*`, `*secret*`, `*api_key*`) does **not** match any current
column name, but the serializer should still apply it
prospectively so a later migration that adds e.g. `runner_api_key`
is caught by the schema, not by code review.

### `stages` — per `0001_initial.sql` + later ALTERs

| column | type | introduced |
| --- | --- | --- |
| `id` | TEXT PRIMARY KEY | 0001 |
| `job_id` | TEXT NOT NULL → `jobs(id)` | 0001 — becomes `run_id` under (B) |
| `ordinal` | INTEGER NOT NULL | 0001 |
| `name` | TEXT NOT NULL | 0001 |
| `status` | TEXT NOT NULL | 0001 |
| `verify_cmd` | TEXT | 0001 |
| `started_at` | INTEGER | 0001 |
| `ended_at` | INTEGER | 0001 |
| `session_id` | TEXT | 0003 |
| `goal` | TEXT | 0005 |
| `acceptance` | TEXT | 0005 |
| `last_activity_at` | INTEGER | 0006 |
| `archived` | INTEGER NOT NULL DEFAULT 0 | 0006 |
| `persona_id` | TEXT → `personas(id)` | 0012 |
| `bypassed_at` | INTEGER | 0013 |
| `bypassed_reason` | TEXT | 0013 |
| `failure_class` | TEXT | 0017 |
| `failure_detail` | TEXT | 0017 |

### `tasks` — per `0001_initial.sql`

| column | type |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `stage_id` | TEXT NOT NULL → `stages(id)` |
| `ordinal` | INTEGER NOT NULL |
| `status` | TEXT NOT NULL |
| `depends_on` | TEXT NOT NULL DEFAULT '[]' |
| `lease_holder` | TEXT (`<pid>:<startup-nonce>`) |
| `lease_expires_at` | INTEGER |
| `cost_cents` | INTEGER NOT NULL DEFAULT 0 |
| `input_tokens` | INTEGER NOT NULL DEFAULT 0 |
| `output_tokens` | INTEGER NOT NULL DEFAULT 0 |
| `started_at` | INTEGER |
| `ended_at` | INTEGER |

Bundle note: `lease_holder` and `lease_expires_at` are runtime state,
not history. The serializer should drop them on export (no point
shipping a stale lease on a `<pid>` that doesn't exist on the
destination) and the importer should write NULL for both.

### `todos` — per `0021_todos.sql` + `0023_todo_failure_detail.sql`

| column | type |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `task_id` | TEXT NOT NULL → `tasks(id)` |
| `ordinal` | INTEGER NOT NULL |
| `title` | TEXT NOT NULL |
| `status` | TEXT NOT NULL (`pending`/`in-progress`/`done`/`skipped`/`failed`) |
| `kind` | TEXT NOT NULL (`runner`/`planner`/`checks`/`docs`/`git`) |
| `created_at` | INTEGER NOT NULL |
| `started_at` | INTEGER |
| `ended_at` | INTEGER |
| `failure_detail` | TEXT (0023) |

UNIQUE `(task_id, ordinal)`.

### `reviews` — per `0001_initial.sql`

| column | type |
| --- | --- |
| `id` | TEXT PRIMARY KEY |
| `stage_id` | TEXT NOT NULL → `stages(id)` |
| `status` | TEXT NOT NULL |
| `comment` | TEXT |
| `requested_at` | INTEGER NOT NULL |
| `resolved_at` | INTEGER |

### `events` — per `0001_initial.sql`

| column | type |
| --- | --- |
| `cursor` | INTEGER PRIMARY KEY AUTOINCREMENT (source-workspace monotonic — confirms open Q 4 in `SCOPE.md`) |
| `job_id` | TEXT (nullable) — becomes `run_id` under (B) per JOB-WORKFLOW migration script |
| `stage_id` | TEXT |
| `task_id` | TEXT |
| `type` | TEXT NOT NULL |
| `payload` | TEXT NOT NULL (JSON) |
| `created_at` | INTEGER NOT NULL |

The bundle's per-event `original_id` (per `SCOPE-JOB-EXPORT.md` open
question 6) maps to the source `cursor`. The importer rewrites
`cursor` in the destination's sequence and preserves `created_at`,
`type`, `payload`, and the `(stage_id, task_id)` triplet — but
needs (B)'s migration to be in flight so it can write `run_id`.

### Tables explicitly **out of the bundle**

- `repos`, `attached_workspaces` — bundle records `repo_url` +
  `repo_commit` in the manifest; the destination's repo row is
  authoritative.
- `pty_sessions` — runtime state on the source host; not history.
- `assistant_*` (0008, 0019, 0020, 0028) — the in-app Assistant
  thread / messages / attachments belong to the user, not the Job.
  Scope doc carries no mention of including them.
- `personas` (0011) — `jobs.persona_id` and `stages.persona_id` are
  exported as opaque IDs; the destination must already have the
  same persona row or import will surface a warning. Out of scope
  for E1.
- `scheduled_pause_points` (0022) — Job-scoped pause schedule;
  arguably should be exported. Logged as a follow-up question for
  stage 2.
- `chat_messages` (0024) / `chat_bindings` (0025) /
  `supervisor_goals` (0026) / `chat_adapters` (0027) /
  `runner_config` (0027) — workspace-level surfaces, not Job
  history.

## What stage 2 must do once (B) lands

1. Re-survey the schema: (B) will introduce `runs`, rename
   `stages.job_id` → `stages.run_id`, and re-key events. The column
   inventory above becomes stale.
2. Lock the manifest JSON shape and bundle directory layout against
   `DOCS/SCOPE-JOB-EXPORT.md` §"What's in the bundle".
3. Resolve `SCOPE.md` open questions 2 (size cap defaults),
   3 (`output_path` jailing), 4 (confirm `events.cursor` monotonicity
   under (B)), 5 (handover lands on `jobs.handover_md`).
4. Lock the secrets denylist regex set and the column-name walk.
5. Write the stage-3 REVIEW packet per `WORKFLOW.md`.

## Verify

No source diff. `cargo` checks not run (no code touched). Per
`WORKFLOW.md` closing trio for a read-only stage: handover + session
doc committed only.

## Stage 2 — design lock  `[x]`

Stage 2 ("design bundle layout and manifest.json schema_version 1
against DOCS/SCOPE-JOB-EXPORT.md; lock the secrets denylist and
per-entry size caps") is paper-only and can land independently of
the (B) precondition that gates implementation stages 4–7.

### Re-verification of the (B) blocker

Re-grepped this stage: `crates/codeless-runtime/migrations/` still
has no `runs` migration; `jobs` still owns the per-attempt columns;
no `handover_md` on `jobs`. Stage 1's halt stands; the design below
targets the post-(B) world per `DOCS/SCOPE-JOB-EXPORT.md`.

### Deliverable

[`.codeless/jobs/job-export/BUNDLE-DESIGN.md`](../../.codeless/jobs/job-export/BUNDLE-DESIGN.md)
— the frozen design lock that stage 3 REVIEW reads and stages 4–7
build against. Covers in ten sections:

1. Bundle file shape (gzipped tar, `.codeless-job`, UTF-8,
   deterministic ordering, mtime-zeroed gzip header).
2. Directory layout — every allowed path, every rejected entry
   shape, ordinal-dir zero-padding, top-level dir stripping.
3. `manifest.json` schema_version 1 — every key typed, required,
   `deny_unknown_fields`, the importer's first read.
4. Per-Run `run.json` field set (with `worktree_path_source`
   rename so destination can't accidentally reuse it) and the
   five JSONL streams' sort orders + drop rules (`lease_holder`
   etc. dropped on export).
5. Secrets denylist (case-insensitive substring regex set:
   `token`, `secret`, `api[_-]?key`, `pass(word|wd)`,
   `private[_-]?key`, `credential`, `bearer`, `auth[_-]?header`).
   Confirmed: zero current columns match.
6. Size caps — 200 MiB per bundle, 10 MiB per entry, plus per-kind
   sub-caps (64 KiB manifest / README, 1 MiB template / handover /
   note / run.json / JSONL line, 1024 runs/bundle, 500k
   events/run); constants live in `limits.rs`; importer enforces
   streaming.
7. Open question resolutions — OQ-1 through OQ-5 from the scope
   doc plus OQ-D (event payload scanning: out for E1) and OQ-E
   (`scheduled_pause_points`: out for E1).
8. Refuse-to-export preconditions (non-terminal Run, empty repo
   URL, output path outside workspace `fs_root_canonical`, any cap
   exceeded).
9. README cover note outline.
10. What stages 3–7 inherit + what's deliberately not locked
    (tar/gzip lib choice, exact error variant names).

### Locked open questions, one-liner each

- OQ-2 (size caps): values per §5 of the design doc.
- OQ-3 (output path): jailed under workspace `fs_root_canonical`
  via canonicalised prefix check.
- OQ-4 (events.cursor monotonicity): confirmed by SQLite
  AUTOINCREMENT; (B) re-keys `job_id → run_id` only.
- OQ-5 (handover lands on `jobs.handover_md`): confirmed; first
  new Run snapshots like any other.
- OQ-D (payload scanning): out for E1; README warns.
- OQ-E (`scheduled_pause_points`): out for E1; logged for E2.

### Verify

No source diff. New file:
`.codeless/jobs/job-export/BUNDLE-DESIGN.md`. `cargo` checks not
run (no code touched). Commit: handover + session doc + design doc
only.
