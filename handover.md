# job-export — stage 2 `[x]` design lock; stage 3 is the REVIEW gate

Stage 2 ("design bundle layout and manifest.json schema_version 1
against DOCS/SCOPE-JOB-EXPORT.md; lock the secrets denylist and
per-entry size caps") landed as paper-only. The frozen design lives
in [`.codeless/jobs/job-export/BUNDLE-DESIGN.md`](.codeless/jobs/job-export/BUNDLE-DESIGN.md).
The (B) precondition that halted stage 1 is still in effect — re-grep
this session confirmed no `runs` migration on disk — and continues to
block implementation stages 4–7. Stages 2 and 3 are paper and can
proceed.

## What stage 2 produced

- `.codeless/jobs/job-export/BUNDLE-DESIGN.md` — ten sections that
  freeze the bundle file shape, the directory layout, the
  `manifest.json` schema_version 1 field contract, the per-Run
  `run.json` field set + JSONL stream sort/drop rules, the secrets
  denylist regex set, the size cap constants table, the open-question
  resolutions (OQ-1…OQ-5 from `DOCS/SCOPE-JOB-EXPORT.md` plus OQ-D
  and OQ-E from this stage), the refuse-to-export preconditions, the
  README cover-note outline, and the inheritance contract for stages
  3–7.
- `DOCS/sessions/2026-05-19-job-export.md` — extended with the stage 2
  record including the (B) re-verification, the deliverable summary,
  and the locked answers to every open question.

No `crates/`, no `ui/`, no migrations touched. `cargo` not run.

## Locked answers (so stage 3 REVIEW has the diff in one place)

- **Size caps.** 200 MiB / bundle, 10 MiB / entry. Per-kind sub-caps
  for manifest, README, template, handover, notes, `run.json`, JSONL
  lines. 1024 Runs/bundle, 500k events/Run. Constants in
  `codeless-runtime/src/job_export/limits.rs` once stage 4 lands.
- **Output path.** Jailed under `attached_workspaces.fs_root_canonical`
  via canonicalised prefix check.
- **Events monotonicity.** SQLite `AUTOINCREMENT` confirms; (B)
  re-keys `events.job_id → events.run_id` only.
- **Handover destination.** `jobs.handover_md` (post-(B)); first new
  Run snapshots like any other.
- **Secrets denylist.** Case-insensitive substring regex set:
  `token`, `secret`, `api[_-]?key`, `pass(word|wd)`,
  `private[_-]?key`, `credential`, `bearer`, `auth[_-]?header`.
  Applied to column names only (payload contents not scanned in E1).
  Re-verified: zero current schema columns match.
- **`scheduled_pause_points`** and **payload-content scanning** are
  both **out for E1** — logged for E2 with rationale.

## What stage 3 (the REVIEW gate) must do

Stage 3 is read-only paper: the human reviews the design before stage
4 starts writing serializer code. Per
[`.codeless/jobs/job-export/WORKFLOW.md`](.codeless/jobs/job-export/WORKFLOW.md#review-gates)
the REVIEW packet must include:

1. The exact `manifest.json` shape — already in
   `BUNDLE-DESIGN.md` §3.
2. The bundle directory layout — `BUNDLE-DESIGN.md` §2.
3. The three RPC signatures — already in
   `DOCS/SCOPE-JOB-EXPORT.md` §"RPC surface"; the design doc does not
   restate them. Stage 3's handover should quote them so the
   reviewer has one place to look.
4. The secrets denylist — `BUNDLE-DESIGN.md` §4.
5. The conflict-policy enum with wired/stubbed notes —
   `DOCS/SCOPE-JOB-EXPORT.md` §"Conflict & rename rules"; for E1 only
   `Refuse` is wired end-to-end. Stage 3's handover should restate
   this.

Stage 3 itself produces only `handover.md` + an updated session doc
that present the above to the reviewer. No source files change. If
the reviewer requests changes, stage 3 amends `BUNDLE-DESIGN.md` and
records the diff in the session doc before passing stage 4.

## What stage 4 will need (after REVIEW approval) — still blocked

Stage 4 builds the walker + serializer in
`crates/codeless-runtime/src/job_export/` (one file per concept per
R3: `walker.rs`, `serializer.rs`, `manifest.rs`, `limits.rs`,
plus tests). It cannot start until JOB-WORKFLOW (B) merges; the
walker reads `runs`, the serializer writes
`template_snapshot` / `handover_snapshot` from the Run row, and the
round-trip test in stage 7 needs at least one immutable Run row in
the dev `codeless.db` to chew on.

The unblock checklist from the prior handover still applies; nothing
about stage 2's design changes it.

## Verify

- No source diff. New / updated docs only:
  `.codeless/jobs/job-export/BUNDLE-DESIGN.md` (new),
  `DOCS/sessions/2026-05-19-job-export.md` (appended), this
  `handover.md` (rewritten for stage 3).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test --workspace` — **not run** (no code
  touched).
- Per `WORKFLOW.md` closing trio for a read-only stage that produced
  no source diff: handover + session doc + design doc committed only.
