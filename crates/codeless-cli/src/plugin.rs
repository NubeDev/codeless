//! `codeless plugin {list,info}` — read-only inspection of the
//! statically-linked plugin registry. PS6 (DOCS/PLUGIN-SUBSTRATE.md
//! item 6).
//!
//! `list` enumerates every plugin directory under the search path
//! (one per immediate subdirectory containing a `plugin.toml`) and
//! prints id, version, crate, and registered-tool count. `info <id>`
//! dumps the full manifest plus the tools the plugin contributes to
//! the registry, plus the active `default_model_family` resolution
//! against the codeless config.
//!
//! Both verbs build a fresh `PluginRegistry` per invocation rather
//! than reaching into a long-lived server: the CLI is process-local
//! and the operator's mental model is "what would `codeless serve`
//! see if I restarted it right now?" -- not "what does the running
//! daemon think." The host-binary registration table is the canonical
//! source of which plugins are statically linked; if it is empty
//! (as it is in this commit, before plugin #0 lands) `list` reports
//! "no plugins compiled in" and exits successfully.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;

use codeless_tools::plugin::{
    ModelFamilyConfig, PluginManifest, PluginRegistry, RegistrationTable,
};

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// List plugins discovered under the search path, with the tools
    /// each one contributes after registration.
    List(ListArgs),
    /// Dump the full manifest, system prompt path, allowed-tools list,
    /// resolved provider model, and registered tool ids for one
    /// plugin.
    Info(InfoArgs),
}

#[derive(Debug, clap::Args)]
pub struct ListArgs {
    /// Directory whose immediate subdirectories are plugin roots
    /// (each subdir contains a `plugin.toml`). Defaults to
    /// `$CODELESS_PLUGINS_DIR` then `$XDG_CONFIG_HOME/codeless/plugins`
    /// then `~/.config/codeless/plugins`.
    #[arg(long, env = "CODELESS_PLUGINS_DIR")]
    pub plugins_dir: Option<PathBuf>,
    /// Path to a codeless config file containing the
    /// `[model_families]` table. Missing file falls back to the
    /// embedded defaults so the happy path stays one-step.
    #[arg(long, env = "CODELESS_CONFIG")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct InfoArgs {
    /// Plugin id (matches `plugin.id` in the manifest).
    pub id: String,
    #[arg(long, env = "CODELESS_PLUGINS_DIR")]
    pub plugins_dir: Option<PathBuf>,
    #[arg(long, env = "CODELESS_CONFIG")]
    pub config: Option<PathBuf>,
}

pub fn handle(verb: Verb) -> Result<ExitCode> {
    let table = host_registration_table();
    match verb {
        Verb::List(args) => list(args, &table),
        Verb::Info(args) => info(args, &table),
    }
}

/// The statically-linked registration table. Today this is empty:
/// plugin #0 (`notes`) lands in a follow-up stage and inserts its
/// `notes_register` here. The table lives in `codeless-cli` (not
/// `codeless-tools`) because only the host binary knows which plugin
/// crates it has compiled in -- exactly the substrate-doc MVP shape
/// for OQ-PS-2.
pub(crate) fn host_registration_table() -> RegistrationTable {
    RegistrationTable::new()
}

fn load_status_label(err: &codeless_tools::plugin::PluginLoadError) -> &'static str {
    use codeless_tools::plugin::PluginLoadError as E;
    match err {
        E::UnknownPlugin(_) => "no registration entry",
        E::RegistrationFailed(_, _) => "registration failed",
        E::Migration(_) => "migration check failed",
        E::Manifest(_) => "manifest error",
        E::DuplicatePlugin(_) => "duplicate plugin id",
        E::DuplicateTool { .. } => "duplicate tool id",
        E::ReadPrompt { .. } => "prompt file unreadable",
    }
}

