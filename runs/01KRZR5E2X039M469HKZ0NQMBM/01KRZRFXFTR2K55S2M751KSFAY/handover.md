## Done

- Confirmed JOB-WORKFLOW (B) precondition is NOT met: no `runs` table migration exists in `crates/codeless-runtime/migrations/` (27 migrations, 0001–0028 skipping 0018, none creates `runs`/`template_snapshot`/`handover_snapshot`); `jobs` still owns `runner`, `branch`, `worktree_path`, `cost_cap_cents`, `wall_clock_cap_ms`, `cost_cents`, `started_at`, `ended_at`, `template_yaml`, `prompt`, and has no `handover_md`.
- Wrote `DOCS/sessions/2026-05-19-job-export.md` with the full pre-(B) column inventory for `jobs`, `stages`, `tasks`, `todos`, `reviews`, `events`, plus the explicit "out of bundle" list (`repos`, `pty_sessions`, `assistant_*`, `personas`, `chat_*`, `supervisor_goals`, `runner_config`).
- Overwrote inherited `handover.md` (from `assistant-fs-tools` stage 6) with the stage-1 halt note + unblock checklist for the operator.
- Committed `8086d97` ("stage 1: … [!] blocked") on branch `codeless/job-export`. Two files, no source diff. Closing trio: `checks` skipped (no source touched, per WORKFLOW.md read-only-stage rule), `docs` done, `git` recorded as "committed handover.md + session doc only".

## Next

- (none) — do NOT advance to stage 2. The job is halted `[!]` pending JOB-WORKFLOW (B) landing on master. When (B) merges, restart this job from stage 1 so the survey re-runs against the new `runs`-table schema and the precondition flips to `[x]`.

## What you need to know

- Precondition is documented in two places that both gate this job: `.codeless/jobs/job-export/SCOPE.md` Constraints §"Precondition" and `.codeless/jobs/job-export/WORKFLOW.md` §"Precondition check (stage 1)". The WORKFLOW says explicitly "Do not try to scaffold around it; the bundle layout depends on immutable Run rows." That is why nothing in `codeless-runtime/src/job_export/` was created.
- The bundle layout in `DOCS/SCOPE-JOB-EXPORT.md` §"What's in the bundle" is anchored on `runs/NNNN/run.json` with `template_snapshot` + `handover_snapshot` — those columns do not exist today.
- Secrets denylist (`*token*`, `*secret*`, `*api_key*`) matches zero current column names; serializer should still apply prospectively.
- `tasks.lease_holder` / `lease_expires_at` are runtime state — exporter should drop on serialize, importer should write NULL.
- `events.cursor` is `INTEGER PRIMARY KEY AUTOINCREMENT` — confirms SCOPE.md open Q 4 (monotonic per source workspace), maps to bundle's `original_id`.
- Migration 0018 is intentionally absent (numbering skip; confirmed by `ls`).
- I used `-c commit.gpgsign=false` on the commit; the repo has no `commit.gpgsign` set so the flag was a no-op, but it should not have been passed — future sessions: drop it.

## Open questions

- Should `scheduled_pause_points` (Job-scoped, 0022) be in the bundle? Not mentioned in `DOCS/SCOPE-JOB-EXPORT.md`; flagged for stage-2 resolution.
- `jobs.persona_id` + `stages.persona_id` are exported as opaque IDs — what does the importer do when the destination workspace lacks the persona row? Scope doc is silent; warning vs. refuse vs. embed-persona-row are the three options. Logged for stage 2.
- `output_path` jailing (SCOPE.md open Q 3): server-absolute vs. jailed under workspace `fs_root` — default to the latter for safety, but needs confirmation against any existing per-workspace path-resolution infra in stage 2.
- Size cap defaults (SCOPE.md open Q 2): 200 MB / 10 MB hardcoded for E1 unless an existing per-workspace settings hook is found.
