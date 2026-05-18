## Done

- Added `crates/codeless-slack/src/chat.rs` — Slack-side per-Job `ChatForwarder` mirroring `codeless-telegram::chat`. Subscribes to `ChatMessageAppended`, uses the shared `codeless_bot_core::chat_forward::classify` helper for echo suppression + delivery-receipt idempotency, fans non-Slack-origin messages out to every `chat_bindings.transport='slack'` row via `ChatPoster::post`, and writes the receipt to `metadata_json.delivery.slack` through `update_chat_message_delivery`.
- Wired the forwarder into `SlackBot::spawn_with` (always-on, drained in `shutdown` alongside the existing failure-card outbound publisher); added a `chat: chat::ChatForwarder` field on `SlackBot`.
- Added `crates/codeless-slack/tests/slack_chat_e2e.rs` — direct port of `codeless-telegram/tests/bot_chat_e2e.rs` with a `CannedSlackApi` wiremock stubbing `chat.postMessage` and returning `ts="1700.4242"`. Both `origin_transport_skips_self_post` and `cross_transport_forwards_with_receipt` are green.
- Added `codeless-runtime` + `async-trait` as dev-deps on `codeless-slack/Cargo.toml` for the e2e harness.
- `cargo test -p codeless-slack -p codeless-telegram -p codeless-bot-core` green; `cargo fmt --check` green; `cargo clippy -p codeless-slack --all-targets -- -D warnings` green.
- Committed as `6eda198` on `codeless/job-chat` with stage-16 title prefix.

## Next

- Stage 17 (per phase plan) — fresh session picks up.

## What you need to know

- The shared `classify` helper in `codeless_bot_core::chat_forward` already covered `ChatTransport::Slack` from earlier stages, so the parity addition is purely an adapter-side wire-up; no bot-core changes were needed.
- Slack thread identifiers are strings (`thread_ts`, e.g. `"1700.0001"`), unlike Telegram's numeric ids; `chat.rs` passes `binding.thread_id` through as `&str` directly to `ChatPoster::post`.
- Inbound parity (the `/codeless bind` command + chat-mirror on inbound Slack events) is **not** in this stage; the stage spec wording calls out "outbound-receipt helper" and "slack_chat_e2e mirrors bot_chat_e2e" and the e2e parity is achieved without inbound. The telegram-side `inbound_chat.rs` has no Slack equivalent yet — a follow-up stage will need to add the Slack `app_mention`/DM inbound substrate writes plus a `/codeless bind` handler if full inbound parity is desired.
- Workspace `cargo test --workspace` shows two pre-existing `codeless-adapters-host` git failures (`git: NotFound`) that are environment-only (no git binary in the test sandbox) and unrelated to this change.
- This worktree has no parent `mani.yaml` (the `mani`-via-workspace commit convention from CLAUDE.md does not apply here); commits in the recent log are plain `git commit`, which is what was used.

## Open questions

- Should a follow-up stage add Slack inbound substrate parity (bind command + chat mirror on `app_mention` / DM)? Stage 16 wording only required outbound + e2e parity; if inbound is expected the stage 17 author should call it out.
