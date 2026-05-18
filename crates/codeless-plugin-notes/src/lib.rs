//! Plugin #0: `notes`.
//!
//! Substrate-doc PS-NOTES (DOCS/PLUGIN-SUBSTRATE.md "Plugin #0: notes"):
//! the smallest possible plugin that touches every substrate primitive,
//! so a breakage in items 1-8 fails CI here long before it reaches a
//! real workflow plugin (the estimator). Lives in-tree alongside the
//! substrate for the same reason.
//!
//! Stage 5 of plugin-substrate-runtimes ports the plugin onto
//! `codeless-plugin-sdk`: the canonical authoring surface is the
//! `ToolBehavior` impl on [`NotesAppend`] plus the
//! [`codeless_plugin_sdk::register!`] invocation. The same source
//! compiles into two flavours:
//!
//! - **builtin** (`cargo build`, `feature = "builtin"`, default):
//!   `NotesAppend` also implements `codeless_tools::Tool` so the
//!   existing [`register`] entry point keeps slotting into the host
//!   binary's `RegistrationTable`. The two impls share the args type,
//!   the schemas, and the call body so a divergence between flavours
//!   is impossible by construction.
//! - **wasm** (`cargo build --target wasm32-wasip2 --no-default-
//!   features --features wasm`): a `#[cfg(target_arch = "wasm32")]`
//!   module below invokes `wit_bindgen::generate!` against
//!   `crates/codeless-tool-wit/wit/tool.wit`, supplies the WIT `Guest`
//!   impl (`describe` builds a manifest from `Manifest::for_behavior::
//!   <NotesAppend>()`, `call` dispatches into the same async
//!   `ToolBehavior::call`), and `export!`s the component. The host
//!   loads the resulting `.wasm` artefact through
//!   `codeless_plugin_host_wasm::WasmPlugin::load`.
//!
//! Shape (unchanged from PS-NOTES):
//!
//! - One tool, `notes.append`, that takes a markdown `body`, persists
//!   it under the plugin's `notes_entries` table, and returns an
//!   attachment reference to a rendered markdown blob. The attachment
//!   creation itself is the runtime's responsibility (the tool layer
//!   stays sqlx-free, see `codeless-tools::ToolCtx`); the tool returns
//!   the attachment-ref the agent loop reconciles via
//!   `codeless_tools::attachment::reconcile_attachment_refs`. Until a
//!   notes-table writer is wired through the host (item 6's static
//!   linkage table + a future `notes.store` ctx extension), the tool's
//!   `call` returns a structured "wire-up pending" failure -- the
//!   substrate test coverage in `tests/plugin_smoke.rs` exercises
//!   manifest + registration + schema, which is what stage PS-NOTES
//!   pins. PS-ACCEPT lands the end-to-end Assistant drive.
//! - One persona, `notes`, granting `notes.*` plus `attachments.read`
//!   (substrate doc: prefer scoped attachment access over raw
//!   `fs.read`). System prompt lives in the plugin directory's
//!   `prompts/system.md`.
//! - One migration, `0001_init.sql`, creating `notes_entries` under
//!   the `<plugin_id>_*` namespace rule (OQ-PS-4); a stray
//!   `personas`-targeting statement here is exactly the failure
//!   `codeless_tools::plugin::check_migration_sql` catches at load time.
//!
//! The plugin directory itself lives at the repo root under
//! `plugins/notes/`; only [`register`] here is statically linked into
//! the host binary's `RegistrationTable`.

use codeless_plugin_sdk::{Tier, ToolBehavior, ToolError, ToolMeta};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Plugin id as declared in `plugins/notes/plugin.toml`. Exposed so the
/// host binary's wiring code can register without re-typing the
/// literal -- a mismatch between the registration-table key and the
/// manifest's `plugin.id` is exactly the load-time
/// `PluginLoadError::UnknownPlugin` we'd otherwise hit.
pub const PLUGIN_ID: &str = "notes";

/// Args for `notes.append`. Standalone struct (not a tuple of bare
/// `serde_json::Value`) so `schemars` generates a typed input schema
/// the runner advertises to the LLM. `JsonSchema` is the SDK trait
/// bound; `Deserialize` lets the per-flavour adapter parse from the
/// WIT `args-json` string straight into this type.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NotesAppendArgs {
    /// Markdown body of the note to append.
    pub body: String,
}

