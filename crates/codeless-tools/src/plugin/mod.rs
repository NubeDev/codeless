//! Plugin substrate — DOCS/PLUGIN-SUBSTRATE.md item 6.
//!
//! Three submodules:
//!
//! - `manifest`: `plugin.toml` parser and validator.
//! - `migrations`: static prefix check on migration SQL so a plugin
//!   cannot squat on a codeless-owned table.
//! - `model_family`: codeless-side `fast/smart/reasoning` alias →
//!   provider model id mapping (substrate-doc rule: plugins never
//!   hardcode provider model ids).
//! - `registry`: the `PluginRegistry` and `load_plugin(path)` entry
//!   point.

pub mod manifest;
pub mod migrations;
pub mod model_family;
pub mod registry;

pub use manifest::{
    DataDir, ManifestError, MigrationsDir, PluginCapabilities, PluginManifest, PluginMetadata,
    PluginPersona, PluginRuntime, PluginRuntimeKind,
};
pub use migrations::{
    check_sql as check_migration_sql, load_migrations_dir, MigrationCheckError, PluginMigration,
};
pub use model_family::{
    is_known_family_alias, ModelFamilyConfig, ModelFamilyError, KNOWN_FAMILIES,
};
pub use registry::{
    LoadedPersona, LoadedPlugin, PluginLoadError, PluginRegistry, PluginToolSink, RegisterFn,
    RegistrationTable,
};
