## Done

- Added `store::connect_pool(path)` in `crates/codeless-runtime/src/store/mod.rs`: builds a `SqlitePool` with `create_if_missing`, max 4 connections, and an `after_connect` hook running `PRAGMA journal_mode = WAL; synchronous = NORMAL; busy_timeout = 5000` for every new connection.
- Routed `InProcessRpc::with_file` in `crates/codeless-runtime/src/rpc/mod.rs` through the new helper so production on-disk pools pick up the PRAGMAs; in-memory test pools (`SqlitePool::connect("sqlite::memory:")` in `InProcessRpc::new` and the per-module test helpers) are untouched and continue to opt out.
- New `pool_pragma_tests` unit module verifies a fresh on-disk pool reports `journal_mode=wal`, `synchronous=1` (NORMAL), `busy_timeout=5000`, and that the embedded migrator still applies cleanly to that DB.
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test -p codeless-runtime` (incl. the `tests/migrations.rs` Appendix-A matrix) all green.
- Committed as `145e337` on `codeless/auto-bypass-hardening`.

## Next

- Stage 6 of 15. Per the job goal, the remaining work covers: prior-stage `failure_class` + `failure_detail` carry-forward into the next stage's prompt on auto-bypass, the pre-check Done-vs-diff tokenizer tightening to drop non-path tokens like `metadata.delivery.slack`, `rest_proxy.path`, `tool.call`, and the StagesOverview UI glyph (~ for bypassed-after-failure vs ! for halted) + tooltip. A fresh session picks one of these.

## What you need to know

- `connect_pool` lives in `store/mod.rs` (per the stage brief) but is a free function, not a `SqliteStore` method, so it can be called before the store exists — `with_file` needs the pool first, then hands it to `with_db` which builds `SqliteStore`. If a later stage wants `SqliteStore::open_on_disk(path)` sugar, wrap this helper.
- The `after_connect` PRAGMA batch is executed via `sqlx::Executor::execute(&str)` so a single round-trip sets all three; `journal_mode = WAL` persists across reconnects because SQLite stores it in the DB header, but the hook still re-asserts it for the synchronous + busy_timeout settings, which do not persist.
- Caller signature unchanged: `InProcessRpc::with_file(&Path)` still returns `Result<Self, sqlx::Error>`; all existing call sites (`codeless-cli/src/rpc_open.rs`, `codeless-tauri-desktop/src/boot.rs`, CLI tests) compile and run untouched.
- Saw one flake during the run: `jobs_assistant_rpc::draft_from_conversation_picks_most_recent_proposal` failed once then passed on rerun and on three follow-up loops. The test relies on two `add_repo` calls landing in distinct ULID milliseconds; it's a pre-existing timing flake on the in-memory path (which this stage did not touch). Worth flagging if it recurs but not in scope here.

## Open questions

- (none)
