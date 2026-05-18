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
pub mod mcp;
pub mod migrations;
pub mod model_family;
pub mod registry;
pub mod substrate;

pub use manifest::{
    DataDir, ManifestError, McpResourceBacking, McpTier, MigrationsDir, PluginCapabilities,
    PluginContributes, PluginFailedCooldown, PluginManifest, PluginMcp, PluginMcpDispatch,
    PluginMcpPrompt, PluginMcpResource, PluginMcpTool, PluginMetadata, PluginPersona,
    PluginRuntime, PluginRuntimeKind, PluginRuntimePolicy, ResolvedPluginRuntimePolicy,
};
pub use mcp::{
    check_parity as check_mcp_parity, mcp_listing_id, McpParityCheckInputs, McpParityError,
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
pub use substrate::{
    resolve_active_runtime, scan_plugins_dir, PluginFailureReason, PluginLoadOutcome, ScanResult,
};
