## Done

- Re-verified JOB-WORKFLOW (B) precondition: no `runs` migration, no `runs.template_snapshot` / `runs.handover_snapshot` / `jobs.handover_md` / `events.run_id`. Status unchanged since stage 1's halt.
- Appended a "Stage 4 — walker + serializer `[!]` halted on (B)" section to `DOCS/sessions/2026-05-19-job-export.md` covering: re-verification grep, three concrete reasons stage 4 cannot scaffold around (B), and a full file-by-file plan for the agent that picks this up post-(B) (`mod.rs`, `limits.rs`, `denylist.rs`, `manifest.rs`, `walker.rs`, `serializer.rs`, `tar_writer.rs`).
- Committed as `stage 4: ... [!] blocked on JOB-WORKFLOW (B)` (27dca3d). Session-doc only; no source touched, no cargo run.

## Next

- Stage 4 must be re-run after JOB-WORKFLOW (B) lands (the `runs` migration + `events.run_id` re-key + `jobs.handover_md`). The handover-time plan in the session doc is the checklist.
- Stage 5 (importer), 6 (RPCs), 7 (round-trip tests) remain blocked on the same precondition.
- Stage 3 (REVIEW gate on bundle shape + RPC arg types) was skipped by the loop runner before this stage fired — flag for the operator. Per WORKFLOW.md the user should approve `BUNDLE-DESIGN.md` + the RPC arg structs before stage-5+ code lands.

## What you need to know

- Worktree path: `/home/user/.codeless/worktrees/job-01KRZR5E2X039M469HKZ0NQMBM`, branch `codeless/job-export`.
- `mani` and `mani.yaml` are not present inside this worktree (we're in the inner repo only), so the commit used raw `git` matching the stage-1 / stage-2 pattern in the log. If a future stage lands code, switch to `./bin/mani --config mani.yaml run commit --projects codeless` from the workspace root per CLAUDE.md.
- Locked design lives in `.codeless/jobs/job-export/BUNDLE-DESIGN.md` (stage 2). §§2–5, 7 are the load-bearing references for stage 4 once (B) lands.
- The session doc commit was deliberately session-doc-only to avoid violating R4. No tests run because no Rust files changed.

## Open questions

- Is there a parallel job working on JOB-WORKFLOW (B), and what's its ETA? Without that, stages 4–7 of `job-export` are dormant.
- Should the loop runner be told to stop firing implementation stages 4–7 until (B) is in, rather than re-firing each stage to halt? (Each fire still costs a session.)
