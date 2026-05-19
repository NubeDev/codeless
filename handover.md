# job-export — stage 7 `[!]` halted on (B) + stage-4/5/6 chain

Stage 7 ("add round-trip property test (export then import into
scratch workspace, assert row body equality and ordering) and
tar-safety unit tests") halted before any code landed. The full
stage 7 record — re-verification, why no test can land, and the
file-by-file plan for re-fire — is in
[`DOCS/sessions/2026-05-19-job-export.md`](DOCS/sessions/2026-05-19-job-export.md)
under "Stage 7 — round-trip property test + tar-safety units
`[!]` halted on same chain".

## Why halted (the short version)

Stage 7 writes tests against modules that do not exist. Re-verified
this session:

1. **JOB-WORKFLOW (B) not merged.** `ls
   crates/codeless-runtime/migrations/ | grep -i run` returns only
   `0002_job_runner_overrides.sql`. No `runs` table, no
   `events.run_id`, no `jobs.handover_md`. The round-trip
   assertion's `runs` row-body equality has no columns to
   compare.
2. **No `job_export` runtime module.** `ls
   crates/codeless-runtime/src/job_export` → no such directory.
   Stages 4 (walker + serializer) and 5 (importer) and 6 (RPCs)
   never produced source — every commit in their slot was
   session-doc + handover only.
3. **No `tar_safety` to unit-test.** The guards listed in
   `.codeless/jobs/job-export/SCOPE.md` §"Importer guards" have
   no implementation file. Writing the rejection tests against an
   absent module is meaningless.
4. **No wire types.** `grep -rl "ExportJob\|ImportJob\|
   InspectJobBundle" crates/codeless-rpc/src` → zero hits. The
   property test's pre/post row comparison has no defined shape.

Per `WORKFLOW.md` §"Precondition check" and repo `CLAUDE.md` R4,
halt with `[!]`. No source touched.

## What lands when stage 7 is re-fired (after (B) + stages 4, 5, 6)

Detailed in the session doc; one-line summary:

1. `crates/codeless-runtime/tests/job_export_roundtrip.rs` —
   `proptest!` generates fixture Job + 1–4 terminal Runs with
   varied Stage / Task / Todo / Event / Review counts; exports
   to `TempDir`; imports into fresh in-memory SQLite + scratch
   `WorkspaceId`; asserts row-body equality on immutable columns
   and ordering on `(run.ordinal, stage.ordinal, task.ordinal,
   todo.ordinal)` plus `events.cursor` re-keying preserves order.
   Skips runtime-state columns (`tasks.lease_holder`,
   `tasks.lease_expires_at`).
2. Unit tests beside `crates/codeless-runtime/src/job_export/
   tar_safety.rs` — one per guard: absolute path, `..` segment,
   symlink entry, off-layout entry, per-entry size cap, total
   uncompressed cap. Each builds a minimal tar via
   `tar::Builder`, runs the safety check, asserts the exact
   `ImportError::TarSafety { kind, path }` variant.
3. Closing trio (`cargo test/clippy/fmt` all green); commit via
   `./bin/mani --config mani.yaml run commit/push --projects
   codeless` from the workspace root.

## What you need to know

- Branch `codeless/job-export`; worktree at
  `/home/user/.codeless/worktrees/job-01KRZR5E2X039M469HKZ0NQMBM`.
  Halt commits in this job have used raw `git` (no `mani.yaml` in
  this inner-repo worktree). Switch to mani from the workspace
  root the first time real code lands.
- Stage 8 is the mid-job REVIEW gate. Firing it against the
  current tree gives the user no manifest output, no transcript,
  no tar-safety results — it will itself halt. Operator should
  not advance to stage 8 until stages 4 + 5 + 6 + 7 have all
  landed real source.

## Open questions

- Five consecutive stages (1, 4, 5, 6, 7) have now halted on the
  same chain. Until the operator gates firing on `[ -d
  crates/codeless-runtime/src/job_export ]` or on the (B)
  migrations grep, every fire burns a session for no work.
- Who owns JOB-WORKFLOW (B) and what is the ETA? Stages 7–10
  remain dormant until it merges and stages 4 + 5 + 6 produce
  source.
