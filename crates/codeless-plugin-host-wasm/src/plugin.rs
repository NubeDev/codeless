//! One loaded `.wasm` plugin component and its per-call dispatch.
//!
//! A [`WasmPlugin`] is built once at `codeless-server` startup
//! (`PluginRegistry::load_plugin`, substrate item 6) from the
//! artefact path in the plugin manifest's `[[runtimes]] kind =
//! "wasm"` entry, and then dispatched against many times. The
//! component is parsed up-front (`Component::from_file`); each call
//! gets a fresh [`wasmtime::Store`], fresh fuel, fresh memory
//! limiter, and is dropped on return per the per-call-instantiation
//! rule in `PLUGIN-WASM.md § Instance lifecycle`.

use std::path::Path;
use std::sync::Arc;

use tokio::time::timeout;
use wasmtime::component::Component;
use wasmtime::{Store, StoreLimitsBuilder};

use codeless_tools::runtime_adapter::{AdapterError, AdapterToolManifest, ToolCallOutcome};

use crate::bindings::Plugin;
use crate::error::{HostError, Result};
use crate::policy::HostPolicy;
use crate::runtime::{PluginStoreState, WasmRuntime};

/// One WASM plugin artefact + the runtime it dispatches against.
///
/// The cached `manifests` field is populated at load time by
/// instantiating the component once and calling its `describe()`
/// export. Subsequent dispatches do not re-instantiate to read the
/// manifest -- that would couple the registry's hot-path lookup to
/// guest code.
pub struct WasmPlugin {
    runtime: Arc<WasmRuntime>,
    component: Component,
    policy: HostPolicy,
    manifests: Vec<AdapterToolManifest>,
}

impl WasmPlugin {
    /// Load a plugin from a `.wasm` file on disk, eagerly compiling
    /// the component and reading its manifest list. A failure here
    /// is a plugin-load failure (substrate `LoadedPlugin::Failed`),
    /// not a per-call failure.
    pub async fn load(
        runtime: Arc<WasmRuntime>,
        path: impl AsRef<Path>,
        policy: HostPolicy,
    ) -> Result<Self> {
        let component = Component::from_file(runtime.engine(), path)
            .map_err(|e| HostError::InvalidComponent(format!("{e:#}")))?;
        let manifests = describe(&runtime, &component, &policy).await?;
        Ok(Self {
            runtime,
            component,
            policy,
            manifests,
        })
    }

    /// Build a plugin around an already-parsed [`Component`].
    /// Useful for tests that synthesise a component in-memory
    /// (`Component::new(&engine, wat_source)`); the disk-loaded
    /// path goes through [`Self::load`].
    pub async fn from_component(
        runtime: Arc<WasmRuntime>,
        component: Component,
        policy: HostPolicy,
    ) -> Result<Self> {
        let manifests = describe(&runtime, &component, &policy).await?;
        Ok(Self {
            runtime,
            component,
            policy,
            manifests,
        })
    }

    pub fn manifests(&self) -> &[AdapterToolManifest] {
        &self.manifests
    }

    pub fn policy(&self) -> HostPolicy {
        self.policy
    }

    /// Dispatch one tool call. Per `PLUGIN-WASM.md § Instance
    /// lifecycle`: fresh store, fresh fuel, fresh limiter, dropped
    /// on return. The three caps are enforced inside the store
    /// (`set_fuel`, `limiter`) plus outside it
    /// ([`tokio::time::timeout`]); the call returns
    /// [`ToolCallOutcome::Err`] with code `"limit-exceeded"` for
    /// any of the three rather than propagating a typed
    /// [`HostError`] up to the dispatcher, because the dispatcher
    /// surface (the runtime-adapter trait) is shared with the
    /// non-WASM flavours that have no concept of fuel.
    pub async fn call(&self, req: AdapterRequest<'_>) -> ToolCallOutcome {
        match self.call_inner(req).await {
            Ok(outcome) => outcome,
            Err(HostError::LimitExceeded { reason }) => ToolCallOutcome::Err(AdapterError::new(
                "limit-exceeded",
                format!("wasm plugin exceeded {reason} cap"),
            )),
            Err(other) => {
                ToolCallOutcome::Err(AdapterError::new("plugin-error", format!("{other}")))
            }
        }
    }

    async fn call_inner(&self, req: AdapterRequest<'_>) -> Result<ToolCallOutcome> {
        let mut store = build_store(&self.runtime, self.policy)?;
        let plugin = Plugin::instantiate_async(&mut store, &self.component, self.runtime.linker())
            .await
            .map_err(|e| HostError::InvalidComponent(format!("instantiate: {e:#}")))?;

        let call_req = crate::bindings::exports::codeless::tool::tool::ToolCall {
            tool_id: req.tool_id.to_string(),
            args_json: req.args_json.to_string(),
            thread_id: req.thread_id.to_string(),
        };

        let fut = plugin.codeless_tool_tool().call_call(&mut store, &call_req);
        let result = match timeout(self.policy.deadline, fut).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                if let Some(reason) = classify_trap(&e) {
                    return Err(HostError::LimitExceeded { reason });
                }
                return Err(HostError::GuestViolatedAbi(format!("call_call: {e:#}")));
            }
            Err(_) => return Err(HostError::LimitExceeded { reason: "deadline" }),
        };

        Ok(map_tool_result(result))
    }
}

