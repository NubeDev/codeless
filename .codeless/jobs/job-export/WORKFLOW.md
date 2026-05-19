# Workflow — job-export

How the agent should drive the stages in
[`template.yaml`](./template.yaml). Read alongside
[`SCOPE.md`](./SCOPE.md) and the full design at
[`DOCS/SCOPE-JOB-EXPORT.md`](../../../DOCS/SCOPE-JOB-EXPORT.md).

## Sequencing

- Stages 1–3 are **paper work**: survey, design, REVIEW. No code
  lands before the first REVIEW.
- Stages 4–7 are the **server-side core**: walker, importer, RPCs,
  tests. Each ends with `cargo test --workspace` green plus
  `cargo clippy --workspace --all-targets -- -D warnings` green.
- Stage 8 is the **mid-job REVIEW** before any UI work — the
  bundle shape and RPC surface are the load-bearing decisions and
  the user wants a second look before they ossify into TSX.
- Stages 9–10 are the **UI**: Export button + Import dialog +
  imported-from chip. The UI imports `RpcClient` only.

Do not batch stages. The user is watching the `Stages` overview
tick over.

## Per-stage discipline

- **Read before writing.** Top of every stage: re-read
  `SCOPE.md`, `WORKFLOW.md`, the relevant section of
  `DOCS/SCOPE-JOB-EXPORT.md`, and any handover the prior stage
  left. The fresh agent that opens this stage has only what is on
  disk.
- **One concept per file** (R3 in repo `CLAUDE.md`). The
  `job_export/` module splits into `walker.rs`, `serializer.rs`,
  `manifest.rs`, `importer.rs`, `tar_safety.rs`, plus tests beside
  each. Resist the urge to add a `util.rs` grab bag.
- **Tests live with the code** (R5 in repo `CLAUDE.md`). Every
  new module ships unit tests in the same commit. The round-trip
  property test goes in `crates/codeless-runtime/tests/`.
- **Wire types first, then implementation.** When a stage touches
  `codeless-rpc`, add the typed arg/result struct + `specta::Type`
  derive before the runtime implementation. Regenerate the TS
  bundle in the same commit so the UI stages have something to
  import.
- **Mobile-safe crates stay mobile-safe** (R1). Process spawn,
  tar, gzip, filesystem walks belong in `codeless-runtime` or
  `codeless-adapters-host`. Do not pull them through
  `codeless-types` / `codeless-rpc` / `codeless-client`.

## REVIEW gates

Two gates:

1. **Stage 3 — bundle shape + RPC arg types.** The handover for
   the prior stage must include: the exact manifest JSON shape
   you intend to ship, the bundle directory layout, the three
   RPC signatures, the secrets denylist, and the conflict-policy
   enum with notes on what's wired vs. stubbed. The user reads
   this and approves or asks for changes before any serializer
   code lands.
2. **Stage 8 — server-side end-to-end.** Handover must include:
   sample manifest output from a real Job in the repo, a round-trip
   test transcript, the tar-safety test results, and the list of
   warnings the importer can surface. The user verifies behaviour
   before any TSX gets written.

REVIEW stages still commit + push the work that led to the gate.
They pause the *next* stage, not the *current* one.

## Precondition check (stage 1)

Stage 1's first action: confirm JOB-WORKFLOW (B) — the Job/Run
split — is merged. Grep for the `runs` table migration; if it does
not exist, halt the stage with `[!]` in the session doc and write
a handover explaining that this job is blocked on (B). Do not try
to scaffold around it; the bundle layout depends on immutable Run
rows.

## Anti-patterns specific to this job

- **No bundle-format YAML.** The manifest is JSON. The
  `template.yaml` inside the bundle is the user's file; everything
  else is JSON / JSONL so the importer doesn't carry a YAML parser
  for runtime data.
- **No streaming-uncompressed-out-of-tar shortcut.** The importer
  must enforce the path-traversal guards before opening any
  entry's contents. One tar entry escaping the layout fails the
  whole import — do not partially-apply rows and roll back later.
- **No "while we're here" refactor** (R4 in repo `CLAUDE.md`). A
  bug fix to the existing handover-pickup path is a separate job.
  This job adds export/import; it does not retouch unrelated code.
- **No silent column drops on import.** If the bundle's JSONL has
  a column the destination's table doesn't, fail the import with a
  schema-mismatch error and a clear `original_column` in the
  message. The schema_version exists to catch this; do not paper
  over it.
- **No new transport for the file bytes.** The browser shell
  reads the exported bundle via `fs.read_file` after `export_job`
  returns the path; the desktop shell uses the native picker. No
  WebSocket binary frame, no SSE base64 blob.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run `cargo test --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, and
   `cargo fmt --check`. Every step must pass. On failure: stop,
   fix, re-run; do not advance to `docs`.
2. `docs` — update `handover.md` for the next stage and the active
   session doc, in the same worktree, so the fresh agent that
   opens the next stage has the context it needs (per
   `DOCS/SCOPE.md` Constraint 2: anything that must survive a
   stage boundary is on disk, not in the agent's head).
3. `git` — stage the changes (`git add -A` from the worktree
   root, or specific paths if the stage was surgical), commit
   with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one, and push to the job's branch
   (`codeless/job-export`) so the work is recoverable even if
   the worktree is wiped.

A stage is not "done" until all three todos are green and the
push succeeds. If `checks` or `git` fails, fix the cause and
retry — do not mark the stage `[x]`, do not advance, and never
`--force` or `--no-verify`. A read-only stage (stage 1, stage 2,
stage 3 REVIEW, stage 8 REVIEW) that produced no source diff
commits `handover.md` only and marks `git` as
`committed handover.md only`.
