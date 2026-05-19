## Done

- Widened `crates/codeless-runtime/src/auto_bypass_policy.rs` `policy_comment` signature to `(policy, prior: Option<&PriorFailure>) -> String`; `None` reproduces prior byte-exact behaviour, `Some` appends a triple-backtick fenced block with `Previous-stage failure: <kebab-wire-name>` and an optional `Detail:` line.
- Added `PriorFailure { class: FailureClass, detail: String }` (pub) and a `failure_class_wire_name` helper covering all 7 variants.
- Prompt-side detail normalisation: trailing whitespace stripped, capped at 400 chars (`chars()`), `…` (U+2026) single-char ellipsis; empty/whitespace-only detail omits the `Detail:` line.
- 12 unit tests in `auto_bypass_policy::tests`: pre-check-failed+detail, review-fail+detail, `None` → bare policy text, single-paragraph invariant, custom-verbatim, empty/whitespace-only detail, trailing-whitespace strip, truncation marker, exact-ceiling no-truncation, and a `serde_json` round-trip pinning the wire-name table.
- Updated the one callsite in `crates/codeless-runtime/src/template_runner.rs` (around line 2044) to pass `None`.
- `cargo test -p codeless-runtime --lib auto_bypass_policy`: 14 passed. `cargo clippy -p codeless-runtime --all-targets -D warnings` clean. `cargo fmt` clean.
- Committed as `53a83ac` "stage 7: policy_comment threads prior-stage failure block" on branch `codeless/auto-bypass-hardening`.

## Next

- Stage 8: in `StageRecorder` / the bypass-comment-build path inside `template_runner.rs`, load the prior stage row's `failure_class` + `failure_detail` and pass `Some(&PriorFailure { class, detail })` into `policy_comment` instead of the current `None`. Add the MockRunner integration test asserting next-stage `prompt_prefix` carries the fenced block.

## What you need to know

- Signature widened from `-> &str` to `-> String`; the only callsite (`template_runner.rs` ~line 2044) was updated and the `.to_string()` shed.
- `PriorFailure` is exported as `pub` so stage 8 can build one from a stage row outside this module without crate-private gymnastics.
- The wire-name table is the source of truth for the prompt; the serde-roundtrip test will trip if a `FailureClass` variant rename ever drifts the prompt from the events stream.
- Did not use `mani` for the commit — this is an isolated worktree without `bin/mani`; per the harness handover instructions, plain `git commit` is the correct path here.

## Open questions

- (none)
