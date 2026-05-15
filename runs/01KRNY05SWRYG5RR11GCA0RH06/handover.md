## Done

- verified H1/H3/H7 handover correctness on branch (commit 3cf98e4 already shipped per-stage keying, keyed find_handover, and write-time validate_handover); no gap found in the runtime surface
- added symmetry coverage for the Next-side H7 floor in crates/codeless-runtime/src/handover.rs: `validate_rejects_whitespace_only_next` (parallel to the existing whitespace-only Done test) and `write_handover_rejects_empty_next_without_touching_fs` (parallel to the existing empty-Done write-side test), closing the asymmetric test gap where only Done had write-time-rejection coverage

## Next

- start stage 5: Step 1 — REVIEW as a real blocking stage type in `crates/codeless-runtime/src/template_runner.rs`. Parse PASS/FAIL sentinel from the assistant's last turn; reuse the existing `Review*` event family (no new `ReviewGate*` variants — see DECISIONS "Event naming"); add the WORK-cannot-touch-rule-bearing-files Layer-1 file-set rule per JOB-LOOP.md "Rule-bearing files".

## What you need to know

- H1/H3/H7 surface lives entirely in `crates/codeless-runtime/src/handover.rs`: `handover_path`, `validate_handover`, `write_handover`, and `find_handover` are the four functions that encode the floor; call sites are `claude_runner.rs` (write on stage completion), `job_driver_loop.rs` (find when prefixing the next prompt), and `rpc/job_files.rs` (the UI seeding RPC). Other runners (anthropic, codex, copilot, mock, template, verify) deliberately do not write handovers — they have no accumulated assistant-text buffer to extract from
- `find_handover` scans `<repo>/.codeless/worktrees/*/runs/<job_id>/<stage_id>/handover.md`; the per-job filter is implicit in the path, so a different job's worktree never matches even though its directory is iterated
- pre-existing flake on `crates/codeless-runtime/tests/rpc_in_process.rs::job_filtered_subscription_drops_unrelated_events` (Conflict: "repo already in use by job in in_repo mode") fails on baseline `381fe87` with the same error — not introduced by this stage; out of scope to fix here
- the ai-runner crate at `../ai-runner/Cargo.toml` is pinned via `workspace = "../job-<ULID>"` to a specific worktree path; running `cargo test` from inside a fresh worktree fails until that path is repointed. Treat ai-runner as read-only per CLAUDE.md vendoring rules — repoint locally only if needed, do not commit the change

## Open questions

- (none)
