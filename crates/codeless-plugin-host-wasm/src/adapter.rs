//! [`WasmAdapter`] -- the [`RuntimeAdapter`] impl that lives in
//! `codeless-tools` for WASM-flavour plugins.
//!
//! Per OQ-WASM-1 (resolved 2026-05-18, stage 1 of
//! plugin-substrate-runtimes): the dispatch table is in
//! `codeless-tools`; the host-only adapter that fills its rows for
//! WASM plugins is here. A future `codeless-plugin-host-process`
//! crate will hold the equivalent `ProcessAdapter` against the same
//! trait.

use std::sync::Arc;

use async_trait::async_trait;

use codeless_tools::runtime_adapter::{
    AdapterToolManifest, RuntimeAdapter, ToolCallOutcome, ToolCallRequest,
};

use crate::plugin::{AdapterRequest, WasmPlugin};

/// Thin shared-pointer wrapper around a loaded [`WasmPlugin`].
///
/// Stored as `Arc<dyn RuntimeAdapter>` in
/// [`codeless_tools::runtime_adapter::RuntimeAdapterTable`]; one
/// adapter handles every tool the plugin contributes, so a plugin
/// exporting N tools costs one [`WasmPlugin`] and N table entries
/// pointing at the same `Arc`.
pub struct WasmAdapter {
    plugin: Arc<WasmPlugin>,
}

impl WasmAdapter {
    pub fn new(plugin: Arc<WasmPlugin>) -> Self {
        Self { plugin }
    }
}

#[async_trait]
impl RuntimeAdapter for WasmAdapter {
    fn manifests(&self) -> Vec<AdapterToolManifest> {
        self.plugin.manifests().to_vec()
    }

    async fn call(&self, req: ToolCallRequest) -> ToolCallOutcome {
        self.plugin
            .call(AdapterRequest {
                tool_id: &req.tool_id,
                args_json: &req.args_json,
                thread_id: &req.thread_id,
            })
            .await
    }
}
