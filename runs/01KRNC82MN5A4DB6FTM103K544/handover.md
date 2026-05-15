## Done

- migration 0011_personas.sql with table, index, and built-in seed rows
- tests/migrations.rs: added personas to table list, added personas_table_matches_schema_sketch_and_seeds_built_ins test
- commit 7ff63b5 "SQLite personas table" on codeless/agent-personas

## Next

- Stage 7: add RpcClient surface (list_personas / get_persona / upsert_persona / delete_persona) backed by the new table; convert the UI ai-agents KV store into a read-through cache mirroring SQLite via these methods. Built-in rows must remain non-deletable (enforce at the RPC layer per the column comment).

## What you need to know

- Seeded built-in rows use created_at=0 / updated_at=0 to keep the migration content-hash stable; the upsert path stamps real wall-clock millis.
- allowed_subagents is seeded as the full registry set ["explore","code-review","security","general"] for every built-in, matching the current UI default. default_snippets is "[]".
- use_for_jobs starts 0 on every built-in per SCOPE.md; the user flips it on per persona / clone (and that flag is the single MCP gate per D3).
- I had to repoint /home/user/.codeless/worktrees/ai-runner/Cargo.toml's `workspace = "../job-..."` to this worktree to get cargo to compile (the sibling crate had a stale pointer to job-01KRNRQ6...). That edit is outside this repo and not part of the commit.
- Pre-existing failing test `rpc_in_process::job_filtered_subscription_drops_unrelated_events` is unrelated to this stage — confirmed via git stash + retest. Do not chase it as part of stage 7.
- sqlx::migrate! is compile-time embedded; if migrations look stale during testing, `touch crates/codeless-runtime/src/migrations.rs` to force a rebuild.

## Open questions

- (none)
