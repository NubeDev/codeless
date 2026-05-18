## Done

- Added `codeless-bot-core::chat_forward` pure helper for the asymmetric echo-suppression rule (Skip/Forward classifier, receipt presence lookup, lowercase wire-name table) with 7 unit tests, and exported it from `lib.rs`.
- Added `serde_json` to `codeless-bot-core/Cargo.toml`.
- Refactored `codeless-telegram::chat::handle_message` to defer to `chat_forward::classify`; removed the duplicate `has_delivery_receipt` and its tests now that the bot-core helper owns them.
- Wired cold-load into `codeless-telegram::inbound_chat::run_bind`: after a successful `bind_chat_thread`, calls `list_job_messages` with `COLD_LOAD_TAIL = 10`, renders one condensed `[joining mid-thread] <job>: last N message(s)` summary with per-row `[transport] author: preview` lines (whitespace collapsed, body truncated at 120 chars), and posts it once. 4 new unit tests cover the renderer.
- Created `crates/codeless-telegram/tests/bot_chat_e2e.rs` with `origin_transport_skips_self_post` and `cross_transport_forwards_with_receipt`, both driving the live `ChatForwarder` against an in-memory `InProcessRpc` bus and a `CannedTelegramApi` wiremock stub.
- Committed as `f5e6c79` on `codeless/job-chat`; commit subject begins with the stage title.

## Next

- Stage 9 takes over per the JOB-CHAT.md punch list. Likely shape: `codeless-slack` adapter parity — same `ChatForwarder` shape against Slack's `chat.postMessage`, using the same `chat_forward::classify` helper and a Slack-side `CannedSlackApi` mirror of these tests.

## What you need to know

- `cargo test -p codeless-bot-core -p codeless-telegram` passes 130 tests (the new `bot_chat_e2e` file contributes 2, the bot-core `chat_forward` module contributes 7, the inbound-chat renderer contributes 4).
- Clippy could not be run at commit time: another concurrent worktree had rewritten `../ai-runner/Cargo.toml`'s `workspace = "…"` pointer to its own path, which makes any cargo workspace operation fail with "wrong workspace". The test run earlier in the session succeeded under the same code; retry clippy once the conflicting session settles, or temporarily rewrite that pointer back to this worktree.
- The shared helper is intentionally a pure function operating on `ChatMessage` — neither the fan-out loop nor the receipt-write UPDATE moved. That keeps the per-transport crate owning its own I/O (R1 boundary holds: bot-core stays free of `tokio::process` / `std::process`, and the helper itself does no async at all). When the Slack adapter lands in the next stage it should call `classify(ChatTransport::Slack, &msg)` and reuse the same `transport_wire_name` so the `metadata.delivery.slack` key matches.
- The CannedTelegramApi is a wiremock-backed stub local to `bot_chat_e2e.rs`; the prior `chat_forwarder.rs` tests still cover the same three behaviour bullets and are not redundant — they assert the receipt key shape from a different angle. Keep both.
- The cold-load summary is posted into the channel (no `message_thread_id`), so on a forum-topic bind the summary lands at the channel root rather than inside the topic. JOB-CHAT.md does not specify the topic-vs-channel placement; flag if the next reviewer wants it inside the topic.

## Open questions

- (none)
