## Done

- Added `crates/codeless-runtime/src/scoped_pause_hook.rs` exposing `TransitionPoint`, `check_and_pause`, and `check_trio`; matcher covers every `TodoSelector` variant.
- Added `StopReason::ScopedPausePoint { point_id: PausePointId }` to `codeless_types::job`. `StopReason` stays `Copy`; the SQLite codec encodes it as `scoped-pause-point:<ulid>` and parses the prefix back.
- Wired four hook call sites: `BeforeStage` + `AfterStage` in `template_runner.rs` (top/bottom of per-stage loop), `BeforeTodo` + `AfterTodo` in `trio_emitter` (around the trio publish).
- Bot-core `notify`/`reply` `stop_reason_word` grew a `ScopedPausePoint { .. }` arm.
- Six new unit tests in `scoped_pause_hook::tests` and three integration tests in `crates/codeless-runtime/tests/scoped_pause_hook.rs` (pause→resume cycle, idempotency, non-match no-op).
- Updated specta snapshots (`wire-rpc.ts.snap`, `wire.ts.snap`), `handover.md`, `DOCS/SCOPED-PAUSE-POINTS.md` (drops the `label` field from §"existing primitive"), and SCOPE.md deliverables.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` all green.
- Committed as `ae42a58` on `codeless/scoped-pause-points`.

## Next

- Stage 7 (REVIEW gate) — server-side complete; do not start stage 8 (UI) without explicit approval.

## What you need to know

- The handover doc / SCOPED-PAUSE-POINTS.md design said `StopReason::ScopedPausePoint { point_id, label }`; the actual variant carries only `point_id` because adding a non-Copy `String` would have rippled through ~40 `StopReason`-Copying call sites. The label is reconstructed at render time from the `scheduled_pause_points` row (which already carries `reason`).
- The `BeforeTodo` / `AfterTodo` hooks fire only for the closing trio today — the runner-authored todo path (claude_runner's TodoWrite tool calls) doesn't yet flow through the hook. Substring/ordinal targets against runner todos will need a follow-up in stage 8 or after, once a UI surface exercises that path.
- Push via `git push` was not performed — `bin/mani` was not present at the workspace root in this worktree environment; the workflow's mani-push step is the next reviewer's call.
- The `transition_job` guard rejects `Paused → Paused`, so the hook is naturally idempotent (returns `HookOutcome::Continue` on a second call against a paused row).

## Open questions

- Whether `fired_at` / `superseded_at` columns from SCOPED-PAUSE-POINTS §4 should land before the UI stage so the divider chip can show "already fired" state, or stay deferred as stage-5's handover suggested.
- Whether the runner-authored todo hook should be wired in stage 8 (so the UI's Playwright test can exercise `~migrate`) or split into its own follow-up.
