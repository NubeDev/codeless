//! End-to-end plugin substrate smoke: write a `notes`-shaped plugin
//! directory to disk, register a tool through the
//! statically-linked-fn shape, and prove `PluginRegistry::load_plugin`
//! threads manifest + tools + personas + migrations correctly.

use std::sync::Arc;

use async_trait::async_trait;
use codeless_tools::plugin::{
    ModelFamilyConfig, PluginRegistry, PluginToolSink, RegistrationTable,
};
use codeless_tools::{Tool, ToolCtx, ToolError};
use serde_json::{json, Value};
use tempfile::TempDir;

struct NotesAppend {
    schema: Value,
}

impl NotesAppend {
    fn new() -> Self {
        Self {
            schema: json!({
                "type": "object",
                "properties": {
                    "body": { "type": "string" },
                },
                "required": ["body"],
            }),
        }
    }
}

#[async_trait]
impl Tool for NotesAppend {
    fn name(&self) -> &str {
        "notes.append"
    }
    fn schema(&self) -> &Value {
        &self.schema
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<Value, ToolError> {
        Ok(json!({"appended": args.get("body").cloned().unwrap_or(Value::Null)}))
    }
}

fn notes_register(sink: &mut PluginToolSink) -> Result<(), String> {
    sink.register(Arc::new(NotesAppend::new()));
    Ok(())
}

#[test]
fn notes_plugin_loads_through_registry() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("notes");
    std::fs::create_dir_all(root.join("prompts")).unwrap();
    std::fs::create_dir_all(root.join("migrations")).unwrap();
    std::fs::create_dir_all(root.join("domains")).unwrap();
    std::fs::write(
        root.join("plugin.toml"),
        r#"
[plugin]
id = "notes"
version = "0.1.0"
crate = "codeless-plugin-notes"

[[personas]]
id = "notes"
prompt_file = "prompts/system.md"
allowed_tools = ["notes.*", "attachments.read"]
default_model_family = "smart"
default_attachments_policy = "inline-thread-scoped"
name = "Notes"
description = "Appends free-form notes."
icon = "spark"

[migrations]
dir = "migrations"

[data]
dir = "domains"
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("prompts/system.md"),
        "You are the notes assistant.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("migrations/0001_init.sql"),
        "-- bootstrap\n\
         CREATE TABLE IF NOT EXISTS notes_entries (\n\
             id TEXT PRIMARY KEY,\n\
             thread_id TEXT NOT NULL,\n\
             body TEXT NOT NULL,\n\
             created_at INTEGER NOT NULL\n\
         );\n\
         CREATE INDEX notes_entries_thread_idx ON notes_entries(thread_id);\n",
    )
    .unwrap();

    let mut table = RegistrationTable::new();
    table.insert("notes", notes_register);
    let mut registry = PluginRegistry::new();
    let loaded = registry.load_plugin(&root, &table).unwrap().clone();

    assert_eq!(loaded.tool_ids, vec!["notes.append"]);
    assert_eq!(loaded.personas[0].name, "Notes");
    assert_eq!(loaded.personas[0].id, "notes:notes");
    assert_eq!(loaded.migrations.len(), 1);

    // Tool is dispatchable through the shared ToolRegistry.
    let tool = registry
        .tool_registry()
        .get("notes.append")
        .expect("registered");
    let cfg = ModelFamilyConfig::builtin();
    assert!(cfg
        .resolve(&loaded.personas[0].default_model_family)
        .is_some());

    // sanity: schema visible
    assert_eq!(tool.schema()["type"], "object");
}

#[test]
fn malformed_migration_fails_load() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("badplugin");
    std::fs::create_dir_all(root.join("prompts")).unwrap();
    std::fs::create_dir_all(root.join("migrations")).unwrap();
    std::fs::write(
        root.join("plugin.toml"),
        r#"
[plugin]
id = "badplugin"
version = "0.1.0"
crate = "codeless-plugin-badplugin"

[[personas]]
id = "p"
prompt_file = "prompts/system.md"
allowed_tools = []
default_model_family = "smart"
default_attachments_policy = "inline-thread-scoped"
"#,
    )
    .unwrap();
    std::fs::write(root.join("prompts/system.md"), "hi").unwrap();
    std::fs::write(root.join("migrations/0001.sql"), "DROP TABLE personas;").unwrap();

    let mut table = RegistrationTable::new();
    table.insert("badplugin", |_| Ok(()));
    let mut registry = PluginRegistry::new();
    let err = registry.load_plugin(&root, &table).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("personas") && msg.contains("namespace"),
        "got: {msg}"
    );
}
