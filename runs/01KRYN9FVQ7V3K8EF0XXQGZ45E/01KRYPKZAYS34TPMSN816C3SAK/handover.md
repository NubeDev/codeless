## Done

- `codeless-types::StopReason::Infrastructure` variant added with kebab-case wire name `infrastructure`, doc comment per CLAUDE.md R2.
- `store/codec.rs::stop_reason_label` + `parse_stop_reason` round-trip the new variant.
- `template_runner.rs::classify_stage_failure` widened to `(store, ctx, failure_class)`; short-circuits to `FailureAction::Halt` when `failure_class == Some(FailureClass::InfrastructureError)`, regardless of `auto_bypass_policy`. The halt branch stamps `StopReason::Infrastructure` via the new `record_infrastructure_halt` helper.
- Six call sites of `classify_stage_failure` (pre-check, runner-outcome-failed, review-patch-invalid, review-fail, review-unparseable, trio-gate-failed) pass the `FailureClass` that just landed on the `StageCompleted{Failed}` envelope.
- New unit tests `classify_halts_on_infra_error_even_under_relentless` and `classify_auto_bypasses_runner_error_without_infra_classification` pin the two-branch contract; the five pre-existing classify tests were updated to pass `Some(FailureClass::RunnerError)`.
- bot-core `stop_reason_word` in both `reply.rs` and `notify.rs` carries the new variant as `infrastructure failure`.
- Specta snapshots regenerated for both `codeless-types/tests/wire.ts.snap` and `codeless-rpc/tests/wire-rpc.ts.snap`.
- Session doc `DOCS/sessions/2026-05-19-auto-bypass-hardening.md` carries a stage-4 handover paragraph for the M-INFRA REVIEW gate.
- Committed as `a96b707` on `codeless/auto-bypass-hardening` with the stage-4 title.

## Next

- Stage 5: M-INFRA REVIEW gate per the job template. Sanity-check the layering order in `classify_stage_failure` (cancel → no-store → row-load → `stop_reason.is_some()` → infra → policy match), confirm the warn-only stamp posture matches `record_thrash_halt`, and verify that `classify_runner_failure_reason` (from stage 3) is the only producer of `FailureClass::InfrastructureError`.

## What you need to know

- `cargo test --workspace --lib --tests --exclude codeless-server`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` all green at HEAD.
- `codeless-adapters-host` git-binary tests are flaky on first run (file-lock contention) but pass on retry; not introduced by this stage.
- Two `.snap.actual` files (`codeless-rpc`, `codeless-types`) are tracked in git. The `codeless-rpc` one was stale and is now back in sync with `.snap`; the `codeless-types` one was already in sync. Both are unchanged or aligned after `SPECTA_UPDATE=1`.
- Committed via plain `git`, not mani. Mani isn't reachable from this isolated worktree, and the job-loop CLAUDE.md rule only binds when an interactive loop is driving; prior stages on this branch also committed via plain git.

## Open questions

- (none)
