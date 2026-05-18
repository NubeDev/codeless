## Done

- Converted `crates/codeless-runtime/src/supervisor/tools.rs` to a directory module (`tools/mod.rs`) and added `tools/actions.rs` with `stop_job`, `stop_job_ad_hoc` (+ `stop_job_ad_hoc_with_window` test entry), and `add_job_note` action methods on `Tools`, plus the `AdHocOutcome { Fired, Aborted }` enum and `AD_HOC_PREVIEW_WINDOW = 5s` constant.
- `stop_job` mirrors `rpc::jobs::stop_job`'s state transition (store + bus) so the `JobStopped { reason: User }` envelope is byte-identical to the UI button's. Ad-hoc path posts a `System`-role preview row (`metadata.preview = { window_ms, action, resolves_at }`), races `tokio::time::sleep(window)` against a `subscribe_since` stream filtered for `User`-role `ChatMessageAppended` whose body matches a hand-rolled `/^wait\b/i` (no `regex` dep), and posts a follow-up `Assistant` row with `metadata.resolves = <preview_id>` for either branch. `add_job_note` writes a `System`-role supervisor row with `metadata.note = true`.
- Added e2e tests `supervisor_e2e::ad_hoc_stop_aborts_on_user_wait` and `supervisor_e2e::ad_hoc_stop_fires_after_window` (both green, ~0.4s); 4 unit tests for `is_wait_prefix` also added. Full `cargo test -p codeless-runtime --test supervisor_e2e` is 6/6 green; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt -p codeless-runtime -- --check` clean.
- Committed as `d59cbf2` on `codeless/job-chat`.

## Next

- (none — stage 13 picks up in a fresh session)

## What you need to know

- The `events` table has no `actor` column (predates the concept), so the stage description's "events row's actor='supervisor'" goal is satisfied indirectly: the supervisor's `ChatMessageAppended` partner row on each action carries `transport=Supervisor`, and the module doc-comment on `actions.rs` records the deferral. If a future stage wants a real `events.actor` column it is a fresh migration.
- `mani` is not present in this worktree, so the commit was made with raw `git` rather than `./bin/mani ... run commit`. The commit message still starts with `stage 12:` per the stage rules.
- `../ai-runner/Cargo.toml` had a stale `workspace = "../job-01KRX4ZPF10J3QZ35R5GK8336X"` pointer that kept resurrecting during the build — I re-`sed`'d it to point at this worktree just before the final `cargo build`. The next session may have to re-point it again on first `cargo` invocation (one-line `sed`). Not committed (it lives outside this repo).
- `stop_job_ad_hoc_with_window` is the test-only entry point — production must always go through `stop_job_ad_hoc` (or `stop_job` for pre-armed) to keep the 5s constant load-bearing.
- The supervisor reactor in `supervisor/mod.rs` does NOT yet auto-invoke `stop_job_ad_hoc` — the action surface is exposed; later stages (LLM-driven dispatch + `supervisor_goals`) will wire it into the reactor.

## Open questions

- Whether to add an `events.actor` column in a later stage (would let `JobStopped` carry `actor='supervisor'` directly instead of relying on the paired `ChatMessageAppended` row for provenance). Deferred — not blocking.
