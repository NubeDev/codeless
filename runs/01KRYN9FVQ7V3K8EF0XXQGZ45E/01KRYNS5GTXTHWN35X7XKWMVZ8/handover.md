## Done

- Added `FailureClass::InfrastructureError` variant in `crates/codeless-types/src/stage.rs` between `RunnerError` and `ReviewPatchInvalid` with a one-line doc comment that explains *why* (operator contract: SQLite disk-full / IOERR / CANTOPEN / CORRUPT / READONLY halt rather than being silently retried), per CLAUDE.md R2.
- Added the `infrastructure-error` arm to both `failure_class_label` and `parse_failure_class` in `crates/codeless-runtime/src/store/codec.rs`.
- Regenerated all three specta-derived TS snapshots: `crates/codeless-types/tests/wire.ts.snap`, `crates/codeless-rpc/tests/wire-rpc.ts.snap`, and `ui/codeless-ui/src/lib/rpc/generated/wire.ts`.
- `cargo test -p codeless-types` and `cargo test -p codeless-rpc` both green. `cargo check -p codeless-runtime` green.
- Committed as `a36b8fa` on `codeless/auto-bypass-hardening` with message starting `stage 2: codeless-types - add FailureClass::InfrastructureError`.

## Next

- Stage 3: sqlx error -> FailureClass mapper at every existing `RunnerError` emit site in `template_runner.rs` (lines 1051, 1494, 1579 per SCOPE.md), classifying the five primary SQLite codes from SCOPE.md Q1 (8 READONLY, 10 IOERR, 11 CORRUPT, 13 FULL, 14 CANTOPEN) to `InfrastructureError`. The matcher inspects the primary `code()` on `sqlx::error::Error::Database(_)` only — extended codes ignored, non-`Database` `sqlx::Error` stays in `RunnerError`. Stage 4 then short-circuits `classify_stage_failure` to `FailureAction::Halt` for the new variant.

## What you need to know

- The wire generator commands the project uses: `SPECTA_UPDATE=1 cargo test -p codeless-types --test specta_snapshot` (types snapshot) and `SPECTA_UPDATE=1 cargo test -p codeless-rpc` (rpc snapshot), then `cargo run -p codeless-rpc --example wire_ts` to write `ui/codeless-ui/src/lib/rpc/generated/wire.ts`.
- The sibling vendored `ai-runner/` Cargo.toml at `/home/user/.codeless/worktrees/ai-runner/Cargo.toml` had a stale `workspace = "../job-01KRXH0RYTT6EYGF435WPQS70Q"` pointer to a previous worktree. I repointed it to `../job-01KRYN9FVQ7V3K8EF0XXQGZ45E` so `cargo` could resolve this worktree's workspace. That file lives outside this git repo so the edit is not in the commit, but it is a load-bearing change on shared disk — the next session in a different worktree will need to repoint it again.
- Two tracked `.snap.actual` files exist (`crates/codeless-types/tests/wire.ts.snap.actual`, `crates/codeless-rpc/tests/wire-rpc.ts.snap.actual`) — they are stale from a prior unrelated session and unmodified by this stage.

## Open questions

- (none)
