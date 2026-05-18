## Done

- stage 17 ("stage 14: pre-armed action loop") committed on codeless/job-chat as 7d3cb8d
- `Tools::fire_pre_armed_goal` + `Tools::mark_goal_fired` added on the supervisor tool surface (crates/codeless-runtime/src/supervisor/tools/actions.rs); StopJob / PostChatMessage / PauseAfterStage all handled, with PauseAfterStage logging a Failed-shaped audit row to honour the store's NoOpFailed signal
- supervisor reactor rewritten in crates/codeless-runtime/src/supervisor/mod.rs: rehydrates armed supervisor_goals on boot, arms one tokio::time::sleep per deadline-stop row via a FuturesUnordered<Pin<Box<dyn Future>>>, races event-stream + timers in a single tokio::select!
- supervisor_e2e::deadline_stop_fires_at_t_plus_one_hour added (crates/codeless-runtime/tests/supervisor_e2e.rs) — uses tokio::time::pause+advance+resume to fire the 1h arm without real wall-clock wait; asserts metadata.replies_to references the authorising chat_messages.id, metadata.goal_id references the goal, no System-role preview row was emitted, and the job + goal both reach terminal
- tokio dev-deps gain the test-util feature in crates/codeless-runtime/Cargo.toml so time::pause / time::advance are callable from tests
- cargo fmt clean, cargo clippy --workspace --all-targets -D warnings clean, cargo test -p codeless-runtime --test supervisor_e2e 7/7 green (verified 10 consecutive runs serial + 10 parallel)

## Next

- stage 18 of 21 per the loop spec — pick up from JOB-CHAT.md §C3 punch list (threshold-stop / event-notify arming, deadline / threshold intent recognition from chat, or Slack adapter parity, depending on the loop's next stage entry)
- driver.rs already calls spawn_supervisor_with_tools, so the pre-armed loop is live in production builds the moment goals exist — no additional wiring needed in the driver

## What you need to know

- The reactor's seam to persistence stays single-source: it never imports the store module directly; Tools::mark_goal_fired wraps SqliteStore::mark_fired so the lint test's process-spawn / direct-bus / loud-tracing guards in supervisor/mod.rs still pass
- arm_goal_timer stores absolute wall-clock deadlines (deadline_ms - now_ms() at boot) and uses tokio::time::sleep over the delta — keeps production on real wall time and tests on mock tokio time; a past deadline maps to a zero-duration sleep so a restart after the original deadline still catches the missed fire on the first select tick
- The pre-existing migrations test `migrator_creates_all_tables_from_appendix_a` fails on this branch (the Appendix-A list does not yet include supervisor_goals); reproduces with `git stash`, not introduced by this stage
- The test's pause+advance dance is order-sensitive: spawn supervisor first (so subscribe_since / list_armed_for_run go through sqlx on the live clock), THEN pause+advance+resume — pausing before spawn starves the sqlx pool acquire path and surfaces as PoolTimedOut

## Open questions

- threshold-stop / event-notify arming is intentionally `None` in arm_goal_timer for this stage — next stage(s) need to introduce their signal sources (metric sampler for cost/wall-clock; bus-tag predicate for event-notify) before those variants can fire
- On a Run terminal envelope the supervisor currently posts its terminal summary and exits; it does not yet flip remaining `armed` rows to `superseded`. JOB-CHAT.md §"How if it runs >1h, stop it works" item 4 covers this case — likely belongs in the next stage or with the goal-insertion path