/// Output for `notes.append`. The `attachment` field carries the
/// substrate-doc PS7 marker (`codeless://attachment` `$ref`) so the
/// Assistant agent loop renders a download card without per-plugin UI
/// code. Encoded inline via a `schemars` schema-with hook because the
/// SDK does not yet ship a typed `AttachmentRef` (porting `codeless_
/// types::AttachmentRef` into a mobile-safe SDK is a future stage).
#[derive(Debug, Serialize, JsonSchema)]
pub struct NotesAppendOutput {
    // schemars wraps a `schema_with` field in `allOf` whenever a doc
    // comment is present, which breaks the PS7 walker contract (it
    // expects `/properties/attachment/$ref` directly). Keep the field
    // free of a doc comment; the rationale lives on the helper below.
    #[schemars(schema_with = "attachment_ref_schema")]
    pub attachment: serde_json::Value,
    /// Optional human-readable summary the planner can quote.
    pub summary: Option<String>,
}

/// Schemars hook producing the codeless-tools attachment-ref schema.
/// Kept in this crate (rather than re-exported from `codeless-tools`)
/// because the wasm flavour must not pull in the host-only crate; the
/// shape is pinned by the substrate-doc PS7 marker + the
/// `output_schema_declares_attachment_marker` test below.
fn attachment_ref_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    use schemars::schema::{Schema, SchemaObject};
    Schema::Object(SchemaObject {
        reference: Some("codeless://attachment".into()),
        ..Default::default()
    })
}

/// `notes.append` -- append a markdown note and return a rendered
/// markdown attachment.
///
/// One struct, two trait impls below: [`ToolBehavior`] (the SDK's
/// authoring surface, present on every build) and -- under the
/// `builtin` feature -- `codeless_tools::Tool` (the host's dispatch
/// surface). Both delegate into [`Self::call_inner`] so the body lives
/// in exactly one place.
#[derive(Debug, Default)]
pub struct NotesAppend;

impl NotesAppend {
    pub fn new() -> Self {
        Self
    }

    /// Shared body for both flavours. Returns the typed
    /// [`NotesAppendOutput`] so the SDK manifest carries the structured
    /// schema; the builtin shim serialises to `serde_json::Value` at
    /// the trait boundary.
    fn call_inner(args: NotesAppendArgs) -> Result<NotesAppendOutput, ToolError> {
        // Argument shape is validated here even though the schema is
        // advertised to the runner: an MCP client that bypasses
        // validation would otherwise reach the runtime with a malformed
        // payload, and the contract from the substrate doc is that the
        // *tool* enforces its own preconditions.
        if args.body.trim().is_empty() {
            return Err(ToolError::invalid_args("notes.append: `body` is empty"));
        }

        // The plugin layer is sqlx-free (R1, plus the documented
        // boundary in `crates/codeless-tools/src/plugin/registry.rs`),
        // so this tool cannot itself write the `notes_entries` row or
        // mint the `assistant_attachments` row. The host wires a
        // notes-specific ctx extension before stage PS-ACCEPT drives
        // the full Assistant -> persona -> tool -> attachment path;
        // until then a structured `Failed` is the honest signal -- a
        // silent stub would hide the wiring gap.
        Err(ToolError::failed(
            "notes.append: runtime wiring lands in PS-ACCEPT; \
             the tool, manifest, and persona are registered through \
             the plugin substrate (PS-NOTES) but the notes-table \
             writer + attachment minting are not yet plumbed through \
             ToolCtx",
        ))
    }
}

impl ToolMeta for NotesAppend {
    const ID: &'static str = "notes.append";
    const TIER: Tier = Tier::Write;
    const DESCRIPTION: &'static str =
        "Append a markdown note and return a rendered markdown attachment.";
}

#[async_trait::async_trait]
impl ToolBehavior for NotesAppend {
    type Args = NotesAppendArgs;
    type Output = NotesAppendOutput;

    async fn call(
        &self,
        _ctx: &codeless_plugin_sdk::ToolCtx,
        args: Self::Args,
    ) -> Result<Self::Output, ToolError> {
        Self::call_inner(args)
    }
}

// SDK packaging hook. Today the macro is the stub described in
// `codeless-plugin-sdk::register`: type-checks the `ToolBehavior` impl
// at compile time, emits no runtime glue. Per-flavour glue lands here
// once the macro grows expansions.
codeless_plugin_sdk::register!(NotesAppend);

// --------------------------------------------------------------------
// Builtin flavour: the host's `RegistrationTable` entry. Compiles only
// when `codeless-tools` is in the dep tree (i.e. `feature = "builtin"`),
// because the `Tool` trait it implements lives there.
// --------------------------------------------------------------------

#[cfg(feature = "builtin")]
mod builtin {
    use super::*;
    use async_trait::async_trait;
    use codeless_tools::plugin::PluginToolSink;
    use codeless_tools::{Tool, ToolCtx as HostToolCtx, ToolError as HostToolError};
    use serde_json::Value;
    use std::sync::Arc;

