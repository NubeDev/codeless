//! Codeless plugin authoring SDK.
//!
//! One source file (`impl ToolBehavior for MyTool { ... }`) compiles
//! into three packaging shapes, picked by Cargo feature:
//!
//! - `builtin` -- statically linked into `codeless-server`. The
//!   `register!` macro emits an inventory entry the host collects at
//!   startup. This is what the in-tree `notes` plugin uses today
//!   (through `codeless-tools::Tool` directly; the migration onto
//!   this SDK is a later stage in the plugin-substrate-runtimes job).
//! - `wasm` -- packaged as a WASI-p2 component (`cargo build
//!   --target wasm32-wasip2`). `register!` emits `wit-bindgen`'s
//!   `export tool` glue. The host-side loader lives in
//!   `codeless-plugin-host-wasm` (PLUGIN-WASM.md).
//! - `process` -- reserved seam for PLUGIN-PROCESS.md item 11. The
//!   feature exists so a plugin manifest declaring
//!   `kind = "process"` does not break the build today; no `register!`
//!   wiring lives behind it yet.
//!
//! Mobile-safety. The SDK has no host-only dependencies (no
//! `codeless-tools`, no `codeless-adapters-host`, no `tokio::process`,
//! no `wasmtime`). The crates that *do* link the WASM or process
//! host are separate (`codeless-plugin-host-wasm`,
//! `codeless-plugin-host-process`) and host-only. A future Phase-6
//! mobile shell that wants in-app trusted plugins compiles against
//! this SDK alone -- the dependency direction is enforced by R1 +
//! the iOS/Android cargo-check matrix.

// codeless-ported-from: rubix-workspace/extension-sdk/extensions-sdk/src/lib.rs@HEAD
//
// The guard shape is copied verbatim from rubix's mutually-exclusive
// feature pattern: two flavours of the `register!` macro both
// expanding into the same translation unit produce conflicting
// symbol exports (inventory submission vs. `wit-bindgen` exports vs.
// gRPC server stubs), and the resulting linker error is unreadable.
// A compile-time check at the SDK root surfaces the real cause.
#[cfg(any(
    all(feature = "builtin", feature = "wasm"),
    all(feature = "builtin", feature = "process"),
    all(feature = "wasm", feature = "process"),
))]
compile_error!(
    "codeless-plugin-sdk: features `builtin`, `wasm`, and `process` are mutually \
     exclusive -- enable exactly one on each consuming crate (default is `builtin`)"
);

mod behavior;
mod ctx;
mod error;
mod manifest;
mod register;
mod tier;

pub use behavior::{ToolBehavior, ToolMeta};
pub use ctx::ToolCtx;
pub use error::ToolError;
pub use manifest::Manifest;
pub use tier::Tier;

// Re-export the proc-macro under the same root so authors write
// `use codeless_plugin_sdk::Tool;` once and get the derive plus the
// trait. The derive lives in a sibling crate because proc-macros
// must be their own crate type.
pub use codeless_plugin_sdk_macros::Tool;

#[doc(hidden)]
pub mod __private {
    // Re-exports the `#[derive(Tool)]` expansion reaches for so the
    // generated code does not assume the consuming crate has these
    // in scope. Not part of the stable API surface.
    pub use schemars;
    pub use serde_json;
}
