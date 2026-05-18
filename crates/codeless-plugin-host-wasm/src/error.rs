//! Failure modes the host loader surfaces.
//!
//! Translated to [`codeless_tools::runtime_adapter::AdapterError`]
//! at the [`crate::WasmAdapter`] boundary so the dispatcher sees a
//! uniform vocabulary regardless of which runtime flavour produced
//! the error. The translation table is in [`crate::adapter`].

use thiserror::Error;

/// Errors a [`crate::WasmRuntime`] or [`crate::WasmPlugin`] can
/// raise. Anything host-loader-shaped lands here; the *plugin's*
/// own `tool-error` payload comes back as a successful
/// [`codeless_tools::runtime_adapter::ToolCallOutcome::Err`] and
/// never enters this enum.
#[derive(Debug, Error)]
pub enum HostError {
    /// Engine setup failed (Wasmtime config invalid, linker missing
    /// WASI imports, ...). Boot-time failure; not reachable from a
    /// per-call code path. The carried string is the formatted
    /// `wasmtime::Error` -- carried by value because
    /// `wasmtime::Error` is `anyhow::Error` in v23 and pulling
    /// `anyhow` into the public surface forces every downstream
    /// consumer to take it too.
    #[error("wasmtime engine setup failed: {0}")]
    Engine(String),

    /// A `.wasm` artefact failed to decode as a WASI-p2 component.
    #[error("plugin artefact is not a valid wasi-p2 component: {0}")]
    InvalidComponent(String),

    /// One of the per-call caps in [`crate::HostPolicy`] tripped.
    /// `reason` is `"fuel"`, `"memory"`, or `"deadline"`; the
    /// adapter maps the same vocabulary into `tool-error.code =
    /// "limit-exceeded"`.
    #[error("plugin call exceeded {reason} cap")]
    LimitExceeded { reason: &'static str },

    /// The store-level invariant the WIT `tool` interface promises
    /// was violated by the guest (e.g. returned an unknown tier
    /// discriminant after the WIT `_lift` debug check).
    #[error("plugin produced wit-invariant violation: {0}")]
    GuestViolatedAbi(String),
}

// `anyhow::Error` is the natural fall-through for wasmtime's own
// `Result` because `wasmtime::Error` is `anyhow::Error` in v23.
// The `From` impl is provided by `#[from]` above; this re-export
// keeps `crate::Result<T> = std::result::Result<T, HostError>` a
// single-arg type without leaking `anyhow` to consumers.
pub type Result<T> = std::result::Result<T, HostError>;
