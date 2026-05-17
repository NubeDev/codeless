//! Plugin registry + `load_plugin(path)` orchestrator.
//!
//! `load_plugin` is the substrate entry point a host (the codeless
//! binary, the test harness) calls once per plugin directory at
//! startup. It threads three sub-results into one transaction-shaped
//! result:
//!
//! 1. Parse `plugin.toml` (manifest module).
//! 2. Look up the plugin's statically-linked registration function in
//!    the `RegistrationTable` the host built at compile time and call
//!    it with `&mut ToolRegistry` so the plugin's `register_tool` calls
//!    fire. This is the MVP — see DOCS/PLUGIN-SUBSTRATE.md OQ-PS-2 for
//!    the dynamic-loading question that lives at Phase 7.
//! 3. Read + validate the plugin's migration SQL (migrations module).
//!    The actual `sqlx` application is the runtime's responsibility,
//!    not codeless-tools'; this module hands the runtime a vetted
//!    `Vec<PluginMigration>` and a vetted `Vec<PluginPersona>` and
//!    stops there. Keeping the SQL apply out of `codeless-tools` is
//!    what lets the plugin layer compile without a sqlx dependency,
//!    which matters for the future split between sandbox-host and
//!    plugin-host binaries.
//!
//! The `LoadedPlugin` returned is the snapshot `codeless plugin list`
//! / `codeless plugin info` reads. The registry indexes loaded plugins
//! by `manifest.plugin.id` and rejects duplicate ids at load time.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::registry::ToolRegistry;
use crate::tool::Tool;

use super::manifest::{ManifestError, PluginManifest, PluginPersona};
use super::migrations::{load_migrations_dir, MigrationCheckError, PluginMigration};

/// Function pointer the host binary supplies for each statically-linked
/// plugin. The closure receives a `PluginToolSink` it pushes
/// `Arc<dyn Tool>` instances into via `sink.register(...)`. The sink
/// stages the tools in a Vec; `load_plugin` then atomically merges
/// them into the shared `ToolRegistry` after checking for collisions
/// with already-registered ids. Staging means a plugin that collides
/// on one of its tool ids does not leave a partial registration
/// behind.
///
/// Plain `fn` (not `Fn`) so the table is `Copy` and can live in a
/// `&'static` table the binary builds at compile time. Plugins that
/// need richer setup can stash a `lazy_static`/`OnceCell` inside the
/// function body.
pub type RegisterFn = fn(&mut PluginToolSink) -> Result<(), String>;

/// Collector handed to a plugin's `RegisterFn`. The plugin calls
/// `sink.register(Arc::new(MyTool::new()))` once per tool; the sink
/// records the tool and the `load_plugin` orchestrator merges them
/// into the host's `ToolRegistry` after the registration call returns
/// successfully.
pub struct PluginToolSink {
    tools: Vec<Arc<dyn Tool>>,
}

impl PluginToolSink {
    fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Stage one tool for registration with the host. The order tools
    /// land in is preserved so a plugin can reason about registration
    /// order (though MCP itself does not care).
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    /// View into the staged tools — used by tests and by the
    /// orchestrator after the registration call returns.
    pub fn staged(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }
}

/// Host-built map from `plugin.toml` id to the registration function
/// the host's binary already links against. `load_plugin` looks the id
/// up here; a missing id means the operator dropped a plugin
/// directory next to the binary without rebuilding the binary, which
/// is an MVP error rather than a runtime feature (see OQ-PS-2).
#[derive(Default)]
pub struct RegistrationTable {
    entries: HashMap<String, RegisterFn>,
}

