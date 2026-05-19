## Done

- Re-verified the JOB-WORKFLOW (B) precondition is still unmet: `crates/codeless-runtime/migrations/` unchanged at 27 files, no `runs` / `events.run_id` / `jobs.handover_md` migration; `crates/codeless-runtime/src/job_export/` does not exist; `codeless-rpc/src/methods.rs` has no `ExportJob` / `ImportJob` / `InspectJobBundle` types. The two intervening commits `92e844c` (`stage 3:`) and `a579c7e` (`stage 4:`) are both session-doc-only per `git show --stat`.
- Appended "Stage 6 — RPC wiring `[!]` halted on (B) + stage-4/5 chain" to `DOCS/sessions/2026-05-19-job-export.md` with four concrete blockers (no runtime entrypoints, no wire types, no Run to check, no `ImportError` variants) and an eight-step file-by-file plan for the agent that re-fires the stage.
- Rewrote `handover.md` for the next session.
- Committed via raw `git` (no `mani.yaml` in this inner-repo worktree) as `stage 6: wire export_job, import_job, inspect_job_bundle RPCs ... [!] blocked on JOB-WORKFLOW (B) + missing stage-4/5 chain` (`3907656`).

## Next

- Do not re-fire stage 6 until (a) JOB-WORKFLOW (B) is merged, (b) stages 4 + 5 have actually produced `crates/codeless-runtime/src/job_export/{manifest,walker,serializer,tar_writer,limits,denylist,tar_safety,inspect,importer}.rs`, and (c) `cargo test --workspace -p codeless-runtime` is green. The session-doc plan is the checklist.
- Stages 7 (round-trip property test), 8 (REVIEW), and 9–10 (UI) are all transitively blocked on the same chain.

## What you need to know

- Branch `codeless/job-export`; worktree `/home/user/.codeless/worktrees/job-01KRZR5E2X039M469HKZ0NQMBM`. Halt commits in this job have used raw `git` (no `mani.yaml` in the inner-repo worktree). The first real-code stage should switch to `./bin/mani --config mani.yaml run commit/push --projects codeless` from the workspace root.
- Design lock for the wire types is `.codeless/jobs/job-export/BUNDLE-DESIGN.md` §§3, 6 plus `SCOPE-JOB-EXPORT.md` §"RPC surface". For E1 only `ImportConflictPolicy::Refuse` is wired; `Suffix` / `Replace` should return `ImportError::UnsupportedConflictPolicy`.
- The "refuse non-terminal Run" check belongs in the export RPC wrapper (RPC seam), not the walker — it reads `runs.status` for the latest ordinal of the target Job.

## Open questions

- Six halt-shaped stages have now fired against the same unresolved (B) precondition (stages 1, 4, 5, 6 plus two doc-only "3"/"4" duplicates). Operator should gate stage firing on `[ -d crates/codeless-runtime/src/job_export ]` or on the migrations grep until (B) lands; each fire burns a session for zero forward motion.
- Who owns JOB-WORKFLOW (B) and what is the ETA?
