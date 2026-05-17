//! Plugin #0: `notes`.
//!
//! Substrate-doc PS-NOTES (DOCS/PLUGIN-SUBSTRATE.md "Plugin #0: notes"):
//! the smallest possible plugin that touches every substrate primitive,
//! so a breakage in items 1-8 fails CI here long before it reaches a
//! real workflow plugin (the estimator). Lives in-tree alongside the
//! substrate for the same reason.
//!
//! Shape:
//!
//! - One tool, `notes.append`, that takes a markdown `body`, persists
//!   it under the plugin's `notes_entries` table, and returns an
//!   attachment reference to a rendered markdown blob. The attachment
//!   creation itself is the runtime's responsibility (the tool layer
//!   stays sqlx-free, see `codeless-tools::ToolCtx`); the tool returns
//!   the [`AttachmentRef`] the agent loop reconciles via
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
//! `plugins/notes/`; only `register` here is statically linked.
//! `codeless-cli` (the host binary) inserts the entry into its
//! `RegistrationTable` via [`PLUGIN_ID`] + [`register`].

use std::sync::Arc;

use async_trait::async_trait;
use codeless_tools::attachment::attachment_ref_schema;
use codeless_tools::plugin::PluginToolSink;
use codeless_tools::{Tool, ToolCtx, ToolError};
use serde_json::{json, Value};

/// Plugin id as declared in `plugins/notes/plugin.toml`. Exposed so the
/// host binary's wiring code can register without re-typing the
/// literal -- a mismatch between the registration-table key and the
/// manifest's `plugin.id` is exactly the load-time
/// `PluginLoadError::UnknownPlugin` we'd otherwise hit.
pub const PLUGIN_ID: &str = "notes";

/// Statically-linked entry point the host invokes via the
/// `RegistrationTable`. Signature matches
/// `codeless_tools::plugin::RegisterFn` (a `fn` pointer, not a closure
/// -- the table needs `Copy`).
pub fn register(sink: &mut PluginToolSink) -> Result<(), String> {
    sink.register(Arc::new(NotesAppend::new()));
    Ok(())
}

/// `notes.append` — append a markdown note and return a rendered
/// markdown attachment.
///
/// Args: `{ body: string }` -- the note text (markdown).
///
/// Output schema declares the attachment marker
/// (`codeless_tools::attachment::ATTACHMENT_SCHEMA_REF`) on the
/// `attachment` field so the Assistant agent loop (PS8) renders the
/// download card without per-plugin UI code (PS7 acceptance).
pub struct NotesAppend {
    input_schema: Value,
    output_schema: Value,
}

impl NotesAppend {
    pub fn new() -> Self {
        Self {
            input_schema: json!({
                "type": "object",
                "properties": {
                    "body": {
                        "type": "string",
                        "description": "Markdown body of the note to append."
                    }
                },
                "required": ["body"]
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "attachment": attachment_ref_schema(),
                    "summary": { "type": "string" }
                },
                "required": ["attachment"]
            }),
        }
    }
}

impl Default for NotesAppend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for NotesAppend {
    fn name(&self) -> &str {
        "notes.append"
    }

    fn schema(&self) -> &Value {
        &self.input_schema
    }

    fn output_schema(&self) -> Value {
        self.output_schema.clone()
    }

    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        // Argument shape is validated here even though the schema is
        // advertised to the runner: an MCP client that bypasses
        // validation would otherwise reach the runtime with a malformed
        // payload, and the contract from the substrate doc is that the
        // *tool* enforces its own preconditions.
        let body = args
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_args("notes.append: missing `body`"))?;
        if body.trim().is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_tools::attachment::ATTACHMENT_SCHEMA_REF;

    #[test]
    fn name_matches_persona_grant_pattern() {
        // Persona grants `notes.*`; the matcher in
        // codeless_types::allowed_tools is dotted-prefix only.
        let tool = NotesAppend::new();
        assert_eq!(tool.name(), "notes.append");
        let (head, _) = tool.name().split_once('.').unwrap();
        assert_eq!(head, PLUGIN_ID);
    }

    #[test]
    fn output_schema_declares_attachment_marker() {
        // PS7 contract: the renderer walks the output schema for
        // `$ref: codeless://attachment` markers. If the marker drifts,
        // the agent loop silently stops rendering the card -- assert
        // the literal here so a substrate rename trips this test.
        let tool = NotesAppend::new();
        let schema = tool.output_schema();
        let r = schema
            .pointer("/properties/attachment/$ref")
            .and_then(Value::as_str)
            .expect("attachment field carries an $ref");
        assert_eq!(r, ATTACHMENT_SCHEMA_REF);
    }

    // `register` itself is covered by the on-disk integration test in
    // `tests/plugin_smoke.rs`, which drives the host's
    // `PluginRegistry::load_plugin` over the real `plugins/notes/`
    // directory. Going through that path is what proves the manifest
    // and the registration entry agree on the plugin id.
}
