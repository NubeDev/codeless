//! Runtime-adapter table -- the dispatch seam between a tool id and
//! the runtime flavour (builtin Rust, WASM component, future
//! process supervisor) that owns it.
//!
//! Resolved 2026-05-18 (plugin-substrate-runtimes stage 1) under
//! OQ-WASM-1 in `DOCS/plugins/PLUGIN-WASM.md`: the **table** stays
//! here in `codeless-tools` (so a future mobile shell that adopts
//! trusted-builtin plugins reuses the same dispatch shape); the
//! **adapter impls** live in their respective host-only crates
//! (`codeless-plugin-host-wasm`, future `codeless-plugin-host-
//! process`) behind Cargo features.
//!
//! Mobile-safety contract: every type in this module is pure data
//! plus `async_trait`. No `wasmtime`, no `tokio::process`, no host
//! handles cross the trait. The iOS/Android cargo-check matrix
//! catches a leak; if one appears, the fallback documented under
//! OQ-WASM-1 is to move this module to a new mobile-safe
//! `codeless-plugin-dispatch` crate without touching the trait
//! shape.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

/// One dispatchable invocation routed through a [`RuntimeAdapter`].
///
/// Field shape mirrors `tool-call` in `crates/codeless-tool-wit/wit/
/// tool.wit` so the WASM-flavour adapter is a thin field-by-field
/// copy at the boundary, not a translation. The builtin-flavour
/// adapter deserialises `args_json` straight into the plugin's
/// `<T as ToolBehavior>::Args` type. `thread_id` is carried because
/// attachment-scoped capabilities (PLUGIN-SUBSTRATE.md item 7) need
/// it before any tool code runs.
#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub tool_id: String,
    /// JSON-encoded args object. Validated against the tool's
    /// declared input schema before this request is built; the
    /// adapter therefore treats it as opaque payload, not as
    /// something to re-validate.
    pub args_json: String,
    pub thread_id: String,
}

/// Outcome of [`RuntimeAdapter::call`]. Mirrors `tool-result` in
/// `tool.wit` -- `Ok` carries the JSON-encoded output (validated
/// against the tool's output schema by the dispatcher *after* it
/// crosses back), `Err` carries the structured error the runtime
/// surfaces to the agent loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallOutcome {
    Ok(String),
    Err(AdapterError),
}

/// Structured failure from a [`RuntimeAdapter::call`]. The string
/// `code` is the same vocabulary the WASM `tool-error.code` field
/// carries (`"limit-exceeded"`, `"invalid-args"`, `"failed"`,
/// `"cancelled"`, ...); the dispatcher maps the small known set to
/// its own typed reasons and passes anything else through as
/// `"plugin-error"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterError {
    pub code: String,
    pub message: String,
    /// True iff the dispatcher should consider an automatic retry.
    /// Defaults to false at the adapter boundary; the typed builtin
    /// shim sets it from `ToolError::Retryable`, the WASM adapter
    /// reads it straight off the WIT field.
    pub retryable: bool,
}

impl AdapterError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

/// One tool the adapter advertises. Mirrors `tool-manifest` in
/// `tool.wit` -- ids, tier label, JSON-Schema strings -- so a
/// builtin adapter and a wasm adapter exporting the same plugin
/// source produce byte-identical manifests on the wire. Stored as
/// `String` (not `&'static str`) because the wasm adapter reads
/// them out of an instantiated guest at load time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterToolManifest {
    pub id: String,
    pub description: String,
    /// JSON-encoded JSON Schema for the args object.
    pub input_schema: String,
    /// JSON-encoded JSON Schema for the return value.
    pub output_schema: String,
    /// Risk tier as the lowercase string the WIT enum and the
    /// `plugin.toml` manifest both use (`"read"` / `"write"` /
    /// `"destructive"`). Carried as a string to keep this trait
    /// independent of `codeless-plugin-sdk::Tier`; the dispatcher
    /// is the one that re-parses into a typed enum.
    pub tier: String,
}

/// Dispatch seam from a tool id to the runtime flavour that owns
/// it. One impl per flavour:
///
/// - builtin -- thin shim wrapping the existing `codeless-tools::
///   Tool` registry once stage 5 swings the notes plugin onto the
///   SDK;
/// - wasm    -- `codeless-plugin-host-wasm::WasmAdapter`, holding
///   the Wasmtime component and per-call instantiation;
/// - process -- reserved for PLUGIN-PROCESS.md item 11, no impl
///   today.
///
/// The trait is intentionally minimal: a description list and a
/// call. Anything richer (streaming events, mid-call cancellation
/// queries) lands as a new method behind a default impl, not as a
/// breaking change.
#[async_trait]
pub trait RuntimeAdapter: Send + Sync + 'static {
    /// Manifests for every tool this adapter dispatches. Called at
    /// registry-load time so the host can populate
    /// [`RuntimeAdapterTable`] without instantiating a wasm guest
    /// on every lookup.
    fn manifests(&self) -> Vec<AdapterToolManifest>;

    /// Invoke a tool. `req.tool_id` is guaranteed to be one of the
    /// ids in [`Self::manifests`]; an adapter that receives an
    /// unknown id should return
    /// [`AdapterError`] with code `"tool-not-found"`.
    async fn call(&self, req: ToolCallRequest) -> ToolCallOutcome;
}

