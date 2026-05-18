//! Wasmtime engine + per-plugin linker construction.
//!
//! One [`WasmRuntime`] per `codeless-server` process. The engine is
//! expensive to build (cranelift codegen pipeline init); plugins are
//! cheap (`Component::from_binary` is parse + validate, no codegen
//! per call). Per-call instantiation lands in
//! [`crate::WasmPlugin::call`].
//!
//! Capability sandbox per `PLUGIN-WASM.md § Capability sandbox`:
//! the per-plugin linker is built fresh inside [`crate::WasmPlugin::load`]
//! from the manifest's [`Capabilities`] set, so the linker contains
//! **only** what the manifest authorises. The default-deny posture
//! is the empty `Capabilities` -- a plugin loaded against it whose
//! component imports any host-implemented interface fails at
//! instantiation, not at the call boundary.

use std::sync::Arc;

use wasmtime::component::Linker;
use wasmtime::StoreLimits;
use wasmtime::{Config, Engine};
use wasmtime_wasi::{IoView, ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::attachments::{AttachmentError as HostAttachmentError, AttachmentStore};
use crate::bindings_with_attachments;
use crate::bindings_with_fs;
use crate::capabilities::Capabilities;
use crate::error::{HostError, Result};

/// Thread-safe handle to the host attachment store shared by every
/// [`crate::WasmPlugin`] that imports `codeless:attachments/store`.
/// Stored as `Arc<dyn AttachmentStore>` so a single in-memory
/// fixture can back many simultaneous plugin instances during the
/// `plugin_wasm_e2e` round-trip test.
pub type AttachmentStoreHandle = Arc<dyn AttachmentStore>;

/// Per-store state every `WasmPlugin` instance is constructed with.
///
/// Default-deny WASI ctx (no preopens, env, stdio, sockets) per
/// `PLUGIN-WASM.md § Capability sandbox`; codeless-side host
/// interfaces (`codeless:attachments/store`, `codeless:fs/probe`)
/// carry the capability data they need on the state directly.
pub struct PluginStoreState {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    pub limits: StoreLimits,
    /// `tool-call.thread-id` for the in-flight call. The host impls
    /// read this directly so a guest cannot fake cross-thread scope.
    pub thread_id: String,
    pub attachments: Option<AttachmentStoreHandle>,
    pub fs_allow: Vec<String>,
    pub attachments_read: bool,
    pub attachments_write: bool,
}

impl IoView for PluginStoreState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiView for PluginStoreState {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl PluginStoreState {
    pub fn new(
        limits: StoreLimits,
        thread_id: String,
        capabilities: &Capabilities,
        attachments: Option<AttachmentStoreHandle>,
    ) -> Self {
        Self {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            limits,
            thread_id,
            attachments,
            fs_allow: capabilities.fs_allow.clone(),
            attachments_read: capabilities.attachments_read,
            attachments_write: capabilities.attachments_write,
        }
    }
}

// ----- Host impls of the codeless-side WIT interfaces ---------------

impl bindings_with_attachments::codeless::attachments::store::Host for PluginStoreState {
    async fn read(
        &mut self,
        id: String,
    ) -> std::result::Result<
        Vec<u8>,
        bindings_with_attachments::codeless::attachments::store::AttachmentError,
    > {
        if !self.attachments_read {
            return Err(map_attachment_err(HostAttachmentError::Denied));
        }
        let store = self.attachments.as_ref().ok_or_else(|| {
            map_attachment_err(HostAttachmentError::Io(
                "host attachment store was not configured".into(),
            ))
        })?;
        store
            .read(&self.thread_id, &id)
            .await
            .map_err(map_attachment_err)
    }

    async fn mint(
        &mut self,
        filename: String,
        bytes: Vec<u8>,
    ) -> std::result::Result<
        String,
        bindings_with_attachments::codeless::attachments::store::AttachmentError,
    > {
        if !self.attachments_write {
            return Err(map_attachment_err(HostAttachmentError::Denied));
        }
        let store = self.attachments.as_ref().ok_or_else(|| {
            map_attachment_err(HostAttachmentError::Io(
                "host attachment store was not configured".into(),
            ))
        })?;
        store
            .mint(&self.thread_id, &filename, &bytes)
            .await
            .map_err(map_attachment_err)
    }
}

fn map_attachment_err(
    e: HostAttachmentError,
) -> bindings_with_attachments::codeless::attachments::store::AttachmentError {
    use bindings_with_attachments::codeless::attachments::store::AttachmentError as W;
    match e {
        HostAttachmentError::Denied => W::Denied,
        HostAttachmentError::NotFound => W::NotFound,
        HostAttachmentError::InvalidId(s) => W::InvalidId(s),
        HostAttachmentError::Io(s) => W::Io(s),
    }
}

impl bindings_with_fs::codeless::fs::probe::Host for PluginStoreState {
    async fn read_file(
        &mut self,
        path: String,
    ) -> std::result::Result<Vec<u8>, bindings_with_fs::codeless::fs::probe::FsError> {
        use bindings_with_fs::codeless::fs::probe::FsError as W;
        let allowed = self.fs_allow.iter().any(|p| path.starts_with(p));
        if !allowed {
            return Err(W::Denied);
        }
        std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => W::NotFound,
            _ => W::Io(format!("{e}")),
        })
    }
}

/// Process-wide Wasmtime runtime. Holds only the [`Engine`]; per-
/// plugin linkers are constructed in [`crate::WasmPlugin::load`] so
/// the capability set scoped to a single plugin lives on that plugin
/// alone.
pub struct WasmRuntime {
    engine: Engine,
}

impl WasmRuntime {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.async_support(true);
        config.consume_fuel(true);
        config.wasm_bulk_memory(true);
        config.wasm_multi_value(true);
        config.wasm_multi_memory(true);
        let engine = Engine::new(&config).map_err(|e| HostError::Engine(format!("{e:#}")))?;
        Ok(Self { engine })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Build a per-plugin [`Linker`] populated with exactly the host
    /// imports the manifest's [`Capabilities`] authorise. The
    /// linker is the load-bearing carrier of the default-deny
    /// posture: an interface that the capability set does not list
    /// is simply not added, so a component that imports it fails to
    /// instantiate against this linker -- `plugin_wasm_e2e::
    /// wasm_plugin_cannot_open_host_file` exercises that failure
    /// mode.
    pub fn build_linker(&self, caps: &Capabilities) -> Result<Linker<PluginStoreState>> {
        let mut linker: Linker<PluginStoreState> = Linker::new(&self.engine);
        if caps.link_attachments() {
            bindings_with_attachments::codeless::attachments::store::add_to_linker(
                &mut linker,
                |s| s,
            )
            .map_err(|e| HostError::Engine(format!("attachments linker: {e:#}")))?;
        }
        if caps.link_fs_probe() {
            bindings_with_fs::codeless::fs::probe::add_to_linker(&mut linker, |s| s)
                .map_err(|e| HostError::Engine(format!("fs probe linker: {e:#}")))?;
        }
        Ok(linker)
    }
}
