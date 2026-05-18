//! Wasmtime engine + WASI-p2 component-model linker prototype.
//!
//! One [`WasmRuntime`] per `codeless-server` process. The engine is
//! expensive to build (cranelift codegen pipeline init); plugins are
//! cheap (`Component::from_binary` is parse + validate, no codegen
//! per call). Per-call instantiation lands in
//! [`crate::WasmPlugin::call`] -- the runtime here owns only the
//! shared engine and the linker prototype every plugin's per-call
//! [`wasmtime::Store`] reuses.

use wasmtime::component::Linker;
use wasmtime::{Config, Engine};
use wasmtime_wasi::{IoView, ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use wasmtime::StoreLimits;

use crate::error::{HostError, Result};

/// Per-store state every `WasmPlugin` instance is constructed with.
///
/// - `wasi` is the per-call WASI context. Default-deny per
///   `PLUGIN-WASM.md § Capability sandbox`: stdio captured, no fs,
///   no http, no env, no args. Stage 6 grants the capability set
///   the plugin's `[runtimes.capabilities]` block lists; today only
///   the empty ctx is reachable.
/// - `table` is the WASI resource table (required by `WasiView`).
/// - `limits` carries the [`StoreLimits`] this call runs under;
///   wired to the store through [`wasmtime::Store::limiter`] so
///   linear-memory growth past [`crate::HostPolicy::memory_max_bytes`]
///   aborts the call instead of silently allocating.
pub struct PluginStoreState {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    pub limits: StoreLimits,
}

// wasmtime-wasi 30 split the table accessor onto the new `IoView`
// supertrait; `WasiView` now carries only the `ctx()` method.
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
    /// Build the default-deny store state. Called from
    /// [`crate::WasmPlugin::call`] -- one fresh state per call,
    /// dropped on return, matching the per-call-instantiation rule
    /// in `PLUGIN-WASM.md § Instance lifecycle`.
    pub fn new(limits: StoreLimits) -> Self {
        // The default `WasiCtxBuilder` returns the deny-everything
        // ctx (no preopens, no env, no stdio inheritance). Stage 6
        // re-builds this through capability-specific helpers; today
        // a plugin that imports any non-default WASI interface will
        // fail to link, which is the correct behaviour.
        Self {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            limits,
        }
    }
}

/// Process-wide Wasmtime runtime for codeless plugin components.
///
/// Built once. The contained [`Engine`] is the source of truth for
/// the codegen profile every plugin's [`wasmtime::component::
/// Component`] is parsed against (async, fuel, component model);
/// the [`Linker`] is the prototype the per-call store clones from.
pub struct WasmRuntime {
    engine: Engine,
    linker: Linker<PluginStoreState>,
}

impl WasmRuntime {
    /// Build the runtime with the codeless-mandatory engine config:
    ///
    /// - `async_support(true)` so per-call instantiation cooperates
    ///   with the surrounding tokio reactor instead of blocking a
    ///   worker thread.
    /// - `consume_fuel(true)` so [`crate::HostPolicy::fuel`] is
    ///   actually enforced -- without this the fuel cap silently
    ///   no-ops.
    /// - `epoch_interruption(false)` -- the wall-clock deadline is
    ///   delivered through a tokio timeout instead of a wasmtime
    ///   epoch tick; one mechanism is easier to reason about than
    ///   two.
    ///
    /// Default-built. A custom-config constructor lands the day a
    /// real reason appears; we resist pre-emptive knobs.
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.async_support(true);
        config.consume_fuel(true);
        // The component-model embedding plus the rustc-emitted core
        // modules need bulk-memory, reference types, multi-value, and
        // mutable globals to validate. They are part of standardised
        // wasm and on by default in wasmtime 23, but pinning them
        // here defends against an accidental disable on a future
        // wasmtime bump.
        config.wasm_bulk_memory(true);
        config.wasm_multi_value(true);
        config.wasm_multi_memory(true);
        let engine = Engine::new(&config).map_err(|e| HostError::Engine(format!("{e:#}")))?;
        let mut linker: Linker<PluginStoreState> = Linker::new(&engine);
        wasmtime_wasi::add_to_linker_async(&mut linker)
            .map_err(|e| HostError::Engine(format!("wasi linker: {e:#}")))?;
        Ok(Self { engine, linker })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn linker(&self) -> &Linker<PluginStoreState> {
        &self.linker
    }
}
