# Scope — job-export

The full design lives in
[`DOCS/SCOPE-JOB-EXPORT.md`](../../../DOCS/SCOPE-JOB-EXPORT.md). This
file is the per-job brief; read both.

## Goal

Give the user a way to package one Job — its current template,
handover, notes, **and the full per-Run history** (Runs, Stages,
Tasks, Todos, Events, Reviews) — into a single `.codeless-job`
bundle, and import that bundle into any attached workspace so that
hitting `[run]` Just Works. After import, prior Runs show as
read-only history; the next Run is a normal fresh Run cut against
the destination workspace's repo at its current HEAD.

This job ships **E1 only**: core export + import + minimal UI. E2
(conflict policies beyond `Refuse`, artifacts, CLI) and E3 (signed
bundles, cross-version shim) are separate jobs.

## In scope

- `codeless-runtime/src/job_export/` module containing the walker,
  serializer, manifest builder, and importer. One module, one
  concept per file.
- New RPCs in [`codeless-rpc`](../../../crates/codeless-rpc/src/methods.rs):
  - `export_job(job_id, output_path, include_artifacts)`
  - `import_job(workspace_id, bundle_path, rename_to?, on_conflict)`
  - `inspect_job_bundle(bundle_path)` (manifest-only, no SQLite
    writes).
- `manifest.json` `schema_version: 1` exactly as specified in
  `DOCS/SCOPE-JOB-EXPORT.md` §"What's in the bundle".
- Bundle layout: gzipped tar with `manifest.json`, `template.yaml`,
  `handover.md`, `notes/`, `runs/NNNN/{run.json, *.jsonl}`,
  `README.md`.
- Importer guards: reject absolute paths, `..` segments, symlinks,
  entries outside the fixed layout, per-bundle and per-entry size
  caps.
- Secrets denylist on the row serializer (`*token*`, `*secret*`,
  `*api_key*`) and an explicit refusal to walk
  `$XDG_DATA_HOME/codeless/` outside the SQLite rows themselves.
- UI: `[Export]` on the Job page header; `[Import Job…]` on the
  active workspace's sidebar menu; preview dialog backed by
  `inspect_job_bundle`; warnings banner on the imported Job page;
  `imported from <repo>@<sha>` chip + read-only manifest viewer.
- `ImportConflictPolicy::Refuse` end-to-end; the field exists in
  the wire type for E2 to fill in.
- Round-trip property test plus tar-safety unit tests covering each
  rejected entry shape.

## Out of scope

- E2 work: `Suffix` / `Replace` conflict policies, artifact
  inclusion, CLI subcommands. Wire-type slots exist; behaviour is
  E2.
- E3 work: Ed25519 manifest signatures, cross-version importer
  shim, `import.require_signature_from`.
- Live-Job export. Exporting a Job with a non-terminal Run is
  explicitly refused.
- Plan bundles. A Plan bundle is its own doc once Plans get UI
  (P3 in [`JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md)).
- Editing imported historical Run rows. Imported Runs are
  read-only; the mutable surface is the Job's template + handover
  + notes (the existing iterate loop).
- Hosted bundle sharing, federation, replaying events through the
  runtime.

## Constraints

- R1 (codeless workspace `CLAUDE.md`): the export walker and the
  importer live in `codeless-runtime` and `codeless-adapters-host`
  only. Nothing under `codeless-types` / `codeless-rpc` /
  `codeless-client` may gain a `tokio::process` or `std::process`
  reach.
- R2: the UI talks to the new RPCs through `RpcClient` only. No
  direct `@tauri-apps/*` import in any Export/Import surface.
- R3: one responsive UI for every shell. No `Export.web.tsx`.
- R4: SQLite is the source of truth for the destination's Job/Run
  rows. The bundle is a transport, not a parallel store.
- R5: single-tenant trust boundary. The bearer token authorises
  every export and every import; no per-bundle ACLs.
- **Precondition**: JOB-WORKFLOW (B) — the Job/Run split — must
  already be merged on master. The bundle layout serializes per
  `runs/NNNN/` and assumes immutable Run rows with frozen
  `template_snapshot` and `handover_snapshot`. If (B) has not
  landed when this job starts, stage 1 must surface that and the
  job halts with `[!]`.
- MSRV 1.78. `cargo clippy --workspace --all-targets -- -D warnings`
  green. `cargo fmt --check` green.
- Never `--force`, never `--no-verify` (closing trio rule).

## Open questions (must be resolved in stage 1 or stage 2)

1. **Is JOB-WORKFLOW (B) merged?** If not, halt with `[!]` and
   write a note for the operator.
2. **What's the size cap default?** Scope doc proposes 200 MB
   per bundle, 10 MB per entry. Confirm against any existing
   per-workspace setting infrastructure; if there's nothing to
   hang it off, hardcode the defaults and leave a TODO.
3. **`output_path` resolution.** Server-side absolute paths only?
   Or jailed under the active workspace's `fs_root`? Default
   to the latter for safety and require the user to type a path
   under the workspace.
4. **JSONL ordering for `events.jsonl`.** Strict ascending
   `event_id` (source-workspace scoped); importer rewrites IDs in
   the destination's sequence and records the original under
   `original_id`. Confirm the runtime event id is monotonic.
5. **Where the imported handover lands on disk.** New Job's
   worktree doesn't exist until the first Run; the bundle's
   `handover.md` must seed the next Run's prompt prefix. Decision:
   store the imported handover on the Job row (per JOB-WORKFLOW
   (B) `handover_md` column); the first new Run snapshots it like
   any other.
