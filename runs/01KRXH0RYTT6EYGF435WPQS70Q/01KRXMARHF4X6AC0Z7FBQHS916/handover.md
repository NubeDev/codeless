## Done

- Telegram chat substrate end-to-end: inbound mirrors text in bound channels via `post_job_message` (transport=telegram, external_id=platform message_id); outbound `ChatForwarder` subscribes to `ChatMessageAppended`, suppresses Telegram-origin and prior-receipt rows, fans others to bindings, then writes `metadata.delivery.telegram` via `update_chat_message_delivery`.
- `/codeless bind <job_id>` parser + handler in new `crates/codeless-telegram/src/inbound_chat.rs`; runs alongside the existing `/status` / `/stop` command dispatcher (chat-mirror Conflict on partial-unique redelivery is the design-as-intended drop).
- Substrate additions: `SqliteStore::update_chat_message_delivery` (read/merge/write inside a tx; never touches body/external_id), `list_chat_bindings_for_job`, new RPCs `update_chat_message_delivery` / `list_chat_bindings_for_job` / `get_chat_binding` wired through `codeless-rpc`, `codeless-runtime`, `codeless-server` routes, `codeless-client` HTTP client, and the specta `wire_ts` snapshot.
- Integration tests in `crates/codeless-telegram/tests/chat_forwarder.rs` cover all three echo-suppression / idempotency bullets against `InProcessRpc` + wiremock; full `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` are green.
- Commit `84b9ae6 stage 7: Telegram adapter — chat forwarder + bind + delivery receipts` on `codeless/job-chat`.

## Next

- Stage 8: per the JOB-CHAT.md C1 punch list this is the cold-load summary on `/codeless bind` (call `list_job_messages` and post a single condensed "joining mid-thread" message) plus the shared echo-suppression helper in `codeless-bot-core` so Slack reuses it.

## What you need to know

- Vendored `../ai-runner/Cargo.toml` had its `workspace = ...` pointer repointed from `job-01KRX4ZPF10J3QZ35R5GK8336X` to `job-01KRXH0RYTT6EYGF435WPQS70Q` so this worktree's build worked; ai-runner is not a git repo so that change is on-disk only. The next worktree will likely need to repoint it again.
- No `mani.yaml` exists in this worktree, so I committed with raw `git` (matching the same pattern visible in `git log` for earlier stages in this branch).
- The chat-forwarder forwards under the `[<transport>] <author>: <body>` shape; tune in stage 9+ if the UI grain wants something else.
- `get_chat_binding` RPC is new on the trait — any custom `RpcServer` impl outside this repo (none today) would need to implement it.

## Open questions

- (none)