/// Borrowed request shape so the per-call dispatch path does not
/// allocate three `String`s per call when the caller already owns
/// them on the stack. The adapter at [`crate::adapter`] forwards
/// from the owned [`codeless_tools::runtime_adapter::ToolCallRequest`]
/// to this borrow.
pub struct AdapterRequest<'a> {
    pub tool_id: &'a str,
    pub args_json: &'a str,
    pub thread_id: &'a str,
}

fn build_store(runtime: &WasmRuntime, policy: HostPolicy) -> Result<Store<PluginStoreState>> {
    // `usize::try_from` because `StoreLimitsBuilder::memory_size`
    // takes `usize`; the policy holds `u64` to fail-fast at config
    // parse on a 32-bit host. See `policy.rs` for the rationale.
    let mem = usize::try_from(policy.memory_max_bytes).map_err(|_| {
        HostError::Engine("memory_max_bytes exceeds usize::MAX on this host".into())
    })?;
    let limits = StoreLimitsBuilder::new().memory_size(mem).build();
    let state = PluginStoreState::new(limits);
    let mut store = Store::new(runtime.engine(), state);
    store.limiter(|s| &mut s.limits);
    store
        .set_fuel(policy.fuel)
        .map_err(|e| HostError::Engine(format!("set_fuel: {e:#}")))?;
    Ok(store)
}

async fn describe(
    runtime: &WasmRuntime,
    component: &Component,
    policy: &HostPolicy,
) -> Result<Vec<AdapterToolManifest>> {
    let mut store = build_store(runtime, *policy)?;
    let plugin = Plugin::instantiate_async(&mut store, component, runtime.linker())
        .await
        .map_err(|e| HostError::InvalidComponent(format!("instantiate-for-describe: {e:#}")))?;
    let list = plugin
        .codeless_tool_tool()
        .call_describe(&mut store)
        .await
        .map_err(|e| HostError::GuestViolatedAbi(format!("describe: {e:#}")))?;
    Ok(list.into_iter().map(map_manifest).collect())
}

fn map_manifest(
    m: crate::bindings::exports::codeless::tool::tool::ToolManifest,
) -> AdapterToolManifest {
    AdapterToolManifest {
        id: m.id,
        description: m.description,
        input_schema: m.input_schema,
        output_schema: m.output_schema,
        tier: map_tier(m.tier).into(),
    }
}

fn map_tier(t: crate::bindings::exports::codeless::tool::tool::Tier) -> &'static str {
    use crate::bindings::exports::codeless::tool::tool::Tier as T;
    match t {
        T::Read => "read",
        T::Write => "write",
        T::Destructive => "destructive",
    }
}

fn map_tool_result(
    r: crate::bindings::exports::codeless::tool::tool::ToolResult,
) -> ToolCallOutcome {
    use crate::bindings::exports::codeless::tool::tool::ToolResult as R;
    match r {
        R::Ok(s) => ToolCallOutcome::Ok(s),
        R::Err(e) => ToolCallOutcome::Err(AdapterError {
            code: e.code,
            message: e.message,
            retryable: e.retryable,
        }),
    }
}

/// Map a wasmtime trap to one of the limit-exceeded reasons we
/// recognise. Other traps fall through to
/// [`HostError::GuestViolatedAbi`]. The matching is by trap code
/// rather than string because the formatted message is not stable
/// across wasmtime versions -- a future refactor that changes the
/// wording must not silently turn a fuel trap into a
/// `plugin-error`.
fn classify_trap(err: &wasmtime::Error) -> Option<&'static str> {
    let trap = err.downcast_ref::<wasmtime::Trap>()?;
    match trap {
        wasmtime::Trap::OutOfFuel => Some("fuel"),
        // Wasmtime reports `memory_max_bytes` exceeded as a
        // `Trap::MemoryOutOfBounds` once the store limiter aborts
        // a growth; that overlaps with genuine out-of-bounds
        // accesses, so we conservatively map it to `memory`. The
        // distinction does not matter to the dispatcher -- both
        // are `limit-exceeded` to the agent loop.
        wasmtime::Trap::MemoryOutOfBounds => Some("memory"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_component_bytes_fail_load() {
        let runtime = Arc::new(WasmRuntime::new().expect("engine builds"));
        let tmp = tempfile_bytes(b"not a wasm component");
        match WasmPlugin::load(runtime, &tmp, HostPolicy::defaults()).await {
            Err(HostError::InvalidComponent(_)) => {}
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("garbage bytes parsed as a component"),
        }
    }

    /// `Trap::OutOfFuel` translates straight to the `fuel` reason
    /// the adapter surfaces to the dispatcher. Pinned as a unit
    /// test so a wasmtime upgrade that renames `OutOfFuel` lights
    /// up here before stage 7's end-to-end fuel test runs.
    #[test]
    fn classify_trap_maps_known_variants() {
        let fuel = wasmtime::Error::from(wasmtime::Trap::OutOfFuel);
        assert_eq!(classify_trap(&fuel), Some("fuel"));
        let mem = wasmtime::Error::from(wasmtime::Trap::MemoryOutOfBounds);
        assert_eq!(classify_trap(&mem), Some("memory"));
        let other = wasmtime::Error::from(wasmtime::Trap::UnreachableCodeReached);
        assert_eq!(classify_trap(&other), None);
    }

    fn tempfile_bytes(bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("codeless-wasm-test-{}.wasm", std::process::id()));
        std::fs::write(&path, bytes).expect("write tempfile");
        path
    }
}
