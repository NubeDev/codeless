## Done

- Extended `crates/codeless-runtime/src/template.rs` with a `pause_points: Vec<RawPausePoint>` field on `JobTemplate`, a `RawPausePoint` struct, and permissive `StageRef` / `TodoRef` deserializers (integer-or-string scalars; tilde-prefixed substrings; bare-word trio kinds).
- Added a typed `ScopeError` enum mapping 1:1 to `DOCS/SCOPED-PAUSE-POINTS.md` §3, plus `JobTemplate::resolve_pause_points() -> Result<Vec<PausePoint>, Vec<ScopeError>>` that resolves symbolic stage names to ordinals against the parsed stages, validates trio words, position keywords, ordinal floors, substring non-empty, reason byte-cap (512), and cross-point duplicates over the resolved key.
- Stage-name lookup strips trailing ` (S)` / `(M)` / `(L)` suffixes and takes the portion before the first colon, so titles like `"design: extend template.yaml (S)"` resolve to `design`.
- 22 new unit tests cover every happy and rejection path; resolver collects every violation in one pass (one test pins this).
- `cargo test -p codeless-runtime --lib template::` → 37 passed; `cargo fmt --check` clean; `cargo clippy -p codeless-runtime --lib --all-targets -- -D warnings` clean.
- Committed as `7d13451` on branch `codeless/scoped-pause-points` with the stage-title prefix.

## Next

- Stage 5 (persistence): add the `scheduled_pause_points` table keyed on `(job_id, ordinal)` and an idempotent rebuild on `resync_template_from_disk`. Write the resolver output verbatim — `PausePointId` is already minted per entry so the row can use it as the PK.
- Wire `resolve_pause_points()` into the submit path so the job genuinely refuses to leave `draft` on `Err(_)`. (Stage 4 lands the resolver; no caller invokes it yet.)

## What you need to know

- Workspace plumbing is broken in this worktree by default: `crates/codeless-server` depends on `ai-ui-core` at `/home/user/.codeless/ai-ui/...`, and `../ai-runner/Cargo.toml` pins `package.workspace` to a different job worktree. To compile I (a) symlinked `/home/user/.codeless/ai-ui -> /home/user/code/rust/ai-ui`, (b) temporarily flipped `/home/user/.codeless/worktrees/ai-runner/Cargo.toml`'s `workspace =` to this worktree, and (c) copied a newer `ai-runner/src/types.rs` from `/home/user/code/rust/codeless-workspace/ai-runner/` because the in-tree `claude_runner.rs` already uses a `mcp_config_path` field the vendored copy didn't have. The ai-runner manifest is reverted; the symlink and the patched `types.rs` remain in shared dirs outside the worktree.
- The committed change is one file, `crates/codeless-runtime/src/template.rs` (+919 lines); no other tracked files moved.
- `resolve_pause_points()` mints a fresh `PausePointId` per resolved entry — keep that contract when the persistence layer in stage 5 inserts rows; the resolver output is the canonical schedule.
- The pre-existing CliCfg mismatch in `claude_runner.rs:318` is unrelated to this stage and was not touched.

## Open questions

- None for this stage. The doc-pinned variant `AmbiguousTitleSubstring` is runtime-only (multi-match at bind time, not parse time), so it intentionally does not appear in `ScopeError`'s parse-time tests — stage 6 owns it.
