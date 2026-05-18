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
//! - [`bindings`]                 -- host-side
//!   `wasmtime::component::bindgen!` output for the default `plugin`
//!   world (no-IO; exports `tool` only).
//! - [`bindings_with_attachments`] -- bindings for the
//!   `plugin-with-attachments` world. Used to register the
//!   `codeless:attachments/store` interface in the per-plugin
//!   linker when the manifest's `[runtimes.capabilities]
//!   attachments` set is non-empty.
//! - [`bindings_with_fs`]         -- bindings for the
//!   `plugin-with-fs` world; same role for the
//!   `codeless:fs/probe` interface.
//! - [`runtime`]                  -- engine + per-plugin linker
//!   construction; the load-bearing carrier of the default-deny
//!   capability sandbox.
//! - [`plugin`]                   -- one loaded component plus
//!   per-call dispatch.
//! - [`adapter`]                  -- the [`RuntimeAdapter`] impl
//!   that plugs into `codeless-tools`' table.
//! - [`policy`]                   -- [`HostPolicy`] fuel / memory /
//!   deadline caps and their override rules.
//! - [`capabilities`]             -- [`Capabilities`] set parsed
//!   from `[runtimes.capabilities]` and threaded into
//!   [`WasmPlugin`] at load time.
//! - [`attachments`]              -- [`AttachmentStore`] trait + an
//!   in-memory impl used by the e2e tests.
//! - [`error`]                    -- [`HostError`] vocabulary.
//!
//! Mobile-safety: this crate transitively pulls in `wasmtime` +
//! `wasmtime-wasi` + `cranelift`. It is host-only and must never
//! appear in the dependency closure of `codeless-types`,
//! `codeless-rpc`, `codeless-client`, or any future
//! `codeless-tauri-mobile`. The R1 grep in CI is the canary.
//!
//! [`RuntimeAdapter`]: codeless_tools::runtime_adapter::RuntimeAdapter

pub mod adapter;
pub mod attachments;
pub mod capabilities;
pub mod error;
pub mod plugin;
pub mod policy;
pub mod runtime;

pub use adapter::WasmAdapter;
pub use attachments::{AttachmentError, AttachmentStore, InMemoryAttachmentStore};
pub use capabilities::Capabilities;
pub use error::{HostError, Result};
pub use plugin::{AdapterRequest, LoadOptions, WasmPlugin};
pub use policy::{HostPolicy, HostPolicyOverride, PolicyError};
pub use runtime::{AttachmentStoreHandle, PluginStoreState, WasmRuntime};

/// Host-side bindings for the default `plugin` world: exports
/// `codeless:tool/tool`, imports nothing. Used for the typed
/// `instantiate_async` + `codeless_tool_tool()` accessors. Plugins
/// built against richer worlds (`plugin-with-attachments`,
/// `plugin-with-fs`) instantiate through the **same** typed
/// surface because component-model instantiation is structural --
/// the only host-side requirement is that the linker satisfies
/// every import the component declares, which the per-plugin
/// linker built from [`Capabilities`] handles.
pub mod bindings {
    wasmtime::component::bindgen!({
        path: "../codeless-tool-wit/wit",
        world: "plugin",
        async: true,
    });
}

/// Bindings for the `plugin-with-attachments` world. The only piece
/// of generated code consumed outside the bindgen module is
/// `codeless::attachments::store::add_to_linker` (called from
/// [`WasmRuntime::build_linker`]) and the `Host` trait
/// [`PluginStoreState`] implements. Generating against the full
/// world (rather than just the interface) is the supported wasmtime
/// API in v30.
pub mod bindings_with_attachments {
    wasmtime::component::bindgen!({
        path: "../codeless-tool-wit/wit",
        world: "plugin-with-attachments",
        async: true,
    });
}

/// Bindings for the `plugin-with-fs` world. Same shape as
/// [`bindings_with_attachments`].
pub mod bindings_with_fs {
    wasmtime::component::bindgen!({
        path: "../codeless-tool-wit/wit",
        world: "plugin-with-fs",
        async: true,
    });
}