fn list(args: ListArgs, table: &RegistrationTable) -> Result<ExitCode> {
    let dir = resolve_plugins_dir(args.plugins_dir)?;
    let model_cfg = resolve_model_config(args.config.as_deref())?;
    let mut registry = PluginRegistry::new();
    let plugin_dirs = discover_plugin_dirs(&dir)?;

    if plugin_dirs.is_empty() && table.ids().next().is_none() {
        println!(
            "no plugins compiled in (registration table empty) and no plugin directories under {}",
            dir.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Print every discovered manifest, falling back to manifest-only
    // data when the static registration table has no entry for the
    // plugin id. "Discovered but not compiled in" is the dominant
    // operator case during plugin authoring -- the plugin.toml exists
    // on disk before the host binary rebuilds against the new crate.
    println!(
        "{:<20} {:<10} {:<32} {:<6} STATUS",
        "ID", "VERSION", "CRATE", "TOOLS",
    );
    for plugin_dir in &plugin_dirs {
        match registry.load_plugin(plugin_dir, table) {
            Ok(loaded) => {
                println!(
                    "{:<20} {:<10} {:<32} {:<6} loaded",
                    loaded.manifest.plugin.id,
                    loaded.manifest.plugin.version,
                    loaded.manifest.plugin.crate_name,
                    loaded.tool_ids.len(),
                );
            }
            Err(err) => {
                // Fall back to manifest-only display so the operator
                // still sees the plugin id.
                match PluginManifest::from_dir(plugin_dir) {
                    Ok(m) => println!(
                        "{:<20} {:<10} {:<32} {:<6} {}",
                        m.plugin.id,
                        m.plugin.version,
                        m.plugin.crate_name,
                        "-",
                        load_status_label(&err),
                    ),
                    Err(merr) => println!(
                        "{:<20} {:<10} {:<32} {:<6} manifest error: {merr}",
                        plugin_dir
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("?"),
                        "-",
                        "-",
                        "-",
                    ),
                }
            }
        }
    }

    // List any registration entries the host has compiled in but
    // whose plugin directories were not found on disk -- helps an
    // operator spot a missing checkout vs. a missing build.
    let loaded: std::collections::HashSet<&str> = registry
        .plugins()
        .map(|p| p.manifest.plugin.id.as_str())
        .collect();
    for id in table.ids() {
        if !loaded.contains(id)
            && !plugin_dirs.iter().any(|d| {
                d.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| n == id)
            })
        {
            println!(
                "{:<20} {:<10} {:<32} {:<6} no plugin.toml under {}",
                id,
                "-",
                "-",
                "-",
                dir.display(),
            );
        }
    }

    // Make the model family resolution visible at the bottom so the
    // operator can confirm what an alias maps to without a second
    // command. This output is read by humans, not parsed -- keeping it
    // a separate section is the readable shape.
    println!();
    println!(
        "model families (config: {}):",
        args.config
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<defaults>".to_string())
    );
    for (alias, model) in model_cfg.entries() {
        println!("  {alias:<10} -> {model}");
    }

    Ok(ExitCode::SUCCESS)
}

fn info(args: InfoArgs, table: &RegistrationTable) -> Result<ExitCode> {
    let dir = resolve_plugins_dir(args.plugins_dir)?;
    let model_cfg = resolve_model_config(args.config.as_deref())?;
    let mut registry = PluginRegistry::new();
    for plugin_dir in discover_plugin_dirs(&dir)? {
        // Best-effort: skip directories whose manifest cannot parse
        // when the operator is asking about a *different* plugin.
        let _ = registry.load_plugin(&plugin_dir, table);
    }

    let plugin = registry
        .get(&args.id)
        .ok_or_else(|| anyhow!("plugin `{}` is not loaded", args.id))?;

    println!("id:      {}", plugin.manifest.plugin.id);
    println!("version: {}", plugin.manifest.plugin.version);
    println!("crate:   {}", plugin.manifest.plugin.crate_name);
    if let Some(root) = &plugin.manifest.root {
        println!("root:    {}", root.display());
    }
    println!(
        "migrations_dir: {}",
        plugin.manifest.migrations.dir.display()
    );
    println!("data_dir:       {}", plugin.manifest.data.dir.display());

    println!();
    println!("tools (from registry, not manifest -- substrate-doc item 6):");
    if plugin.tool_ids.is_empty() {
        println!("  (none)");
    } else {
        for t in &plugin.tool_ids {
            println!("  {t}");
        }
    }

    println!();
    println!("personas:");
    for p in &plugin.personas {
        let resolved = model_cfg
            .resolve(&p.default_model_family)
            .unwrap_or("<unresolved>");
        println!("  - id: {}", p.id);
        println!("    name: {}", p.name);
        println!("    allowed_tools: {:?}", p.allowed_tools);
        println!(
            "    default_model_family: {} -> {}",
            p.default_model_family, resolved
        );
        println!(
            "    default_attachments_policy: {}",
            p.default_attachments_policy
        );
        println!("    description: {}", p.description);
    }

    println!();
    println!("migrations ({} file(s)):", plugin.migrations.len());
    for m in &plugin.migrations {
        println!("  {}", m.path.display());
    }

    Ok(ExitCode::SUCCESS)
}

fn resolve_plugins_dir(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p);
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("codeless").join("plugins"));
    }
    let home =
        std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set; pass --plugins-dir"))?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("codeless")
        .join("plugins"))
}

fn resolve_model_config(config: Option<&Path>) -> Result<ModelFamilyConfig> {
    match config {
        Some(path) => ModelFamilyConfig::from_file(path)
            .with_context(|| format!("load model family config from {}", path.display())),
        None => Ok(ModelFamilyConfig::builtin()),
    }
}

fn discover_plugin_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in
        std::fs::read_dir(root).with_context(|| format!("read plugins dir {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("plugin.toml").exists() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}
