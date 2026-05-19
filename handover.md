# job-export — stage 1 `[!]` halted: blocked on JOB-WORKFLOW (B)

Stage 1 ("survey current Job/Run schema and confirm JOB-WORKFLOW (B)
precondition status; list every table column the bundle must
serialize") halts with `[!]`. Per
[`.codeless/jobs/job-export/WORKFLOW.md`](.codeless/jobs/job-export/WORKFLOW.md#precondition-check-stage-1)
the first action is the precondition check, and per
[`.codeless/jobs/job-export/SCOPE.md`](.codeless/jobs/job-export/SCOPE.md#constraints)
**JOB-WORKFLOW (B) — the Job/Run split — must already be merged on
master** before any code lands.

## Why stage 1 is halted

`crates/codeless-runtime/migrations/` contains 27 migrations
(0001…0028, no 0018). None creates a `runs` table. The grep is
clean:

```
$ grep -i "runs\|template_snapshot\|handover_snapshot" \
    crates/codeless-runtime/migrations/*.sql
0009_job_system_prompt.sql:-- the row so reruns and resumes reproduce …
0011_personas.sql:     'General-purpose coding assistant. …'
0016_precheck_override_once.sql:-- … the pre-check runs. …
0026_supervisor_goals.sql:-- … "if it runs >1h, stop it" …
```

The `jobs` table still owns every per-attempt column that
[`DOCS/JOB-WORKFLOW.md` §(B)](DOCS/JOB-WORKFLOW.md#b-full-step--split-job-and-run)
hoists onto `runs` (`runner`, `branch`, `worktree_path`,
`cost_cap_cents`, `wall_clock_cap_ms`, `cost_cents`, `started_at`,
`ended_at`, `template_yaml`, `prompt`). There is no `handover_md`
column on `jobs`, no `(job_id, ordinal)` uniqueness, no
`template_snapshot` / `handover_snapshot` columns.

The bundle layout in
[`DOCS/SCOPE-JOB-EXPORT.md`](DOCS/SCOPE-JOB-EXPORT.md) is built
around `runs/NNNN/run.json` with the frozen snapshots and per-Run
JSONL streams. Without (B) there is no Run row to serialize and no
snapshot to freeze. `SCOPE.md` and `WORKFLOW.md` both forbid
scaffolding a different shape now and translating later.

## What was produced this stage

- This `handover.md` (overwriting the inherited
  `assistant-fs-tools` handover).
- [`DOCS/sessions/2026-05-19-job-export.md`](DOCS/sessions/2026-05-19-job-export.md) —
  the halt record, the full survey of the **current** (pre-(B))
  Job / Stage / Task / Todo / Event / Review column inventory the
  bundle would need to serialize, the explicit "tables out of the
  bundle" list (repos, pty_sessions, assistant_*, personas, chat_*,
  supervisor_goals, runner_config), and the to-do for stage 2 once
  (B) lands.

No source files were edited. No `cargo` commands were run (per
`WORKFLOW.md` closing trio for a read-only stage that produced no
source diff: handover + session doc committed only; `git` recorded
as `committed handover.md only`).

## Unblock checklist for the operator

This job cannot start until JOB-WORKFLOW (B) merges. Concretely, the
next session can only proceed once **all** of the following are
true on master:

1. A `runs` migration exists (e.g. `0029_split_jobs_into_runs.sql`)
   with columns matching
   [`JOB-WORKFLOW.md` §(B)](DOCS/JOB-WORKFLOW.md#b-full-step--split-job-and-run):
   `id`, `job_id`, `ordinal`, `template_snapshot`,
   `handover_snapshot`, `runner`, `branch`, `worktree_path`,
   `status`, `stop_reason`, `started_at`, `ended_at`,
   `cost_cap_cents`, `wall_clock_cap_ms`, `cost_cents`,
   `resumed_from_stage`, `created_at`; UNIQUE `(job_id, ordinal)`.
2. `jobs` carries `handover_md` and has shed the columns that
   moved to `runs` (the `0012`-style table-recreate pattern is the
   precedent).
3. `stages.job_id` is renamed `run_id` (or a parallel `run_id`
   column has been backfilled and `job_id` retired) and the same
   for `events.job_id`.
4. `codeless-types` carries a `Run` struct; `submit_job` returns
   a Run; `list_runs(job_id)` / `get_run(run_id)` exist on
   `codeless-rpc`.
5. The migration script that synthesises ordinal-1 Runs for
   pre-(B) Jobs has run on the dev `codeless.db` so there is
   something for the round-trip test in stage 7 to chew on.

When (B) is on master, restart this job. Stage 1 then re-runs the
survey against the new schema, marks the precondition `[x]`, and
hands stage 2 a column inventory keyed on `runs` instead of `jobs`.

## What stage 2 will pick up (after unblock)

Stage 2 ("design bundle layout and manifest.json schema_version 1
against DOCS/SCOPE-JOB-EXPORT.md; lock the secrets denylist and
per-entry size caps") needs:

- The post-(B) column inventory (re-do the survey in the session
  doc).
- Confirm or revise SCOPE.md open questions 2 (200 MB / 10 MB
  defaults), 3 (`output_path` jailed under workspace `fs_root`),
  4 (events.cursor monotonicity under (B)), 5 (handover lands on
  `jobs.handover_md`).
- Decide what to do with `scheduled_pause_points` (Job-scoped, was
  flagged as "arguably should be in the bundle" in the survey —
  not in SCOPE-JOB-EXPORT.md as written; needs a call).
- Lock the secrets-column denylist regex set (`*token*`,
  `*secret*`, `*api_key*`) and confirm no current column matches.

## Verify

- No source diff this stage.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test --workspace` — **not run** (no code
  touched).
- Commit: handover + session doc only. `git` step: committed
  `handover.md` + session doc only.
