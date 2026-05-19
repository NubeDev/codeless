## Done

- Re-verified the (B) + missing-source chain (no `runs` migration, no `crates/codeless-runtime/src/job_export/`, no `ExportJob*`/`ImportJob*`/`InspectJobBundle` wire types) and confirmed stage 7 has nothing to test against.
- Appended a stage 7 halt entry to `DOCS/sessions/2026-05-19-job-export.md` documenting why round-trip + tar-safety tests cannot land, plus the file-by-file plan for re-fire (`tests/job_export_roundtrip.rs` proptest skeleton; per-guard tar-safety units beside `tar_safety.rs`).
- Rewrote `handover.md` to capture the stage 7 halt for the next session.
- Committed (`fc72be6`) with message `stage 7: ... [!] blocked on JOB-WORKFLOW (B) + missing stage-4/5/6 chain` and pushed to `codeless/job-export`.

## Next

- Stage 8 (mid-job REVIEW). Per WORKFLOW.md the REVIEW needs a manifest sample, a round-trip transcript, and tar-safety results — none of which exist. Operator should not advance until stages 4 + 5 + 6 + 7 have landed real source.

## What you need to know

- Five consecutive stages (1, 4, 5, 6, 7) have now halted on the same chain. The loop is burning a session per fire for zero forward motion. The operator-side fix is to gate stage firing on `[ -d crates/codeless-runtime/src/job_export ]` or on the (B) migrations grep.
- Branch `codeless/job-export`; worktree `/home/user/.codeless/worktrees/job-01KRZR5E2X039M469HKZ0NQMBM`. Halt commits in this job have used raw `git` because no `mani.yaml` exists at the inner-repo root; first commit of real code should switch to `./bin/mani --config mani.yaml run commit/push --projects codeless` from the workspace root.
- The exact stage 7 re-fire plan (proptest shape, ordering invariants, six tar-safety rejection variants, columns to skip on the round-trip equality check) is recorded in the session doc under "Stage 7 — round-trip property test + tar-safety units `[!]` halted on same chain".

## Open questions

- Who owns JOB-WORKFLOW (B) and what is its ETA? Stages 7–10 are dormant until it merges and stages 4 + 5 + 6 produce source.
- Will the operator gate further stage firing on the precondition, or keep accepting halt-shaped commits?
