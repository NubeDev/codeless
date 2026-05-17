//! Plugin-#0 end-to-end load: points `PluginRegistry::load_plugin` at
//! the on-disk `plugins/notes/` directory shipped with the workspace
//! and proves the manifest + migration + registration entry all line
//! up. Substrate-doc PS-NOTES: this is the test that fails first when
//! any of substrate items 1, 5, 6, or 7 regress.

use std::path::PathBuf;

use codeless_tools::plugin::{PluginRegistry, RegistrationTable};

fn plugin_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` -> .../crates/codeless-plugin-notes
    // walk up two levels to the workspace root, then `plugins/notes`.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("plugins")
        .join("notes")
}

#[test]
fn on_disk_notes_plugin_loads_through_registry() {
    let dir = plugin_dir();
    assert!(
        dir.join("plugin.toml").exists(),
        "missing plugins/notes/plugin.toml at {}",
        dir.display(),
    );

    let mut table = RegistrationTable::new();
    table.insert(
        codeless_plugin_notes::PLUGIN_ID,
        codeless_plugin_notes::register,
    );

    let mut registry = PluginRegistry::new();
    let loaded = registry.load_plugin(&dir, &table).expect("load").clone();

    assert_eq!(loaded.manifest.plugin.id, "notes");
    assert_eq!(loaded.manifest.plugin.crate_name, "codeless-plugin-notes");
    assert_eq!(loaded.tool_ids, vec!["notes.append"]);
    assert_eq!(loaded.personas.len(), 1);

    // The loader namespaces bare persona ids with `<plugin_id>:` --
    // substrate-doc item 5 wants personas globally addressable, so
    // pin the resulting id here so a refactor in the loader cannot
    // silently change the public address.
    assert_eq!(loaded.personas[0].id, "notes:notes");
    assert_eq!(loaded.personas[0].default_model_family, "fast");
    assert!(loaded.personas[0]
        .allowed_tools
        .iter()
        .any(|t| t == "notes.*"));
    assert!(loaded.personas[0]
        .allowed_tools
        .iter()
        .any(|t| t == "attachments.read"));
    assert!(loaded.personas[0].system_prompt.contains("notes.append"));

    // One migration shipped (0001_init.sql) with the notes_entries
    // table under the `<plugin_id>_` namespace; the static prefix
    // check already ran inside load_plugin, so reaching here is the
    // proof of OQ-PS-4 compliance.
    assert_eq!(loaded.migrations.len(), 1);
    assert!(loaded.migrations[0]
        .path
        .to_string_lossy()
        .ends_with("0001_init.sql"));

    // Tool is dispatchable through the shared ToolRegistry the host
    // hands to codeless-mcp.
    let tool = registry
        .tool_registry()
        .get("notes.append")
        .expect("notes.append registered");

    // PS7 contract: the tool's output schema carries the attachment
    // marker so the Assistant renderer surfaces a download card.
    let output = tool.output_schema();
    assert_eq!(
        output
            .pointer("/properties/attachment/$ref")
            .and_then(|v| v.as_str()),
        Some("codeless://attachment"),
    );
}