impl RegistrationTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a `(plugin_id, register_fn)` pair. Returns the previous
    /// entry if any -- the host binary's wiring code is the only
    /// caller and a duplicate registration there is a build-time bug.
    pub fn insert(&mut self, id: impl Into<String>, f: RegisterFn) -> Option<RegisterFn> {
        self.entries.insert(id.into(), f)
    }

    pub fn get(&self, id: &str) -> Option<RegisterFn> {
        self.entries.get(id).copied()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

/// One plugin's view into the registry after `load_plugin` completes.
/// Hand back enough for `codeless plugin info` to render the manifest
/// and the *registered* tool ids -- the registry is authoritative for
/// tools (substrate doc item 6).
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    /// Tools the plugin's registration function added to the shared
    /// `ToolRegistry`. Stored as ids (the registry owns the
    /// `Arc<dyn Tool>` instances).
    pub tool_ids: Vec<String>,
    /// Personas declared in the manifest, with `prompt_file` resolved
    /// into the persona's actual system-prompt text.
    pub personas: Vec<LoadedPersona>,
    /// Migrations to apply, in lex order, already prefix-checked.
    pub migrations: Vec<PluginMigration>,
}

/// Persona row materialised from `plugin.toml`. Field names match the
/// `personas` SQL columns one-for-one; the runtime upserts via
/// `SqliteStore::upsert_persona`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedPersona {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub default_model_family: String,
    pub default_attachments_policy: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginLoadError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Migration(#[from] MigrationCheckError),
    #[error(
        "plugin `{0}` has no statically-linked registration entry in the host binary; \
         rebuild codeless with the plugin's crate compiled in (DOCS/PLUGIN-SUBSTRATE.md \
         OQ-PS-2)"
    )]
    UnknownPlugin(String),
    #[error("plugin `{0}` registration failed: {1}")]
    RegistrationFailed(String, String),
    #[error("plugin `{0}` is already loaded")]
    DuplicatePlugin(String),
    #[error("read prompt_file {path}: {source}")]
    ReadPrompt {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "plugin `{plugin}` tried to register tool `{tool}` which is already in the \
         registry; tool ids are global across plugins"
    )]
    DuplicateTool { plugin: String, tool: String },
}

/// The substrate registry. Holds one `LoadedPlugin` per id and the
/// shared `ToolRegistry` every plugin's registration entry pushes into.
pub struct PluginRegistry {
    plugins: BTreeMap<String, LoadedPlugin>,
    tool_registry: ToolRegistry,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self {
            plugins: BTreeMap::new(),
            tool_registry: ToolRegistry::new(),
        }
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    pub fn tool_registry_mut(&mut self) -> &mut ToolRegistry {
        &mut self.tool_registry
    }

    pub fn plugins(&self) -> impl Iterator<Item = &LoadedPlugin> {
        self.plugins.values()
    }

    pub fn get(&self, id: &str) -> Option<&LoadedPlugin> {
        self.plugins.get(id)
    }

    /// Convenience wrapper: take the shared `ToolRegistry` out by value.
    /// Called after every plugin has loaded so the host can hand a
    /// frozen `Arc<ToolRegistry>` to `codeless-mcp`.
    pub fn into_parts(self) -> (BTreeMap<String, LoadedPlugin>, ToolRegistry) {
        (self.plugins, self.tool_registry)
    }

