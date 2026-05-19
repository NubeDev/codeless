## Done

- Re-verified stage 10's preconditions are still unmet: no `runs` migration, no `crates/codeless-runtime/src/job_export/`, no `imported_from` column or wire field, no server-side `ExportJob`/`ImportJob`/`InspectJobBundle` types.
- Appended a full "Stage 10 — imported-from chip + manifest viewer + ordinal verification `[!]` halted on same chain" section to `DOCS/sessions/2026-05-19-job-export.md` covering re-verification commands, why each of the four deliverables cannot land, the file-by-file plan for re-fire, and the open questions.
- Rewrote the top-level `handover.md` for stage 10, including the recommendation that the operator either merge (B) and re-fire stages 4 → 10 in order or revert stages 8 + 9's premature UI commits.
- Committed as `268adcb` with the stage title prefix and `[!] blocked` suffix matching prior halts (1, 4, 5, 6, 7).

## Next

- (none) — this was the final stage in the job's 10-stage list. The job has now exhausted its scope with the foundation (JOB-WORKFLOW (B)) still unmerged.

## What you need to know

- Stages 8 and 9 landed real UI (`JobPage` Export button, `ImportJobDialog`, TS-only RPC method declarations) even though the server-side RPCs do not exist. The `ImportJobDialog` will get `MethodNotFound` until stages 4/5/6 actually ship. Stage 10 chose not to deepen that R4 violation.
- The `Job` wire type (`ui/codeless-ui/src/lib/rpc/generated/wire.ts:1677`) has no `imported_from*` fields; the chip + viewer have nowhere to read from on the destination Job row until a migration + types change lands.
- Six of ten stages (1, 4, 5, 6, 7, 10) halted on the same (B) + missing-source chain. Loop overhead was one full session per halt.

## Open questions

- Should stages 8 + 9's premature UI be reverted before any re-fire? Leaving it on the branch means a user importing a Job sees a dialog that returns `MethodNotFound`.
- Should future stage firing be gated on `[ -d crates/codeless-runtime/src/job_export ]` or on the `runs` migration grep so the next chain of dependent stages doesn't burn six sessions for no work?
