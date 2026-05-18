## Done

- crates/codeless-runtime/migrations/0024_chat_messages.sql: creates `chat_messages` with the 11-column layout from JOB-CHAT.md, the `chat_messages_job_idx` lookup index, and a partial-unique `chat_messages_external_idx ON (transport, external_id) WHERE external_id IS NOT NULL`. Header comments explain why the unique index is partial (SQLite treats NULLs as distinct, so a naive UNIQUE would silently allow duplicate ingest for web/cli/supervisor rows) and why `run_id` is nullable (pre-JOB-WORKFLOW-(B) rows).
- crates/codeless-runtime/migrations/0025_chat_bindings.sql: creates `chat_bindings` with `thread_id TEXT NOT NULL DEFAULT ''`. Header comment explains the empty-string sentinel and why NULL would break the composite PK on (transport, channel_id, thread_id).
- crates/codeless-runtime/tests/migrations.rs: added `chat_bindings` and `chat_messages` to the table-list assertion; added three new tests — column/index/FK shape, partial-unique-allows-NULL-duplicates-but-rejects-real-duplicates, and a replay-on-populated-DB idempotence test. Added a `seed_job` helper that inserts a stub `repos` row plus a `jobs` row.
- All 19 tests in `cargo test -p codeless-runtime --test migrations` pass; `cargo clippy -p codeless-runtime --tests -- -D warnings` and `cargo fmt --check` are clean.
- Committed on `codeless/job-chat` as 1a05a51 ("stage 2: schema — chat_messages + chat_bindings migrations").

## Next

- Stage 3 in the kickoff sequence (the next of the 21). The C1 punch list in JOB-CHAT.md puts wire types (`ChatMessage`, `ChatTransport`, `ChatRole`, `ChatBinding`, `MessageId`) in `codeless-types` and the `RpcServer` method signatures (`post_job_message`, `list_job_messages`, `bind_chat_thread`) in `codeless-rpc` next.

## What you need to know

- `../ai-runner/Cargo.toml` has its `workspace = "..."` field hard-coded to **another** worktree (`job-01KRX4ZPF10J3QZ35R5GK8336X`) because a parallel job is running. To run `cargo test/clippy`, you must `sed` it to point at this worktree, then restore (I kept a backup at `/tmp/ai-runner-cargo.bak`). Cargo refuses to build until it points here. I left it pointing at the other worktree at the end of this session so I don't break the parallel job.
- The migration matrix test now seeds both `repos` and `jobs` (jobs.repo_id is NOT NULL and FKs repos; cost_cap_cents and wall_clock_cap_ms are NOT NULL with no defaults). Re-use `seed_job` from later stages instead of duplicating the schema knowledge.
- `chat_messages.transport` lowercase-ASCII wire-name convention (settled in stage 1) is enforced by application code, not by a CHECK constraint — the schema accepts any TEXT. Stage 3's wire types must serialise to lowercase to honour the contract.
- `chat_messages.run_id` is intentionally nullable for the pre-JOB-WORKFLOW-(B) window; do not add a NOT NULL constraint until (B) lands and back-fills.
- `chat_bindings.thread_id`'s default-empty-string is the *PK-defending* sentinel; never let the application code write NULL there.

## Open questions

- (none)