    /// Host-side bridge from the SDK's [`ToolBehavior`] to
    /// `codeless_tools::Tool`. The runtime sees a `Tool` (JSON in /
    /// JSON out); the bridge translates to and from the typed
    /// `ToolBehavior::Args` / `Output` on every call.
    ///
    /// Lives in this crate (not in `codeless-tools`) so the dependency
    /// edge stays one-way (`codeless-tools` does not depend on
    /// `codeless-plugin-sdk`). The bridge is generic so a second
    /// builtin plugin can reuse the same shape without duplicating
    /// the serde/schema dance.
    pub struct BuiltinBridge<T: ToolBehavior> {
        behavior: T,
        input_schema: Value,
    }

    impl<T: ToolBehavior + Default> BuiltinBridge<T> {
        pub fn new() -> Self {
            let manifest = codeless_plugin_sdk::Manifest::for_behavior::<T>();
            Self {
                behavior: T::default(),
                input_schema: manifest.input_schema,
            }
        }
    }

    impl<T: ToolBehavior + Default> Default for BuiltinBridge<T> {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl<T> Tool for BuiltinBridge<T>
    where
        T: ToolBehavior + Send + Sync + 'static,
        T::Args: Send,
        T::Output: Send,
    {
        fn name(&self) -> &str {
            <T as ToolMeta>::ID
        }

        fn schema(&self) -> &Value {
            &self.input_schema
        }

        fn output_schema(&self) -> Value {
            codeless_plugin_sdk::Manifest::for_behavior::<T>().output_schema
        }

        async fn call(&self, _ctx: &HostToolCtx, args: Value) -> Result<Value, HostToolError> {
            // Pre-`call` validation: a plugin author may have advertised
            // a typed schema, but the host accepts `Value` at the
            // trait boundary because that is what MCP clients deliver.
            // Map the parse error to `InvalidArgs` rather than `Failed`
            // so the agent loop reports a precondition violation
            // instead of a tool-internal fault.
            let parsed: T::Args = serde_json::from_value(args).map_err(|e| {
                HostToolError::invalid_args(format!("{}: {e}", <T as ToolMeta>::ID))
            })?;
            let sdk_ctx = codeless_plugin_sdk::ToolCtx::__from_host_seal();
            let out = self
                .behavior
                .call(&sdk_ctx, parsed)
                .await
                .map_err(map_tool_error)?;
            serde_json::to_value(out).map_err(|e| {
                HostToolError::failed(format!(
                    "{}: output serialisation failed: {e}",
                    <T as ToolMeta>::ID,
                ))
            })
        }
    }

    /// Map the SDK's `ToolError` vocabulary onto the host's. The
    /// variants line up 1-1 (the SDK's enum was lifted to match);
    /// keeping a typed bridge here means a future SDK addition
    /// breaks the build instead of getting silently flattened.
    fn map_tool_error(e: ToolError) -> HostToolError {
        match e {
            ToolError::InvalidArgs(m) => HostToolError::invalid_args(m),
            ToolError::Failed(m) => HostToolError::failed(m),
            // The host's `ToolError` does not yet carry a retryable
            // variant; surface as `Failed` with the message prefix so
            // the dispatcher can still report a meaningful reason.
            ToolError::Retryable(m) => HostToolError::failed(format!("retryable: {m}")),
            // Same story for cancelled -- the host vocabulary lacks
            // the variant in this stage.
            ToolError::Cancelled => HostToolError::failed("cancelled"),
        }
    }

    /// Statically-linked entry point the host invokes via the
    /// `RegistrationTable`. Signature matches
    /// `codeless_tools::plugin::RegisterFn` (a `fn` pointer, not a
    /// closure -- the table needs `Copy`).
    pub fn register(sink: &mut PluginToolSink) -> Result<(), String> {
        sink.register(Arc::new(BuiltinBridge::<NotesAppend>::new()));
        Ok(())
    }
}

#[cfg(feature = "builtin")]
pub use builtin::register;

// --------------------------------------------------------------------
// WASM flavour: WIT guest exports. Compiles only when the build
// target is wasm32 *and* the `wasm` feature is on, so a host build
// with the `wasm` feature accidentally enabled does not produce
// dangling `unsafe extern "C"` symbols.
// --------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
#[allow(unsafe_code)]
mod wasm_guest {
    use super::*;

    // Local copy of the wit-bindgen guest output. Separate from the
    // committed bindings in `codeless-tool-wit/src/bindings.rs`: those
    // are the review artefact for ABI changes (OQ-WASM-2); this is the
    // export site -- the WIT `export!` macro the generator emits is
    // `pub(crate)` and can only be invoked from the same crate that
    // ran `generate!`. Regenerating against the same `tool.wit` keeps
    // the two copies in lockstep by construction.
    wit_bindgen::generate!({
        path: "../codeless-tool-wit/wit",
        world: "plugin",
    });

    use exports::codeless::tool::tool::{
        Guest, Tier as WitTier, ToolCall as WitToolCall, ToolError as WitToolError,
        ToolManifest as WitToolManifest, ToolResult as WitToolResult,
    };

