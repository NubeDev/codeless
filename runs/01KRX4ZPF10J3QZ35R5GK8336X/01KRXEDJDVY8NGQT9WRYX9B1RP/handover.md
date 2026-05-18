## Done

- Scaffolded mobile-safe crate `crates/codeless-tool-wit/` with `wit/tool.wit` (codeless tool ABI per PLUGIN-WASM.md: `package codeless:tool@0.1.0`, `tool` interface, `plugin` world, records/variant/enum mirroring `codeless-plugin-sdk::Manifest`).
- Committed `wit-bindgen 0.57.1` Rust guest output in-tree at `src/bindings.rs` per OQ-WASM-2; per-crate `unsafe_code = "deny"` override documented; `clippy::all`/`unsafe_code`/`unused_imports`/`rustfmt::skip` localised to the bindings module so the file remains a pure generator output.
- Added `tests/smoke.rs` with three doc-only checks: `wit-parser` reparses `TOOL_WIT` and asserts `PACKAGE_ID`; `ToolManifest`/`ToolCall`/`ToolResult` round-trip field-by-field; `Tier` discriminants pinned to 0/1/2.
- Workspace `Cargo.toml` adds the new member with an OQ-WASM-2 pointer comment.
- `cargo test -p codeless-tool-wit`, `cargo clippy -p codeless-tool-wit --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo fmt --check` all green; stage committed as `acc46ff`.

## Next

- Stage 4: scaffold host-only `codeless-plugin-host-wasm` (Wasmtime engine, WASI-p2 linker, per-call instantiation, HostPolicy fuel/memory/wall-clock caps) and introduce the mobile-safe `RuntimeAdapter` trait + table in `codeless-tools`. Host bindings come from `wasmtime::component::bindgen!` against `crates/codeless-tool-wit/wit/tool.wit`, not from the guest `bindings.rs` landed here.

## What you need to know

- The `plugin` world deliberately ships **no WASI imports**; stage 4 wires `wasmtime-wasi` against the per-instance capability set. Companion worlds only land when a future plugin needs static-WIT-visible imports.
- `wit-bindgen` CLI v0.57.1 is required for regeneration. The exact command is documented in `src/lib.rs`; never call it from `build.rs` (OQ-WASM-2).
- The workspace-wide `unsafe_code = "forbid"` is incompatible with wit-bindgen's CABI glue; the per-crate `deny` override is scoped, and hand-written unsafe outside `bindings.rs` is still a review failure.
- Mani isn't checked out into this worktree; commit was made with raw `git` (no push). Stage 1 and stage 2 commits also landed via raw git on this branch.
- One earlier `cargo test --workspace` run produced 4 transient failures in `codeless-adapters-host` git_diff/git_commit tests; re-runs green. Looks like a pre-existing parallel-tempdir flake unrelated to this stage.

## Open questions

- (none)
