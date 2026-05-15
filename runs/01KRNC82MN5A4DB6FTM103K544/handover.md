## Done

- Added `Persona` wire type (`crates/codeless-types/src/persona.rs`) re-exported from `codeless-types` and `codeless-rpc`.
- Added `ListPersonasArgs/Result`, `GetPersonaArgs`, `UpsertPersonaArgs`, `DeletePersonaArgs` to `codeless-rpc/src/methods.rs` and four corresponding `RpcServer` trait methods in `server.rs`.
- Implemented persona store CRUD in `codeless-runtime/src/store.rs` (`list_personas`, `get_persona`, `upsert_persona`, `delete_persona` with `built_in`/`created_at` preservation across upsert) plus a `persona_from_row` helper.
- Added runtime RPC dispatch module `crates/codeless-runtime/src/rpc/personas.rs` (with validation, NotFound mapping, built-in delete refusal as `Conflict`) and wired it through `rpc/mod.rs`.
- Added HTTP client trait impls in `codeless-client/src/http_client.rs` and POST routes/handlers in `codeless-server/src/routes.rs`.
- Added integration test `crates/codeless-runtime/tests/personas_rpc.rs` (5 tests, all pass): seeded built-ins listed in order, get/404, upsert create+update preserving built_in and created_at, validation rejection, delete refuses built-ins and removes user rows.
- UI: extended `RpcMethodMap` with the four methods (`ui/.../lib/rpc/methods.ts`), added in-memory persona seed + 4 handlers to `mock-client.ts`, and made `ai-agents` KV a cache that mirrors SQLite — `loadAgentsFromRpc`, `upsertPersonaViaRpc`, `deletePersonaViaRpc` in `modules/ai/lib/agents.ts`; the `agentsStore.hydrate/upsert/remove` now accept an optional `RpcClient` and are wired through `AgentsSection.tsx` via `useRpc()`. KV fallback preserved for the RPC-offline path.
- `cargo build`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, the new persona tests, `pnpm typecheck`, `pnpm test` (vitest) all pass.
- Committed (`stage 7: RpcClient surface for personas`) and pushed to `codeless/agent-personas`.

## Next

- Stage 8: promote `jobs.persona_id` to a real FK, add `stages.persona_id` with the same FK, and record the persona on each stage's handover.

## What you need to know

- Pre-existing flaky test `rpc_in_process::job_filtered_subscription_drops_unrelated_events` fails before and after this stage — verified via `git stash`. Not caused by this change; flag for separate triage.
- `Persona.built_in` is preserved by `SqliteStore::upsert_persona` (explicit INSERT/UPDATE branches instead of REPLACE) so a user-edited built-in stays a built-in and the RPC's delete-refusal of built-ins still bites.
- The UI's `agentsStore.upsert(agent, rpc)` writes through to SQLite first, then updates the in-memory list and KV from the *returned* row, so the three layers stay coherent. Built-ins remain absent from `customAgents` even after an edit echoes them back.
- `loadAgentsFromRpc` falls back to the KV cache on transport error, so a brief outage does not blank the persona rail — but R4 still holds (SQLite is the source of truth; a successful refresh overwrites the cache).
- `BUILTIN_AGENTS` (the local TS constant) and the seeded SQLite built-ins both ship with `use_for_jobs = false` and all four registry subagents allowed.

## Open questions

- (none)
