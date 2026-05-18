//! Canonical home of the codeless WIT tool ABI.
//!
//! The contract this crate carries:
//!
//! - `wit/tool.wit` is the **load-bearing artefact**. Every other
//!   file here is downstream of it. A change to the WIT is an ABI
//!   change; bump the package version (`codeless:tool@0.1.0` -> `…@0.2.0`)
//!   and add a successor doc (`DOCS/plugins/PLUGIN-WASM-V2.md`)
//!   before touching the existing surface.
//!
//! - `src/bindings.rs` is `wit-bindgen`'s guest-side Rust output for
//!   that WIT, committed in-tree per OQ-WASM-2 (`PLUGIN-WASM.md §
//!   Open questions`). The generated file's diff is the load-bearing
//!   review artefact when the WIT changes; that visibility is the
//!   whole reason we do not regenerate it from `build.rs`.
//!
//! Mobile-safety. This crate has no host-only dependencies
//! (`wasmtime`, `tokio::process`, `codeless-tools`, ...). On non-wasm
//! targets, only the pure data types in `bindings.rs` are reachable;
//! the WASM ABI glue is gated behind `#[cfg(target_arch = "wasm32")]`
//! in the generated file. The host-side bindings (for
//! `codeless-plugin-host-wasm`, landing in stage 4) come from
//! `wasmtime::component::bindgen!` invoked against `wit/tool.wit` --
//! a separate generator that produces host code, not guest code.
//!
//! Regeneration. The exact command is documented next to the WIT
//! file:
//!
//! ```bash
//! wit-bindgen rust crates/codeless-tool-wit/wit/ \
//!     --out-dir crates/codeless-tool-wit/src/
//! mv crates/codeless-tool-wit/src/plugin.rs \
//!    crates/codeless-tool-wit/src/bindings.rs
//! ```
//!
//! A `cargo xtask wit-bindgen` task will land alongside the host
//! crate in stage 4 -- until then the manual command above is the
//! contract. Never call `wit-bindgen` from `build.rs`: hiding an ABI
//! change behind a build script defeats OQ-WASM-2.

// The committed wit-bindgen output uses `unsafe` for the WASM ABI
// glue (CABI alloc / `String::from_utf8_unchecked`); see the
// per-crate `[lints]` override in `Cargo.toml` that downgrades
// `unsafe_code` from the workspace-wide `forbid` to `deny` so a
// hand-written unsafe still trips review.
// `wit-bindgen` regenerates this file from `wit/tool.wit` -- any
// clippy disagreement with its style is its problem, not ours.
// Hand edits here would be lost on the next regeneration.
#[allow(unsafe_code, unused_imports, clippy::all, clippy::pedantic)]
#[rustfmt::skip]
pub mod bindings;

/// WIT source string compiled into the crate, so consumers that only
/// have the binary (and not the source tree) can still recover the
/// canonical ABI text. Stage 4's host-side `bindgen!` invocation
/// reads from `wit/tool.wit` directly; this constant exists for
/// runtime introspection and for the smoke test that re-parses it.
pub const TOOL_WIT: &str = include_str!("../wit/tool.wit");

/// Package identifier the WIT file declares. Mirrored as a Rust
/// constant so a stage 4 host-side smoke test can assert the host
/// loader's expected package name matches the artefact -- a cheap
/// guard against a wit-bindgen regeneration that silently renames
/// the package.
pub const PACKAGE_ID: &str = "codeless:tool@0.1.0";
