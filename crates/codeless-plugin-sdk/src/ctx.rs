/// Per-call context handed to [`crate::ToolBehavior::call`].
///
/// Today the type is opaque -- it carries no host handles because
/// the SDK is mobile-safe and must not pull in `codeless-tools` (the
/// host-only crate that owns the live `ToolCtx`). The per-flavour
/// adapter layer (builtin shim in `codeless-tools`, wasm host in
/// `codeless-plugin-host-wasm`) translates between its real context
/// and this opaque value at the SDK boundary.
///
/// As the substrate grows the typed capabilities listed in
/// PLUGIN-WASM.md ("Capability sandbox") -- attachments-read,
/// attachments-write, kv -- they land here as inherent methods
/// behind feature gates. Keeping the type a struct rather than a
/// trait means adding a method is a non-breaking change for plugin
/// authors.
pub struct ToolCtx {
    // Private field so plugin code cannot construct a `ToolCtx`
    // directly -- only the per-flavour adapter is allowed to,
    // through `ToolCtx::__from_host()` once that ships.
    _seal: (),
}

impl ToolCtx {
    /// Constructor for the per-flavour adapter. Not part of the
    /// stable plugin-author surface; reachable only because Rust has
    /// no friend visibility -- the docs make the contract explicit.
    #[doc(hidden)]
    pub fn __from_host_seal() -> Self {
        Self { _seal: () }
    }
}
