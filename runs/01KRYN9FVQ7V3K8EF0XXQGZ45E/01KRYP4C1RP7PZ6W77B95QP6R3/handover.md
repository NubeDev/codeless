## Done

- Added `classify_runner_failure_reason` + `extract_sqlite_extended_code` + `classify_sqlite_extended_code` helpers in `crates/codeless-runtime/src/template_runner.rs` (just above `truncate_failure_detail`).
- Wired the new classifier at all three `RunnerError` emit sites (formerly hard-coded `FailureClass::RunnerError` at lines 1051, 1494, 1579) so SQLITE_FULL / IOERR / CORRUPT / CANTOPEN / READONLY now classify as `FailureClass::InfrastructureError`.
- Mask is `extended_code & 0xff` so extended siblings (e.g. SQLITE_IOERR_FSTAT=1546 -> primary 10) collapse correctly.
- Two unit tests in `template_runner::tests` pin classification: one covers every code in the SCOPE Q1 infra set, the exclusion list (1,5,6,19,20,21,25,26 — incl. SQLITE_NOTADB stays RunnerError per the decision), extended-code mask behaviour, non-sqlx reasons, and malformed markers; the other constructs a real `sqlx::Error::Database` with a `DatabaseError` impl mirroring `sqlx-sqlite`'s Display format and asserts the round-trip through `format!("{err}")` works for SQLITE_FULL and SQLITE_CONSTRAINT.
- `cargo test --workspace --lib --tests --exclude codeless-server`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` all green.
- Committed as `stage 3: sqlx error -> FailureClass mapper at RunnerError emit sites` on `codeless/auto-bypass-hardening`.

## Next

- Stage 4: short-circuit `classify_stage_failure` to `FailureAction::Halt` for `FailureClass::InfrastructureError`, mirroring the existing `stop_reason.is_some()` branch. Halt must write a structured `stop_reason` so the UI labels it `Infrastructure` rather than the generic crash wording. SCOPE references `classify_stage_failure` around line ~1956 (now shifted by stage 3 insertions; grep by name, not by line).

## What you need to know

- The mapper is reason-string-based because the `RunnerError` emit sites only have `RunnerOutcome::Failed { reason: String }` to work with; the upstream runner adapters (`anthropic_runner`, `codex_runner`, `claude_runner`, `copilot_runner`) format any error into that string with `format!("{e}")`. sqlx-sqlite's `SqliteError` Display is `(code: <ext>) <msg>`, so parsing `(code: NNN)` and masking to the primary code is the contract; the helper returns `RunnerError` for any reason that doesn't carry the marker, which keeps existing failure semantics intact.
- The doc comment on line ~2178 (`/// stage failure under FailureClass::RunnerError`) is on `classify_stage_failure` and still accurate after stage 3 — stage 4 will need to amend it to mention the infra short-circuit.
- mani was not used for the commit because the worktree-headless job doesn't ship `bin/mani`; raw `git commit` matches what stages 1–2 in this branch did. If the JOB-LOOP supervisor needs mani-via-workspace, it pushes from the workspace root after the loop finishes.

## Open questions

- (none)
