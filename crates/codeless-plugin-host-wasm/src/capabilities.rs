//! Capability set granted to one loaded `WasmPlugin`.
//!
//! Resolves the `[runtimes.capabilities]` block of `plugin.toml` (see
//! `PLUGIN-WASM.md § Capability sandbox`) into a typed struct the
//! host loader uses to decide which host-implemented WIT interfaces
//! to add to the per-plugin linker. Default-deny per the doc: a
//! manifest that omits the block grants nothing, and the resulting
//! [`Capabilities::default`] keeps every interface out of the
//! linker.
//!
//! The "fail at the linker, not the call boundary" rule is what
//! gives [`Capabilities`] its load-bearing semantics: a plugin built
//! against a world that imports `codeless:attachments/store` and
//! loaded without the `attachments` capability fails at
//! [`wasmtime::component::Linker::instantiate_async`], because the
//! interface is simply not present. That is the signal exercised by
//! `plugin_wasm_e2e::wasm_plugin_cannot_open_host_file`.

/// Capabilities a plugin's manifest authorises. Pure data; the
/// translation from `[runtimes.capabilities]` TOML to this struct
/// happens in `codeless-tools::plugin::manifest`, and the
/// translation from this struct to linker registrations happens in
/// [`crate::WasmPlugin::load`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether the plugin may read previously-minted attachments
    /// (`codeless:attachments/store/read`).
    pub attachments_read: bool,
    /// Whether the plugin may mint new attachments
    /// (`codeless:attachments/store/mint`).
    pub attachments_write: bool,
    /// Globs (verbatim from the manifest) listing host paths the
    /// `codeless:fs/probe.read-file` host implementation may open
    /// on behalf of the plugin. Empty -> the interface is unlinked
    /// entirely (default-deny); a non-empty list links the
    /// interface and the host implementation checks each requested
    /// path against the list before opening it.
    pub fs_allow: Vec<String>,
    /// Whether the plugin may import `wasi:clocks/wall-clock`. Off
    /// by default; the implementation ships in a later stage and
    /// today this field exists so the manifest parser has somewhere
    /// to land the value.
    pub wall_clock: bool,
}

impl Capabilities {
    /// True iff `codeless:attachments/store` should be linked for
    /// this plugin. The host's gate is the manifest list, not a
    /// per-call check, so a plugin that holds neither `read` nor
    /// `write` cannot even instantiate against a component that
    /// imports the interface.
    pub fn link_attachments(&self) -> bool {
        self.attachments_read || self.attachments_write
    }

    /// True iff `codeless:fs/probe` should be linked. Same gate
    /// shape as [`Self::link_attachments`].
    pub fn link_fs_probe(&self) -> bool {
        !self.fs_allow.is_empty()
    }
}
