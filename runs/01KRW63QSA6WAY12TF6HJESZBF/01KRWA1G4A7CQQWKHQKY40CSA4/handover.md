## Done

- Added `Event::AssistantThreadTouched { thread_id: AssistantThreadId }` to `crates/codeless-types/src/event.rs` (with the kebab-case wire label `assistant-thread-touched`); regenerated `crates/codeless-types/tests/wire.ts.snap`, `crates/codeless-rpc/tests/wire-rpc.ts.snap`, and `ui/codeless-ui/src/lib/rpc/generated/wire.ts` via `SPECTA_UPDATE=1` and `wire_ts` example.
- Added `publish_thread_touched` helper in `crates/codeless-runtime/src/rpc/assistant.rs` and wired it into every `touch_assistant_thread` callsite (`upload_assistant_attachment`, `append_assistant_message`, both `confirm_assistant_action` branches, `cancel_assistant_action`). Publishes with `bus_job_id = JobId(thread_id.0)` to match the planner's existing key. Publish failures downgraded to a `tracing::warn` — the DB touch already landed (R4).
- Added test `append_assistant_message_publishes_thread_touched_envelope` asserting the envelope lands on the bus on the per-thread filter.
- Retired `refreshTick` / `bumpRefresh` from `ui/codeless-ui/src/modules/assistant/focusStore.ts`; updated the docstring to point at the new envelope.
- Updated `AssistantPage.tsx` to subscribe via `useEventStream({ scope: "all" }, …)` with a microtask-coalesced refresh; updated `AssistantThreadView.tsx` to subscribe with the per-thread `{ scope: "job", job_id: thread.id }` filter and bump a local `touchTick` that re-fires its `list_assistant_messages` fetch; updated `AssistantFooterBar.tsx` to do the same for the pending-card count and to drop the `bumpRefresh` calls on submit / new-thread.
- Updated `.codeless/jobs/assistant-parity/SCOPE.md` open question §2 with the W1c resolution.
- Trio green: `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, all `rpc::assistant` / `rpc::assistant_planner` tests (56 passed). UI `pnpm test --run` 63 passed, `pnpm typecheck` clean, `pnpm lint` is a no-op stub. Pre-existing failure in `migrations::migrator_creates_all_tables_from_appendix_a` is unrelated (introduced by the parallel `todos-recorder-and-gate` job adding a `todos` table) — verified by stashing my changes and re-running.
- Committed as `0822b07` "W1c retire focusStore refreshTick once the rail subscribes to the planner thread-touched envelope" and pushed to `origin/feat/assistant-parity` via mani.

## Next

- Stage 7: W1d parity test asserts identical message-list DOM for job vs assistant threads. The wiring W1c added (per-thread touch subscription on `AssistantThreadView`) is the last freshness-signal piece before the parity test can assert that a touch in either pane produces the same DOM mutation downstream.

## What you need to know

- New envelope shape: `{ type: "assistant-thread-touched"; thread_id: AssistantThreadId }`. It is published with `job_id = Some(JobId(thread_id.0))`, so the existing `{ scope: "job", job_id: thread_id }` filter (already used by `ChatMessageList` in `AssistantThreadView`) receives the envelope without changes. Surfaces that want touches across every thread (the rail) must use `{ scope: "all" }`; the bus has no per-thread server-side filter.
- `AssistantPage` uses a `refreshPendingRef` + `queueMicrotask` to coalesce the replay backlog into a single refresh on first mount. The default `useEventStream(since=0)` replays every persisted touch envelope — without the coalesce the rail would re-fetch once per historical touch.
- A pre-existing test failure remains in `crates/codeless-runtime/tests/migrations.rs::migrator_creates_all_tables_from_appendix_a` (the table list in the test does not include `todos`, but the latest migration adds one). Out of scope for this stage; the `todos-recorder-and-gate` job is the owner.
- An incidental `cargo fmt` reflow landed in `crates/codeless-runtime/examples/parse_check.rs` — pre-existing drift that fmt cleaned up. Three lines, no semantic change.

## Open questions

- (none)
