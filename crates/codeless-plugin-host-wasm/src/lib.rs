//! WASM-flavour plugin host (PLUGIN-SUBSTRATE item 9,
//! `DOCS/plugins/PLUGIN-WASM.md`).
//!
//! Load a `.wasm` component built with
//! `cargo build --target wasm32-wasip2` against the
//! [`codeless-tool-wit`](codeless_tool_wit) ABI; expose it to
//! `codeless-tools`' runtime-adapter table as a [`WasmAdapter`];
//! enforce fuel / memory / wall-clock caps per call.
//!
//! Crate layout:
//!
//! - [`bindings`]   -- host-side `wasmtime::component::bindgen!`
//!   output, generated at macro expansion against
//!   `crates/codeless-tool-wit/wit/tool.wit`. This is the *host*
//!   generator and is separate from the guest bindings committed
//!   in-tree under `codeless-tool-wit/src/bindings.rs` (OQ-WASM-2).
//! - [`runtime`]    -- the process-wide [`WasmRuntime`]: engine +
//!   linker prototype.
//! - [`plugin`]     -- one loaded component plus per-call dispatch.
//! - [`adapter`]    -- the [`RuntimeAdapter`] impl that plugs into
//!   `codeless-tools`' table.
//! - [`policy`]     -- [`HostPolicy`] fuel / memory / deadline
//!   caps and their override rules.
//! - [`error`]      -- [`HostError`] vocabulary.
//!
//! Mobile-safety: this crate transitively pulls in `wasmtime` +
//! `wasmtime-wasi` + `cranelift`. It is host-only and must never
//! appear in the dependency closure of `codeless-types`,
//! `codeless-rpc`, `codeless-client`, or any future
//! `codeless-tauri-mobile`. The R1 grep in CI is the canary.
//!
//! [`RuntimeAdapter`]: codeless_tools::runtime_adapter::RuntimeAdapter

pub mod adapter;
pub mod error;
pub mod plugin;
pub mod policy;
pub mod runtime;

pub use adapter::WasmAdapter;
pub use error::{HostError, Result};
pub use plugin::{AdapterRequest, WasmPlugin};
pub use policy::{HostPolicy, HostPolicyOverride, PolicyError};
pub use runtime::{PluginStoreState, WasmRuntime};

/// Host-side bindings for `codeless:tool@0.1.0`, generated at
/// macro-expansion time. Separate from the guest bindings in
/// `codeless-tool-wit/src/bindings.rs`: those are CABI glue for the
/// plugin author (committed in-tree per OQ-WASM-2); this is the
/// host-side typed accessor surface (regenerated every build, so
/// no in-tree diff hides ABI drift on the host side).
///
/// `async: true` because the entire host loop is tokio-based; the
/// per-call dispatch in [`plugin::WasmPlugin::call`] cooperates
/// with the surrounding reactor rather than blocking a worker.
pub mod bindings {
    wasmtime::component::bindgen!({
        path: "../codeless-tool-wit/wit",
        world: "plugin",
        async: true,
    });
}