    /// Item 6's central entry point. Parses `plugin.toml` under `dir`,
    /// runs the static migration prefix check, materialises persona
    /// rows from `prompt_file`, and invokes the registration function
    /// looked up in `table`. The tool registry is mutated in place;
    /// the returned `LoadedPlugin` is the snapshot the CLI reads.
    ///
    /// Failures roll back: if any step after registration fails the
    /// already-registered tools are not removed (registry mutations
    /// are not transactional today). The host stops applying further
    /// plugins on the first failure; a partial registry is acceptable
    /// because the binary will exit and the next start re-runs every
    /// `load_plugin` from scratch.
    pub fn load_plugin(
        &mut self,
        dir: &Path,
        table: &RegistrationTable,
    ) -> Result<&LoadedPlugin, PluginLoadError> {
        let manifest = PluginManifest::from_dir(dir)?;
        let plugin_id = manifest.plugin.id.clone();

        if self.plugins.contains_key(&plugin_id) {
            return Err(PluginLoadError::DuplicatePlugin(plugin_id));
        }

        let register = table
            .get(&plugin_id)
            .ok_or_else(|| PluginLoadError::UnknownPlugin(plugin_id.clone()))?;

        let mut sink = PluginToolSink::new();
        register(&mut sink)
            .map_err(|e| PluginLoadError::RegistrationFailed(plugin_id.clone(), e))?;

        // Collision check against the host registry and against the
        // sink itself (a plugin staging two tools with the same id is
        // the same shape of bug). Failing here leaves the host's
        // ToolRegistry untouched.
        let mut seen = std::collections::HashSet::new();
        for tool in sink.staged() {
            let id = tool.name().to_string();
            if self.tool_registry.get(&id).is_some() || !seen.insert(id.clone()) {
                return Err(PluginLoadError::DuplicateTool {
                    plugin: plugin_id,
                    tool: id,
                });
            }
        }

        let mut added = Vec::with_capacity(sink.tools.len());
        for tool in sink.tools {
            added.push(tool.name().to_string());
            self.tool_registry.register(tool);
        }
        added.sort();

        let migrations =
            load_migrations_dir(&plugin_id, &manifest.resolve(&manifest.migrations.dir))?;

        let mut personas = Vec::with_capacity(manifest.personas.len());
        for p in &manifest.personas {
            personas.push(load_persona(&plugin_id, &manifest, p)?);
        }

        let loaded = LoadedPlugin {
            manifest,
            tool_ids: added,
            personas,
            migrations,
        };
        let entry = self.plugins.entry(plugin_id.clone()).or_insert(loaded);
        Ok(entry)
    }

    /// Convenience: register a tool from outside a `load_plugin` call,
    /// used by codeless built-in tools that pre-date the plugin shape.
    pub fn register_builtin_tool(&mut self, tool: Arc<dyn Tool>) {
        self.tool_registry.register(tool);
    }
}

