## Done

- Added `codeless-tools::fs` module: `Sandbox` (workspace-root path resolver, rejects absolute / `..` / out-of-root symlinks; canonicalises both root and target), `FsListTool` (`fs.list`), `FsReadTool` (`fs.read`, 5 MiB cap = `READ_BYTE_CAP`), `FsSearchTool` (`fs.search`, 200-match cap = `SEARCH_MATCH_CAP`, gitignore-aware via `ignore::Walk`, regex per line, skips files with a NUL byte).
- `register_assistant_thread_read_tools(&mut ToolRegistry, Arc<Sandbox>)` is the seam stage 5 will call to wire the trio onto a per-assistant-thread registry.
- Per-tool unit tests in each `tools::*` module + sandbox unit tests including a Unix-only symlink-escape test.
- Integration test `crates/codeless-tools/tests/fs_tools.rs` drives all three tools through `ToolRegistry::get` (the same dispatch path `codeless-mcp` uses) and asserts every tool refuses absolute and parent-traversal paths.
- Added direct deps `ignore = "0.4"` and `regex = "1"` to `codeless-tools/Cargo.toml`.
- Verified: `cargo fmt --check`, `cargo clippy -p codeless-tools --all-targets -- -D warnings`, and `cargo test -p codeless-tools` all green (153 lib tests + integration suites).

## Next

- Stage 5: wire the per-thread tool registry into the planner's runner so the model can actually invoke `fs.list` / `fs.read` / `fs.search`. The registration helper is in place; the missing piece is the MCP / planner glue that constructs a `Sandbox` from the workspace root, builds a registry per assistant-thread call, and exposes it to the `agent_chat`-spawned CLI runner. Reads `AssistantThreadMode` server-side per SCOPE D8 so write tools (stage 6) are not registered when mode == `ReadOnly`.

## What you need to know

- The "planner's tool registry" referenced in the stage description does not yet exist as a concrete type in `codeless-runtime`. The current planner (`crates/codeless-runtime/src/rpc/assistant_planner.rs`) uses ai-runner's CLI with no MCP/tool registry attached; `BUILTIN_ASSISTANT_TOOLS` is a prompt-trailer-only catalogue producing `AssistantAction` cards, which is not the right surface for read-only fs tools. Stage 5 will add the actual registry plumbing; stage 4 only delivers the tools and the registration helper.
- `Sandbox::resolve_existing` requires the target to exist. It surfaces a missing file as `ToolError::Failed` (not `Denied`) so the planner can distinguish "wrong path" from "policy refused"; stage 6 will need a `resolve_for_create` variant for `fs.write`.
- `fs.search` is synchronous internally (wrapped in `spawn_blocking`) because `ignore::Walk` is sync. It honours `.gitignore`, `.ignore`, hidden file filtering, and a 2 MiB per-file cap (`SEARCH_FILE_BYTE_CAP`) so a checked-in lockfile cannot stall the walk.
- The `ignore` crate re-exports `globset`; the `glob` arg on `fs.search` is plumbed through `OverrideBuilder::add`.
- All new files honour CLAUDE.md R2: comments explain *why*, no emojis, no task-status markers.

## Open questions

- Stage 5's `Sandbox` lifetime: should the planner construct one per call (re-canonicalising the root every dispatch) or hoist it to the thread context? Per-call is simpler; hoisting requires invalidating on workspace re-init.
- Whether `fs.read` should also surface a SHA-256 of the content (for the planner to dedupe against earlier reads). Out of scope here; mention it if stage 5 reveals duplicate reads in real traffic.
