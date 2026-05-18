## Done

- Added `crates/codeless-runtime/src/supervisor/tools.rs` with `Tools` carrying `Arc<EventBus>` + `Arc<SqliteStore>` and an optional `worktree_root`, exposing the seven supervisor tools: `get_job_state`, `read_events`, `read_handover`, `read_template`, `read_stage_log`, `read_notes`, and the single write `post_chat_message`.
- Added `EventBus::fetch_recent_for_job` (public, cursor-ascending tail per job, clamped by caller) so `read_events` routes through an existing event-bus method rather than reaching into the pool directly.
- Rewrote `crates/codeless-runtime/src/supervisor/mod.rs`: preserved `spawn_supervisor(bus, job_id)` for the existing driver wiring; added `spawn_supervisor_with_tools(bus, store, job_id)` with a reactor that answers non-supervisor "what stage" / "which stage" chat questions via `get_job_state` + `post_chat_message`. The supervisor's lint test now also forbids `std::process` and `tokio::process` substrings (R1 of CLAUDE.md + JOB-CHAT.md Hard rule 2).
- Added `supervisor_e2e::supervisor_answers_what_stage_is_it_on` (canned `Stage` row at ordinal 10 + Web-origin chat post → Supervisor reply within 2s citing "stage 10"). Two pre-existing supervisor_e2e tests still green.
- `cargo test -p codeless-runtime --lib supervisor` (7/7), `--test supervisor_e2e` (3/3), `cargo clippy -p codeless-runtime --all-targets -- -D warnings` clean, `cargo fmt --check -p codeless-runtime` clean.
- Commit `ba21cd8` on `codeless/job-chat`.

## Next

- Stage 11 (per JOB-CHAT.md C2 punch list): wire `spawn_supervisor_with_tools` into `driver::drive_job` in place of the lifecycle-only spawn, so the production path gets the tool surface. That means threading an `Arc<InProcessRpc>` (or at minimum the store) into `drive_job` — today it takes `&InProcessRpc`. Plus a "on Run terminal status, supervisor posts a one-paragraph summary and exits" hook (JOB-CHAT.md C2 last bullet).
- Stage 11 should also extend the reactor's intent vocabulary beyond hand-rolled "what stage" — likely by dispatching through the assistant runner registry so the supervisor's reply is LLM-driven against the same tool surface.

## What you need to know

- The chat reactor pattern-matches on lowercase substring "what stage" / "which stage". It is intentionally narrow per the stage-10 contract; the LLM-driven path is the next stage's problem.
- `post_chat_message` is the sole write tool. The supervisor mod.rs lint test scans the source of `mod.rs` only (via `include_str!`), so tool implementations in `tools.rs` (which legitimately call `bus.publish` and `store.insert_chat_message`) are not constrained. The doc-comments in `mod.rs` carefully avoid the literal strings `std::process` / `tokio::process` / `bus.publish` / `eprintln!` / `println!` / `tracing::info!|warn!|error!` because the lint scans up to `fn forbidden_tokens`.
- The ai-runner workspace pointer at `../ai-runner/Cargo.toml` belongs to a sibling worktree; toggling it back to this worktree is required to build/test locally and must be reverted afterwards. The harness reminder confirms the canonical pointer for that file is `job-01KRX4ZPF10J3QZ35R5GK8336X`; the commit on `codeless/job-chat` does not touch it.
- `Tools::new(bus, store)` defaults `worktree_root` to `None`, in which case `read_handover` / `read_stage_log` / `read_notes` return `ToolError::NotConfigured`. Production wiring (next stage) should call `.with_worktree_root(...)` with the same root the `WorktreeManager` uses.
- `JobStateView::current_stage` resolution: highest-ordinal `Running` stage; falls back to highest-ordinal stage overall. The reactor's reply format is `"Currently on stage {ordinal} ({name}). Status: {status:?}."`.

## Open questions

- Should `drive_job` start passing `Arc<InProcessRpc>` (broader refactor across all callers, including tests in `job_worktree.rs`, `cap_cancellation.rs`, etc.), or should the driver only pass `Arc<SqliteStore>` and keep its `&InProcessRpc` signature? The current stage punts; stage 11 has to decide.
- The supervisor's reactor matcher is hand-rolled. JOB-CHAT.md §C2 says "ask 'what stage is it on?' in any surface, get a real answer that references the actual event stream" — stage 11 has to decide whether the dispatch upgrades through the existing `agent_chat` registry or through a separate supervisor-specific runner adapter.
