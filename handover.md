# job-export — stage 10 `[!]` halted on (B) + stage-4/5/6 chain

Stage 10 ("add imported-from chip on imported Jobs plus read-only
manifest viewer; verify `[run]` on an imported Job creates ordinal
max+1 against destination HEAD") halted before any code landed. The
full stage 10 record — re-verification, why no chip / viewer / test
can land, and the file-by-file plan for re-fire — is in
[`DOCS/sessions/2026-05-19-job-export.md`](DOCS/sessions/2026-05-19-job-export.md)
under "Stage 10 — imported-from chip + manifest viewer + ordinal
verification `[!]` halted on same chain".

## Why halted (the short version)

Stage 10 builds against four absent foundations. Re-verified this
session:

1. **JOB-WORKFLOW (B) not merged.** `ls
   crates/codeless-runtime/migrations/ | grep -i run` returns only
   `0002_job_runner_overrides.sql`. No `runs` table, so the "next
   `[run]` writes `ordinal = MAX(ordinal) + 1`" assertion has
   nothing to assert against.
2. **No `job_export` runtime module.** `ls
   crates/codeless-runtime/src/job_export` → no such directory.
   Stages 4 (walker + serializer), 5 (importer), 6 (RPCs), 7
   (round-trip tests) never produced source.
3. **No `imported_from` field anywhere.** `grep -rn
   "imported_from\|importedFrom" crates/codeless-runtime/migrations
   crates/codeless-rpc/src crates/codeless-types/src` → zero. The
   chip + viewer have no data source on the destination Job row.
4. **Stages 8–9 already shipped UI that the server can't answer.**
   `ImportJobDialog.tsx` and the `export_job` / `import_job` /
   `inspect_job_bundle` TS RPC declarations exist; the matching
   server methods do not. The loop is already carrying one R4
   half-finished-implementation violation forward; stage 10 not
   deepening it is the only consistent choice with stages 1, 4, 5,
   6, 7.

## What lands when stage 10 is re-fired

Re-fire only after (B) merges and stages 4 + 5 + 6 + 7 land real
source. Single-commit deliverable:

1. Migration adding `jobs.imported_from_workspace_name`,
   `imported_from_repo_url`, `imported_from_repo_commit`,
   `imported_from_job_id`, `imported_at`, `imported_manifest_json`
   (verbatim manifest bytes — chip + viewer read it without a
   second RPC).
2. `codeless-types::Job` gains those six fields; specta regen
   updates `ui/codeless-ui/src/lib/rpc/generated/wire.ts`.
3. `<ImportedFromChip />` + `<JobManifestViewerDialog />` under
   `ui/codeless-ui/src/modules/jobs/`; shared
   `<ManifestSummary />` extracted from `ImportJobDialog`'s
   preview block (R3). `JobPage` renders the chip near the title;
   click opens the viewer.
4. `crates/codeless-runtime/tests/job_export_imported_run.rs`:
   import a 3-Run fixture bundle, drive `start_job`, assert the
   new `runs` row carries `ordinal = 4` and `(job_id, ordinal)`
   uniqueness held, and that `events.run_id` for the new Run
   points at the new row id (not at any imported Run's id).
5. Closing trio: `cargo test --workspace` green, `cargo clippy
   --workspace --all-targets -- -D warnings` green, `cargo fmt
   --check` green; `pnpm --filter codeless-ui test` + `lint` green.

## State of the job

Ten of ten stages have now fired against the same unresolved chain.
Six halted clean (1, 4, 5, 6, 7, 10); stages 8 + 9 landed UI in
advance of the server it calls. The job has exhausted its stage
list with the foundation unmerged.

Operator should either:

- **(a)** merge JOB-WORKFLOW (B), then re-fire stages 4 → 10 in a
  fresh session in order; or
- **(b)** revert stages 8 + 9's premature UI commits
  (`86144a2`, `2de4369`) so the next iteration starts from a clean
  base where every RPC the UI calls has a server impl.

Either way, gating loop firing on `[ -d
crates/codeless-runtime/src/job_export ]` or on the (B)
migrations grep would have saved six full sessions; consider it
for the next chain of dependent stages.
