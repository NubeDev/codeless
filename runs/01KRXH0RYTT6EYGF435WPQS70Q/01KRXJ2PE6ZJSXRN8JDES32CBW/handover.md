## Done

- Added `PostJobMessageArgs`, `ListJobMessagesArgs`/`ListJobMessagesResult`, `BindChatThreadArgs` to `codeless-rpc::methods` with specta derives and snapshot registration.
- Added the three trait methods (`post_job_message`, `list_job_messages`, `bind_chat_thread`) on `RpcServer`.
- Implemented them in new `codeless-runtime::rpc::job_chat` backed by a new `codeless-runtime::store::chat` (insert with `InsertChatMessage::{Inserted,DuplicateExternalId}`, paginated newest-first list reversed to oldest-first, idempotent binding upsert).
- Wired the methods through `InProcessRpc`, `HttpRpcClient`, and the axum `routes.rs` POST surface (`/rpc/post_job_message`, `/rpc/list_job_messages`, `/rpc/bind_chat_thread`).
- 8 new unit tests pin the post→list round-trip, backward pagination, partial-unique-index conflict (and NULL non-collision), bind upsert idempotency, and NotFound through the RPC wrapper.
- `cargo test -p codeless-rpc -p codeless-runtime -p codeless-client -p codeless-server`, `cargo clippy --all-targets -D warnings`, `cargo fmt --check` all green.
- Committed as `8902e1f` on `codeless/job-chat`.

## Next

- (none — stage 5 is for a fresh session)

## What you need to know

- The RPC handler module is named `rpc::job_chat`, not `rpc::chat`, because `rpc::chat` already owned the unrelated `agent_chat` surface — splitting per R3 (one concept per file).
- `run_id` is left NULL by `post_job_message`; per JOB-CHAT.md OQ-CHAT-4 it stays NULL until the JOB-WORKFLOW (B) Job/Run split lands.
- The `external_id` partial-unique-index conflict maps to `RpcError::Conflict`. NULL-external_id rows do not collide (verified by test).
- No event variants were added in this stage; `ChatMessageAppended` / `ChatBindingCreated` are explicitly future-stage work.
- `crates/codeless-runtime/src/store/chat.rs` includes a SQLite-error helper `is_unique_violation` that branches on `ErrorKind::UniqueViolation` plus the code strings (2067 / 1555 / 19) for older sqlx fallback.
- The `ai-runner` sibling crate's `workspace = "..."` pointer in `Cargo.toml` was pointed at this worktree to make `cargo test` resolve; if a parallel worktree needs it back, that file is the single point of repoint.

## Open questions

- (none for this stage)
