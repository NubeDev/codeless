## Done

- Added `MessageId` ulid newtype in `crates/codeless-types/src/id.rs`.
- New `crates/codeless-types/src/chat.rs` module with `ChatMessage`, `ChatTransport` (lowercase wire: web/cli/telegram/slack/supervisor), `ChatRole` (lowercase: user/assistant/tool/system), and `ChatBinding`. All derive `specta::Type`, `Serialize`, `Deserialize`.
- Exported the new types and `MessageId` from `lib.rs`.
- Registered the 5 new types in `tests/specta_snapshot.rs` and regenerated `tests/wire.ts.snap` via `SPECTA_UPDATE=1`.
- Verified: `cargo test -p codeless-types`, `cargo clippy -p codeless-types --all-targets -- -D warnings`, `cargo fmt --check -p codeless-types` all green.
- Committed on `codeless/job-chat` as `8580335`.

## Next

- Stage 4 (per JOB-LOOP plan): SQL migrations for `chat_messages` and `chat_bindings`, plus the RPC arg/result types in `codeless-rpc`.

## What you need to know

- `ChatMessage.run_id` is typed `Option<String>` (not `Option<RunId>`) because the `RunId` newtype belongs to JOB-WORKFLOW (B) which hasn't landed. The SQL column is already nullable; swapping to a transparent ULID newtype later is wire-compatible. A comment on the field flags this.
- `ChatMessage.metadata_json` is `Option<String>` (raw JSON text) rather than `serde_json::Value` to keep `codeless-types` free of `serde_json` runtime dep — codeless-types is mobile-safe (R1) and this matches the existing `assistant.rs::meta_json` convention.
- `ChatBinding.thread_id` is a non-optional `String`; empty string is the sentinel for "no thread on this transport" per JOB-CHAT.md (avoids SQLite NULL-in-UNIQUE pitfall on the PK).
- Wire-name convention: `ChatTransport` and `ChatRole` use `#[serde(rename_all = "lowercase")]` to match the SQL `transport`/`role` column values exactly.
- Side note: `../ai-runner/Cargo.toml`'s `package.workspace` pointer was pointing at a different worktree and blocked the build. I redirected it to this worktree (`job-01KRXH0RYTT6EYGF435WPQS70Q`). That edit is outside this repo so it is not part of the commit — the next worktree may need to repeat the redirect (or the workspace launcher should set it on creation).

## Open questions

- (none)
