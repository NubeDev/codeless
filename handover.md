# job-export — stage 5 `[!]` halted on (B) + missing stage-4 chain

Stage 5 ("implement importer with tar-streaming, path-traversal
guards, batched SQLite inserts in one transaction, and
ImportConflictPolicy (Refuse default)") halted before any code
landed. The full stage 5 record — re-verification, three blockers,
and a file-by-file plan for the agent that picks this up — is in
[`DOCS/sessions/2026-05-19-job-export.md`](DOCS/sessions/2026-05-19-job-export.md)
under "Stage 5 — importer  `[!]` halted on (B) + stage-4 chain".

## Why halted (the short version)

Two preconditions are still unmet:

1. **JOB-WORKFLOW (B) not merged.** Re-grepped this session:
   `crates/codeless-runtime/migrations/` is unchanged at 27 files
   (0001…0028 minus 0018); no migration creates a `runs` table; no
   migration adds `template_snapshot`, `handover_snapshot`,
   `events.run_id`, or `jobs.handover_md`. Stages 1 and 4 halted on
   the same finding; nothing has moved.
2. **Stage 4 (walker + serializer) source does not exist.**
   `ls crates/codeless-runtime/src/job_export` → no such directory.
   The commit titled `stage 3: implement codeless-runtime
   job_export walker plus serializer …` (`92e844c`) is doc-only:
   `git show --stat 92e844c` lists one file (a handover). The loop
   runner appears to have re-fired stage 4 under a renumbered title
   without producing the `manifest.rs` / `walker.rs` / `serializer.rs`
   / `tar_writer.rs` / `limits.rs` / `denylist.rs` modules that the
   importer must consume.

The importer reads the bundle stage 4 emits and writes into the
post-(B) schema. With neither in place, every concrete piece of
stage 5 — JSONL deserialization, batched inserts into `runs` /
`events.run_id`, `inspect_job_bundle` manifest validation — has no
target. Per repo `CLAUDE.md` R4 and `WORKFLOW.md` §"Precondition
check", halt with `[!]`; do not commit a partial implementation
with a TODO. No `importer.rs` / `tar_safety.rs` / `inspect.rs` were
created.

## What lands when stage 5 is re-fired (after stages 4 + (B))

Full plan is in the session doc. Headline points:

- New files under `crates/codeless-runtime/src/job_export/`:
  `tar_safety.rs` (strict allow-list of bundle paths, hostile-tar
  fixtures), `inspect.rs` (manifest-only read, size cap check,
  `schema_version == 1`), `importer.rs` (streaming tar decode,
  one-transaction SQLite write, FK rewrite map keyed on source IDs,
  conflict policy dispatch).
- For E1 only `ImportConflictPolicy::Refuse` is wired; `Suffix` and
  `Replace` return `ImportError::UnsupportedConflictPolicy`. Default
  in the RPC arg type is `Refuse`.
- `lease_holder` / `lease_expires_at` written as NULL.
  `worktree_path_source` (renamed from `worktree_path` by stage 4)
  is read for forensics but not written; destination Run allocates
  its own path on first run.
- Crate boundary (R1): `tar`, `flate2`, `sqlx`, `ulid` all stay in
  `codeless-runtime`. The wire types in `codeless-rpc::methods`
  (`ImportJobArgs`, `ImportJobResult`, `ImportConflictPolicy`,
  `ImportWarning`, `ImportError`) are reused verbatim.
- Tests with the code (R5): tar-safety hostile fixtures, inspect
  edge cases, importer refuse-on-collision + FK-rewrite correctness.
  The round-trip property test stays in stage 7.

## Pointers for the next session

- Design lock: [`.codeless/jobs/job-export/BUNDLE-DESIGN.md`](.codeless/jobs/job-export/BUNDLE-DESIGN.md) §§3–6 are
  load-bearing for the importer (manifest schema, JSONL field set,
  denylist columns, size caps).
- Stage-by-stage record: [`DOCS/sessions/2026-05-19-job-export.md`](DOCS/sessions/2026-05-19-job-export.md).
  Stage 4's "what stage 4 will do once (B) lands" checklist must run
  to completion before stage 5 fires; the stage-5 plan added this
  session is conditional on that.
- Branch: `codeless/job-export`. Inner-repo worktree only — no
  `mani.yaml` here, so this halt commit uses raw `git` matching the
  prior halt commits. The first code-bearing stage must switch to
  `./bin/mani --config mani.yaml run commit/push --projects codeless`
  from the workspace root.

## Open questions

- Is JOB-WORKFLOW (B) being worked on in a parallel session? If so,
  who owns it and what is the ETA? Stages 4–7 of `job-export` are
  dormant until it merges.
- Should the loop runner be told to stop firing implementation
  stages 4–7 until (B) is in? Each fire costs a session and produces
  the same halt commit shape.
- The `92e844c` commit titled `stage 3: implement …` is mislabelled
  (it is doc-only). The operator may want the loop runner to skip
  duplicate stage titles, or to detect that the named source files
  are still absent before re-numbering the next stage.
