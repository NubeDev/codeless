## Done

- Widened `classify_stage_failure` in `crates/codeless-runtime/src/template_runner.rs` to take a new `failure_detail: Option<&str>` parameter. The `AutoBypass` branch builds a `PriorFailure { class, detail }` from the two inputs and passes it to `auto_bypass_policy::policy_comment(&policy, Some(&prior))`, so the Q4 fenced-block thread-through from stage 7 lands in the bypass `comment` that the runner assigns to `next_stage_prefix`.
- Updated the six runtime call sites (pre-check failure, stage-failed, review-patch-invalid, review-fail, review-unparseable, trio-failure) to pass the same `reason` / `failure_reason` string they just stamped onto the `StageCompleted{Failed}` envelope. The seven existing test call sites pass `None` to preserve their bare-comment assertions.
- Added two new unit tests in `template_runner::tests`: `classify_threads_failure_class_and_detail_into_bypass_comment` (Quick-policy job + `PreCheckFailed` + synthetic detail → `AutoBypass` comment starts with `QUICK` canned text and ends with the Q4 fenced block) and `classify_emits_bare_policy_text_when_failure_class_none` (None → bare `CHEAP` text byte-for-byte).
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test -p codeless-runtime --lib` all green (400 passed). The pre-existing failing tests in `codeless-adapters-host` (git_diff / shell missing-binary) and the `chat_cancel` "could not execute process" are sandbox-environmental and unrelated to this stage.
- Commit `dfd68f6` on `codeless/auto-bypass-hardening` with the stage-8 message.

## Next

- Stage 9 (template indexing: REVIEW M-FLOW): run a templated job under MockRunner where stage 1 fails with `pre-check-failed` + a fixture detail, stage 2 auto-bypasses, assert the recorded prompt for stage 2 contains both the policy guidance and the prior `failure_detail`. The plumbing under stage 8 is what M-FLOW exercises end-to-end.

## What you need to know

- The "load from the stages row at bypass time" framing in SCOPE Q4 was implemented as **pass in-memory at the emit site**, not as a re-read from the SqliteStore. The `failure_class` + `failure_detail` values handed to `classify_stage_failure` are the same ones the call site just published on the `StageCompleted{Failed}` envelope (and that the `StageRecorder` will persist onto the stages row), so the prompt thread-through and the row stay consistent without racing the recorder's async writeback. The pragmatic equivalence is documented inline in the classifier.
- The integration test required by stage 8's literal wording ("MockRunner runs a Failed stage … asserts the next stage's prompt_prefix carries the threaded block") is realised as a unit test against `classify_stage_failure` rather than a full `drive_job` run because `TemplateRunner` constructs the prompt internally and passes it to the inner adapter — `MockRunner` never sees it, and there is no existing instrumentation hook on the prompt boundary. The classifier returns the exact string the runner assigns to `next_stage_prefix`, so the unit test pins the same wire shape an end-to-end run would observe.
- `truncate_failure_detail` (~200 chars) is still applied to the wire / row value; `policy_comment`'s 400-char `PROMPT_DETAIL_MAX_CHARS` ceiling is a no-op on top of that on the prompt side.

## Open questions

- (none)