    /// The component the host instantiates. Stateless across calls
    /// (PLUGIN-WASM.md "Instance lifecycle"): per-call instantiation
    /// resets every field, so there is nothing to hold between
    /// `Guest::call` invocations.
    struct NotesComponent;

    impl Guest for NotesComponent {
        fn describe() -> Vec<WitToolManifest> {
            // `Manifest::for_behavior` re-runs schemars on every
            // call; the host calls `describe` at load time only, so
            // the cost is paid once.
            let m = codeless_plugin_sdk::Manifest::for_behavior::<NotesAppend>();
            vec![WitToolManifest {
                id: m.id.to_string(),
                description: m.description.to_string(),
                input_schema: m.input_schema.to_string(),
                output_schema: m.output_schema.to_string(),
                tier: tier_to_wit(m.tier),
            }]
        }

        fn call(req: WitToolCall) -> WitToolResult {
            // WIT signature is sync; the SDK's `ToolBehavior::call`
            // is async because the builtin flavour runs on tokio.
            // `pollster::block_on` runs the future to completion
            // without pulling tokio (which does not build for
            // wasm32-wasip2 with its default features).
            let args: NotesAppendArgs = match serde_json::from_str(&req.args_json) {
                Ok(a) => a,
                Err(e) => {
                    return WitToolResult::Err(WitToolError {
                        code: "invalid-args".into(),
                        message: format!("notes.append: {e}"),
                        retryable: false,
                    });
                }
            };
            let ctx = codeless_plugin_sdk::ToolCtx::__from_host_seal();
            let outcome = pollster::block_on(<NotesAppend as ToolBehavior>::call(
                &NotesAppend,
                &ctx,
                args,
            ));
            match outcome {
                Ok(out) => match serde_json::to_string(&out) {
                    Ok(s) => WitToolResult::Ok(s),
                    Err(e) => WitToolResult::Err(WitToolError {
                        code: "failed".into(),
                        message: format!("notes.append: output serialisation failed: {e}"),
                        retryable: false,
                    }),
                },
                Err(e) => WitToolResult::Err(tool_error_to_wit(e)),
            }
        }
    }

    fn tier_to_wit(t: Tier) -> WitTier {
        match t {
            Tier::Read => WitTier::Read,
            Tier::Write => WitTier::Write,
            Tier::Destructive => WitTier::Destructive,
        }
    }

    fn tool_error_to_wit(e: ToolError) -> WitToolError {
        let code = e.code();
        let retryable = e.retryable_flag();
        let message = match e {
            ToolError::InvalidArgs(m) | ToolError::Failed(m) | ToolError::Retryable(m) => m,
            ToolError::Cancelled => "cancelled".into(),
        };
        WitToolError {
            code: code.into(),
            message,
            retryable,
        }
    }

    export!(NotesComponent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_constants_match_persona_grant_pattern() {
        // Persona grants `notes.*`; the matcher in
        // codeless_types::allowed_tools is dotted-prefix only.
        assert_eq!(<NotesAppend as ToolMeta>::ID, "notes.append");
        let (head, _) = <NotesAppend as ToolMeta>::ID.split_once('.').unwrap();
        assert_eq!(head, PLUGIN_ID);
        assert_eq!(<NotesAppend as ToolMeta>::TIER, Tier::Write);
    }

    #[test]
    fn output_schema_declares_attachment_marker() {
        // PS7 contract: the renderer walks the output schema for
        // `$ref: codeless://attachment` markers. The schema is derived
        // through `schemars` from the typed `NotesAppendOutput`, so a
        // drift between the typed struct and the runtime walker also
        // trips here.
        let m = codeless_plugin_sdk::Manifest::for_behavior::<NotesAppend>();
        let r = m
            .output_schema
            .pointer("/properties/attachment/$ref")
            .and_then(|v| v.as_str())
            .expect("attachment field carries an $ref");
        assert_eq!(r, "codeless://attachment");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn behavior_rejects_empty_body() {
        let ctx = codeless_plugin_sdk::ToolCtx::__from_host_seal();
        let err = <NotesAppend as ToolBehavior>::call(
            &NotesAppend,
            &ctx,
            NotesAppendArgs { body: "   ".into() },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn behavior_pre_wire_returns_failed() {
        let ctx = codeless_plugin_sdk::ToolCtx::__from_host_seal();
        let err = <NotesAppend as ToolBehavior>::call(
            &NotesAppend,
            &ctx,
            NotesAppendArgs {
                body: "remember".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)));
    }

    // `register` itself is covered by the on-disk integration test in
    // `tests/plugin_smoke.rs`, which drives the host's
    // `PluginRegistry::load_plugin` over the real `plugins/notes/`
    // directory. Going through that path is what proves the manifest
    // and the registration entry agree on the plugin id.
}
