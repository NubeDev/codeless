## Done

- Created `crates/codeless-runtime/src/supervisor/mod.rs` with `spawn_supervisor(bus, job_id) -> JoinHandle<()>`. The task subscribes to `SubscribeFilter::Job(job_id)`, observes `ChatMessageAppended` (no-op scaffold), and self-exits on `JobCompleted` / `JobFailed` / `JobStopped`. `AwaitingReview` and `JobPaused` are non-terminal here, per stage 9 wording.
- Re-exported `spawn_supervisor` from `codeless_runtime` and wired the spawn into `driver::drive_job` immediately after the `JobStarted` publish, before the cap-watcher / runner kick off. R1 holds: the module imports only `codeless_types`, `event_bus`, `futures_util`, `tokio` — no `std::process` / `tokio::process`.
- Added a cargo-test-time lint (`supervisor::lint::supervisor_module_source_has_no_forbidden_outbound_calls`) that greps the module source for `eprintln!` / `println!` / `bus.publish` / `tracing::{info,warn,error}!`. Forbidden tokens are built by `concat!`-style string fragments and the haystack stops at the `fn forbidden_tokens` definition so the linter cannot trip itself.
- Added unit tests for the three terminal events and the non-terminal stay-alive property, plus integration test file `crates/codeless-runtime/tests/supervisor_e2e.rs` with `supervisor_spawns_on_run_start_and_exits_on_run_terminal` and `each_run_spawns_an_independent_supervisor`.
- `cargo test -p codeless-runtime --lib supervisor`, `cargo test -p codeless-runtime --test supervisor_e2e`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` all green. Committed on `codeless/job-chat` as `38c7d4c`.

## Next

- Stage 10 picks up the supervisor read-only tool surface (`get_job_state`, `read_events`, `read_handover`, `read_template`, `read_stage_log`, `read_notes`) and the assistant-runner wiring so `ChatMessageAppended` actually produces a `post_job_message` reply with `transport='supervisor'`.

## What you need to know

- The supervisor is intentionally fire-and-forget from `drive_job`'s side — the `JoinHandle` is bound to `_supervisor` and dropped. The task survives drop because it's a `tokio::spawn`, and exits on the terminal event the rest of `drive_job` is about to publish.
- The voice-contract lint reads the module source via `include_str!("mod.rs")` and slices off everything from `fn forbidden_tokens` onward before searching. A future contributor renaming that function will trip the lint on its own definition — the assert message tells them what to do.
- The doc-comment at the top of `supervisor/mod.rs` deliberately spells out the voice contract in prose (no literal forbidden tokens) so the same grep does not accidentally fire on documentation.
- A pre-existing flaky integration test, `codeless-runtime::tests::job_worktree::worktree_preserved_when_stop_wins_against_completion`, failed once during the first `cargo test --workspace` run and passed on the next run alone. Same for a couple of `codeless-adapters-host` shell/git tests under workspace-wide parallelism. None of these are caused by the supervisor wiring — they pass in isolation — and they were not introduced by stage 9.
- This worktree has no `mani` binary or `mani.yaml` in scope, so the commit was created via raw `git commit` on this branch. The workspace-level loop runner can push via mani in the next iteration if needed.

## Open questions

- (none)
