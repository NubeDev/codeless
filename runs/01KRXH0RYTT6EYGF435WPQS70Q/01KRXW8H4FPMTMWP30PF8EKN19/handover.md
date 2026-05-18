## Done

- Added rehydration scan + supersede walk in `crates/codeless-runtime/src/supervisor/mod.rs::run_with_tools`: on boot, fetch the Job row, if terminal supersede every armed goal via a new `supersede_goal` helper (chat note + `mark_superseded`); otherwise re-arm `deadline-stop` rows and leave `threshold-stop` / `event-notify` rows armed for a future supervisor version.
- Added `supervisor_e2e::supervisor_rehydrates_deadline_after_restart` (abort the first supervisor mid-Run, spawn a fresh one, `tokio::time::pause` + `advance` past the deadline, assert the goal fires with the same `authorised_by` / `goal_id` metadata edge) and `supervisor_e2e::supervisor_supersedes_armed_goals_when_run_is_terminal_at_boot` (Job flipped to `Completed` pre-boot → supervisor walks the row to `superseded` with a chat note that names the goal id).
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test -p codeless-runtime --test supervisor_e2e` all green (9/9 tests).
- Committed as `d00e157` on `codeless/job-chat`.

## Next

- Stage 19 (16 of 21) per JOB-CHAT.md — next session picks it up.

## What you need to know

- The `bin/mani` tool referenced by CLAUDE.md is not present in the isolated worktree, so the commit used plain `git commit` (consistent with the existing stage history on this branch). Push was not attempted — the worktree has no remote / network access wired here.
- Pre-existing failure in `cargo test -p codeless-runtime --test migrations::migrator_creates_all_tables_from_appendix_a` reproduces on `HEAD~1` too — the test's expected table list was never updated when `supervisor_goals` landed in stage 13/16. Not caused by this stage; flag for a future micro-fix if no later stage already addresses it.
- The supersede "reason" lives in the chat thread (the supervisor's only voice), not in a new SQL column — `supervisor_goals` schema is unchanged.
- v0.1 still re-arms `deadline-stop` only; the rehydration loop traces a debug line and skips `threshold-stop` / `event-notify` rather than superseding them, since their signal sources land in a later stage.

## Open questions

- (none)