fn load_persona(
    plugin_id: &str,
    manifest: &PluginManifest,
    p: &PluginPersona,
) -> Result<LoadedPersona, PluginLoadError> {
    let prompt_path = manifest.resolve(&p.prompt_file);
    let system_prompt =
        std::fs::read_to_string(&prompt_path).map_err(|source| PluginLoadError::ReadPrompt {
            path: prompt_path.clone(),
            source,
        })?;
    // Prefix the persona id with `<plugin_id>:` if the manifest entry
    // is not already namespaced. The substrate doc's example uses bare
    // ids (`notes`); namespacing on insertion keeps the personas table
    // free of collisions with built-ins (`builtin:general`,
    // `builtin:coding`) and with other plugins' personas without
    // forcing every plugin author to repeat their id.
    let id = if p.id.contains(':') {
        p.id.clone()
    } else {
        format!("{plugin_id}:{}", p.id)
    };
    Ok(LoadedPersona {
        id,
        name: p.name.clone().unwrap_or_else(|| p.id.clone()),
        description: p
            .description
            .clone()
            .unwrap_or_else(|| format!("{plugin_id} plugin persona")),
        icon: p.icon.clone().unwrap_or_else(|| "spark".into()),
        system_prompt,
        allowed_tools: p.allowed_tools.clone(),
        default_model_family: p.default_model_family.clone(),
        default_attachments_policy: p.default_attachments_policy.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    use crate::ctx::ToolCtx;
    use crate::error::ToolError;
    use crate::tool::Tool;

    use super::*;

    struct DummyTool {
        name: String,
        schema: Value,
    }

    impl DummyTool {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                schema: json!({"type": "object"}),
            }
        }
    }

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn schema(&self) -> &Value {
            &self.schema
        }
        async fn call(&self, _ctx: &ToolCtx, _args: Value) -> Result<Value, ToolError> {
            Ok(json!({}))
        }
    }

    fn write_notes_plugin(tmp: &TempDir) -> PathBuf {
        let root = tmp.path().join("notes");
        std::fs::create_dir_all(root.join("prompts")).unwrap();
        std::fs::create_dir_all(root.join("migrations")).unwrap();
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
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("prompts/system.md"),
            "You are the notes-plugin persona.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("migrations/0001_init.sql"),
            "CREATE TABLE notes_entries (id TEXT PRIMARY KEY, body TEXT NOT NULL);\n",
        )
        .unwrap();
        root
    }

    fn notes_register(sink: &mut PluginToolSink) -> Result<(), String> {
        sink.register(Arc::new(DummyTool::new("notes.append")));
        Ok(())
    }

    #[test]
    fn load_plugin_end_to_end() {
        let tmp = TempDir::new().unwrap();
        let root = write_notes_plugin(&tmp);
        let mut table = RegistrationTable::new();
        table.insert("notes", notes_register);

        let mut registry = PluginRegistry::new();
        let loaded = registry.load_plugin(&root, &table).unwrap().clone();

        assert_eq!(loaded.manifest.plugin.id, "notes");
        assert_eq!(loaded.tool_ids, vec!["notes.append"]);
        assert_eq!(loaded.personas.len(), 1);
        assert_eq!(loaded.personas[0].id, "notes:notes");
        assert!(loaded.personas[0]
            .system_prompt
            .contains("You are the notes-plugin persona"));
        assert_eq!(loaded.migrations.len(), 1);
        assert!(registry.tool_registry().get("notes.append").is_some());
    }

    #[test]
    fn missing_registration_entry_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let root = write_notes_plugin(&tmp);
        let table = RegistrationTable::new(); // empty
        let mut registry = PluginRegistry::new();
        let err = registry.load_plugin(&root, &table).unwrap_err();
        assert!(matches!(err, PluginLoadError::UnknownPlugin(id) if id == "notes"));
    }

    #[test]
    fn rejects_migration_that_touches_codeless_table() {
        let tmp = TempDir::new().unwrap();
        let root = write_notes_plugin(&tmp);
        // Overwrite the migration with one that targets a codeless-
        // owned table; load must fail.
        std::fs::write(
            root.join("migrations/0001_init.sql"),
            "ALTER TABLE personas ADD COLUMN x TEXT;",
        )
        .unwrap();
        let mut table = RegistrationTable::new();
        table.insert("notes", notes_register);
        let mut registry = PluginRegistry::new();
        let err = registry.load_plugin(&root, &table).unwrap_err();
        assert!(matches!(err, PluginLoadError::Migration(_)));
    }

    #[test]
    fn duplicate_plugin_id_rejected() {
        let tmp = TempDir::new().unwrap();
        let root = write_notes_plugin(&tmp);
        let mut table = RegistrationTable::new();
        table.insert("notes", notes_register);
        let mut registry = PluginRegistry::new();
        registry.load_plugin(&root, &table).unwrap();
        // Loading the same dir again is the textbook double-load.
        let err = registry.load_plugin(&root, &table).unwrap_err();
        assert!(matches!(err, PluginLoadError::DuplicatePlugin(_)));
    }

    #[test]
    fn duplicate_tool_id_rejected() {
        let tmp = TempDir::new().unwrap();
        let root = write_notes_plugin(&tmp);
        let mut table = RegistrationTable::new();
        table.insert("notes", notes_register);
        let mut registry = PluginRegistry::new();
        // Pre-register a tool with the id the plugin is about to claim.
        registry.register_builtin_tool(Arc::new(DummyTool::new("notes.append")));
        let err = registry.load_plugin(&root, &table).unwrap_err();
        assert!(matches!(err, PluginLoadError::DuplicateTool { .. }));
    }
}