/// Mobile-safe registry of [`RuntimeAdapter`] trait objects keyed
/// by tool id (`<plugin_id>.<tool_id>`).
///
/// Host-only adapters (`WasmAdapter`, future
/// `ProcessAdapter`) plug in through `register_plugin`, which
/// expands each adapter's `manifests()` list into per-tool-id table
/// entries pointing at the same `Arc<dyn RuntimeAdapter>` so a
/// plugin contributing N tools costs one shared adapter, not N.
#[derive(Default)]
pub struct RuntimeAdapterTable {
    entries: HashMap<String, AdapterEntry>,
}

/// One entry in [`RuntimeAdapterTable`]. Carries the adapter handle
/// plus the manifest, so a lookup at dispatch time does not need to
/// reach back into the adapter for argument-schema information.
#[derive(Clone)]
pub struct AdapterEntry {
    pub manifest: AdapterToolManifest,
    pub adapter: Arc<dyn RuntimeAdapter>,
}

impl RuntimeAdapterTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register every tool an adapter advertises. Returns the list
    /// of `(tool_id, previous)` pairs for entries that collided so
    /// the caller can fail load-time on a double-registration --
    /// `codeless-server`'s two-phase scan (lifted from rubix
    /// `extensions-host::registry`) treats a non-empty list as a
    /// fatal manifest error.
    pub fn register_plugin(&mut self, adapter: Arc<dyn RuntimeAdapter>) -> Vec<TableCollision> {
        let manifests = adapter.manifests();
        let mut collisions = Vec::new();
        for manifest in manifests {
            let id = manifest.id.clone();
            let entry = AdapterEntry {
                manifest,
                adapter: Arc::clone(&adapter),
            };
            if let Some(prev) = self.entries.insert(id.clone(), entry) {
                collisions.push(TableCollision {
                    tool_id: id,
                    previous: prev.manifest,
                });
            }
        }
        collisions
    }

    pub fn get(&self, tool_id: &str) -> Option<&AdapterEntry> {
        self.entries.get(tool_id)
    }

    pub fn manifests(&self) -> impl Iterator<Item = &AdapterToolManifest> {
        self.entries.values().map(|e| &e.manifest)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Reported back from [`RuntimeAdapterTable::register_plugin`] when
/// the same tool id is contributed twice. The dispatcher decides
/// whether to treat this as a fatal load error (today: yes); the
/// table itself never panics or silently drops, so a test can build
/// the same registry shape and assert on the collisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCollision {
    pub tool_id: String,
    pub previous: AdapterToolManifest,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubAdapter {
        manifest: AdapterToolManifest,
        reply: ToolCallOutcome,
    }

    #[async_trait]
    impl RuntimeAdapter for StubAdapter {
        fn manifests(&self) -> Vec<AdapterToolManifest> {
            vec![self.manifest.clone()]
        }

        async fn call(&self, _req: ToolCallRequest) -> ToolCallOutcome {
            self.reply.clone()
        }
    }

    fn manifest(id: &str) -> AdapterToolManifest {
        AdapterToolManifest {
            id: id.into(),
            description: String::new(),
            input_schema: "{}".into(),
            output_schema: "{}".into(),
            tier: "read".into(),
        }
    }

    #[tokio::test]
    async fn register_routes_call_to_adapter() {
        let mut table = RuntimeAdapterTable::new();
        let stub = Arc::new(StubAdapter {
            manifest: manifest("notes.append"),
            reply: ToolCallOutcome::Ok("{\"ok\":true}".into()),
        });
        let collisions = table.register_plugin(stub);
        assert!(collisions.is_empty());
        assert_eq!(table.len(), 1);

        let entry = table.get("notes.append").expect("registered");
        let out = entry
            .adapter
            .call(ToolCallRequest {
                tool_id: "notes.append".into(),
                args_json: "{}".into(),
                thread_id: "t1".into(),
            })
            .await;
        assert_eq!(out, ToolCallOutcome::Ok("{\"ok\":true}".into()));
    }

    #[tokio::test]
    async fn double_registration_reports_collision() {
        let mut table = RuntimeAdapterTable::new();
        let first = Arc::new(StubAdapter {
            manifest: manifest("dup"),
            reply: ToolCallOutcome::Ok("1".into()),
        });
        let second = Arc::new(StubAdapter {
            manifest: manifest("dup"),
            reply: ToolCallOutcome::Ok("2".into()),
        });
        assert!(table.register_plugin(first).is_empty());
        let collisions = table.register_plugin(second);
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].tool_id, "dup");
    }

    #[test]
    fn adapter_error_builder_defaults_non_retryable() {
        let err = AdapterError::new("limit-exceeded", "fuel exhausted");
        assert!(!err.retryable);
        let err = err.retryable();
        assert!(err.retryable);
    }
}
