## Done

- Added migration `0022_scheduled_pause_points.sql` (table keyed on `(job_id, ordinal)`, FK `job_id` → `jobs(id)` ON DELETE CASCADE, `target_json` carries the serde-shape `PausePointTarget`)
- Added `crates/codeless-runtime/src/store/scheduled_pause_points.rs` with idempotent `replace_scheduled_pause_points` (DELETE + bulk INSERT in one tx) and `list_scheduled_pause_points` (YAML order); 7 async tests
- Wired `resync_template_from_disk` (`rpc/jobs.rs`) and `update_job_template` (`rpc/job_files.rs`) to call a shared `rebuild_scheduled_pause_points` helper; resolution failures surface as `RpcError::InvalidArgument` with the full punch list
- Updated `crates/codeless-runtime/tests/migrations.rs`: added `scheduled_pause_points` to Appendix A allow-list and a new test pinning columns / index / FK cascade
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` all green
- Updated `handover.md` for stage 6
- Committed as `stage 5: persistence: …` on `codeless/scoped-pause-points` (`e973b72`)

## Next

- Stage 6 (runtime hook): read the schedule via `SqliteStore::list_scheduled_pause_points(job_id)` at the trio-gate transition point, call existing `pause_job` with the new `StopReason::ScopedPausePoint { point_id, label }` variant (this variant lands in stage 6, not here)

## What you need to know

- Stage 5 is the persistence layer only. No `fired_at` / `superseded_at` columns yet — stage 6 owns that follow-up if it needs the question-3 resync silencing semantics
- `PausePointId` is freshly minted by the parser on every `resolve_pause_points` call, so the runtime hook cannot rely on id stability across a resync; the (job_id, ordinal) key is the stable handle
- I had to retarget `/home/user/.codeless/worktrees/ai-runner/Cargo.toml`'s `workspace = "../job-…"` pointer to this worktree so the workspace would build; that file lives outside the codeless repo and is uncommitted — the next worktree session will likely need to do the same dance
- Commit/push via mani was not used because the workspace `bin/mani` flow operates from the outer `codeless-workspace` root which isn't present in this worktree; the `stage 5: …` commit lives on `codeless/scoped-pause-points` but has not been pushed

## Open questions

- Should the rebuild emit a dedicated `pause_points_updated` event (called out in SCOPE.md §4 / R4) so the UI does not have to re-derive the schedule from `JobTemplateUpdated`? Deferred — currently the `JobTemplateUpdated` event already fires after the rebuild, and the UI stage (stage 8) can read the rows in response
