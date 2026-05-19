# job-export — stage 6 `[!]` halted on (B) + stage-4/5 chain

Stage 6 ("wire `export_job`, `import_job`, `inspect_job_bundle`
RPCs in `codeless-rpc` and `codeless-runtime`; refuse export of any
Job with a non-terminal Run") halted before any code landed. The
full stage 6 record — re-verification, four blockers, and a
file-by-file plan for the agent that picks this up — is in
[`DOCS/sessions/2026-05-19-job-export.md`](DOCS/sessions/2026-05-19-job-export.md)
under "Stage 6 — RPC wiring  `[!]` halted on (B) + stage-4/5 chain".

## Why halted (the short version)

Three preconditions remain unmet:

1. **JOB-WORKFLOW (B) not merged.** Re-grepped this session:
   `crates/codeless-runtime/migrations/` is unchanged at 27 files
   (0001…0028 minus 0018); no migration creates a `runs` table,
   `events.run_id`, or `jobs.handover_md`. Stages 1, 4, 5 halted
   on the same finding; the world has not moved.
2. **Stage 4 (walker + serializer) and stage 5 (importer) source
   do not exist.** `ls crates/codeless-runtime/src/job_export` →
   no such directory. The two commits titled `stage 3:` and
   `stage 4:` between this halt and the prior one (`92e844c`,
   `a579c7e`) are both session-doc-only — `git show --stat` lists
   one file each, a handover. No `walker.rs` / `serializer.rs` /
   `manifest.rs` / `tar_writer.rs` / `limits.rs` / `denylist.rs` /
   `importer.rs` / `tar_safety.rs` / `inspect.rs` on disk.
3. **No wire types in `codeless-rpc`.** Grep for `ExportJob` /
   `ImportJob` / `InspectJobBundle` returns zero hits across
   `codeless-rpc/src/`. Stage 2's commit (`2840f81`) was also
   session-doc-only.

The RPC seam this stage adds dispatches into runtime fns that
don't exist, exposes wire types that don't exist, and requires
inspecting `runs.status` for a terminal value before allowing
export — but `runs` itself doesn't exist. The "refuse non-
terminal Run" precondition cannot be implemented against a schema
with no Run concept; faking it against `jobs.status` writes a
check that the post-(B) world silently deletes. Per repo
`CLAUDE.md` R4 and `WORKFLOW.md` §"Precondition check", halt with
`[!]`; do not commit a partial implementation with a TODO. No
methods, traits, errors, or impls were added.

## What lands when stage 6 is re-fired (after stages 4 + 5 + (B))

Detailed in the session doc; one-line summary:

1. Add `ExportJobArgs/Result/Error`, `ImportJobArgs/Result/Error`,
   `InspectJobBundleArgs/Result`, `ImportConflictPolicy`,
   `ImportWarning`, `ExportWarning` to `codeless-rpc/src/methods.rs`
   with `specta::Type` + `serde::deny_unknown_fields`.
2. Add three trait methods to `RpcServer` in
   `codeless-rpc/src/server.rs`; matching `MockRpc` impls.
3. Add thin runtime impls that delegate to
   `codeless_runtime::job_export::{export_job,import_job,inspect}`.
   The "refuse non-terminal Run" check lives in the export
   wrapper (RPC seam), not the walker — wraps `SELECT status FROM
   runs WHERE job_id=? ORDER BY ordinal DESC LIMIT 1` and
   surfaces `ExportError::NonTerminalRun`.
4. Tests: serde round-trip for every type +
   `deny_unknown_fields` rejection fixtures + per-RPC test
   (`NonTerminalRun` refusal, `inspect_job_bundle` round-trip).
   Round-trip integration test deferred to stage 7.
5. `cargo test --workspace`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo fmt --check` all green.

## What you need to know

- Branch `codeless/job-export`; worktree at
  `/home/user/.codeless/worktrees/job-01KRZR5E2X039M469HKZ0NQMBM`.
  Halt commits in this job have used raw `git` (no `mani.yaml` in
  this inner-repo worktree). Switch to `./bin/mani --config
  mani.yaml run commit/push --projects codeless` from the
  workspace root the first time real code lands.
- Design lock is `.codeless/jobs/job-export/BUNDLE-DESIGN.md`
  §§3, 6 — load-bearing for the wire types this stage will add.
  `SCOPE-JOB-EXPORT.md` §"RPC surface" is the matching reference.
- For E1 only `ImportConflictPolicy::Refuse` is wired; `Suffix`
  and `Replace` return `ImportError::UnsupportedConflictPolicy`.

## Open questions

- Operator should gate stage firing on `[ -d
  crates/codeless-runtime/src/job_export ]` or on the migrations
  grep until (B) lands; the loop has now fired six stages (1, 4,
  5, 6, plus two duplicate-titled "3"/"4" doc-only commits)
  against the same unresolved precondition. Each fire burns a
  session for zero forward motion.
- Who owns JOB-WORKFLOW (B) and what is the ETA? Stages 6–10
  remain dormant until it merges and stages 4 + 5 actually
  produce source.
- Stages 7, 8 (REVIEW), 9–10 (UI) are all transitively blocked
  on this same chain.
