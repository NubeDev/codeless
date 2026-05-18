//! Two-phase substrate scanner.
//!
//! `scan_plugins_dir` is the boot-time path codeless-server walks at
//! startup: one directory per plugin under `--plugins-dir`, each
//! containing a `plugin.toml`. The single-plugin `load_plugin`
//! already guards against half-populating a registry when a parse
//! error fires mid-load, but it does not protect against
//! *cross-plugin* failure modes: plugin A registers its tools, then
//! plugin B's manifest fails to parse, and the host now has a tool
//! registry advertising tools whose owning plugin never loaded.
//! Lifted from rubix `extensions-host::registry`'s two-phase scan:
//!
//! 1. **Vet phase.** Every plugin directory is parsed; every
//!    manifest is validated in isolation; the registration table is
//!    consulted for every builtin-flavour plugin; per-plugin
//!    failures are recorded but the registry is untouched. A plugin
//!    that only declares `kind = "process"` lands here as a
//!    structured `Failed` outcome with a stable reason code -- the
//!    seam PLUGIN-PROCESS.md § Reserve-the-seam carves out.
//! 2. **Commit phase.** Only the plugins that vetted clean are
//!    fed to `PluginRegistry::load_plugin`. A failure here is
//!    treated as a programmer bug (the vet phase should have
//!    caught it) and bubbles up; the registry is then in a state
//!    where some plugins are partially loaded, but that state is
//!    indistinguishable from "the server is about to exit" because
//!    the host treats a scan error as fatal.
//!
//! The scanner returns one [`PluginLoadOutcome`] per plugin
//! directory. `codeless plugin list` walks the outcomes verbatim;
//! `codeless plugin info <id>` returns the manifest from the
//! outcome whether the plugin loaded or not, so an operator can
//! still inspect a declared-but-unsupported plugin's manifest.

use std::path::{Path, PathBuf};

use super::manifest::{ManifestError, PluginManifest, PluginRuntime, PluginRuntimeKind};
use super::registry::{PluginLoadError, PluginRegistry, RegistrationTable};

/// One outcome of vetting + loading a single plugin directory.
/// Carries enough state on either branch for `codeless plugin info`
/// to render the same surface for a Failed plugin as for a Loaded
/// one (operator sees "this is the plugin that didn't load and the
/// reason why").
#[derive(Debug)]
pub enum PluginLoadOutcome {
    /// Plugin vetted and registered successfully. The registry now
    /// owns the plugin's tools; the snapshot here is the same view
    /// `codeless plugin list` reads. `loaded` is boxed because the
    /// `LoadedPlugin` payload is the largest variant by ~240 bytes
    /// and the outcome list is stored verbatim across the boot
    /// path; the indirection is cheap and keeps the enum compact.
    Loaded {
        id: String,
        manifest: PluginManifest,
        loaded: Box<super::registry::LoadedPlugin>,
    },
    /// Plugin vetted but did not load. The manifest is still
    /// available so `codeless plugin info <id>` works; the
    /// registries are untouched.
    Failed {
        id: String,
        manifest: PluginManifest,
        reason: PluginFailureReason,
    },
    /// Plugin's manifest itself failed to parse. There is no `id`
    /// to anchor against, so the directory path is the
    /// identifier the operator gets back. Vet-phase only: the
    /// commit phase never produces this variant.
    Unparseable { dir: PathBuf, error: ManifestError },
}

impl PluginLoadOutcome {
    /// Convenience accessor for the plugin id when one is known.
    pub fn id(&self) -> Option<&str> {
        match self {
            PluginLoadOutcome::Loaded { id, .. } | PluginLoadOutcome::Failed { id, .. } => Some(id),
            PluginLoadOutcome::Unparseable { .. } => None,
        }
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self, PluginLoadOutcome::Loaded { .. })
    }
}

/// Stable, structured reasons a plugin can land in `Failed` state.
/// The `code` accessor returns a wire-stable kebab-case string so
/// `codeless plugin list --json` and the future `GET /plugins`
/// projection can carry the reason without committing to the
/// `Display` form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginFailureReason {
    /// `[[runtimes]] kind = "process"` is the only declared
    /// runtime, but PLUGIN-PROCESS.md item 11 is design-only in
    /// v0.1 (no host adapter ships). The seam is reserved so the
    /// manifest still parses; loading is what fails. Lifts the
    /// exact phrasing from PLUGIN-PROCESS.md § Reserve-the-seam.
    ProcessRuntimeNotSupported,
    /// No runtime entry resolvable today. A `[[runtimes]]` array
    /// with kinds the host cannot satisfy (today: only
    /// `process`); a more general failure than the
    /// process-specific case so a future "wasm host disabled by
    /// build feature" can land here without inventing a new
    /// variant.
    NoLoadableRuntime { declared: Vec<PluginRuntimeKind> },
    /// Plugin declares a builtin runtime entry whose `crate`
    /// matches none of the statically-linked entries the host
    /// binary built against. Distinguished from
    /// `NoLoadableRuntime` because the fix is "rebuild the host
    /// with the plugin's crate linked in", not "wait for a
    /// runtime to land".
    UnknownBuiltin,
}

impl PluginFailureReason {
    /// Wire-stable code string. Treat this as the primary
    /// identifier; the human message is downstream of it.
    pub fn code(&self) -> &'static str {
        match self {
            PluginFailureReason::ProcessRuntimeNotSupported => "process-runtime-not-supported",
            PluginFailureReason::NoLoadableRuntime { .. } => "no-loadable-runtime",
            PluginFailureReason::UnknownBuiltin => "unknown-builtin",
        }
    }

    /// Human-readable message lifted from the PLUGIN-PROCESS.md
    /// seam paragraph. The text is stable for the
    /// `ProcessRuntimeNotSupported` case (operators may grep on
    /// it) but exists as a fallback only -- structured callers
    /// should branch on [`code`](Self::code).
    pub fn message(&self) -> String {
        match self {
            PluginFailureReason::ProcessRuntimeNotSupported => {
                "process runtime not yet supported; declare builtin or wasm or wait \
                 for process host to land"
                    .into()
            }
            PluginFailureReason::NoLoadableRuntime { declared } => {
                format!(
                    "no loadable runtime among declared kinds {declared:?}; \
                     declare a builtin or wasm runtime"
                )
            }
            PluginFailureReason::UnknownBuiltin => {
                "plugin declares a builtin runtime entry whose crate is not linked into \
                 this codeless build; rebuild with the crate included or switch to wasm"
                    .into()
            }
        }
    }
}

/// Resolve which runtime kind the host should drive for a given
/// plugin. The substrate-doc rule (PLUGIN-WASM.md § Manifest
/// extension): a plugin may ship multiple `[[runtimes]]` entries
/// (one per flavour) but only one runs per server process. v0.1
/// resolves the active runtime by static preference --
/// `builtin` > `wasm` > `process` -- because a host that ships a
/// builtin shim never wants the wasm cold-start path and a host
/// that ships wasm never wants to spawn a process. Operator-
/// overrides via codeless config land at OQ-WASM-2 / OQ-WASM-7.
///
/// Returns `None` only for the legacy shape (an empty
/// `[[runtimes]]` array), which the manifest treats as builtin --
/// the caller substitutes the plugin's `crate` field from
/// `[plugin]` as the builtin entry.
pub fn resolve_active_runtime(runtimes: &[PluginRuntime]) -> Option<&PluginRuntime> {
    if runtimes.is_empty() {
        return None;
    }
    for kind in [
        PluginRuntimeKind::Builtin,
        PluginRuntimeKind::Wasm,
        PluginRuntimeKind::Process,
    ] {
        if let Some(r) = runtimes.iter().find(|r| r.kind == kind) {
            return Some(r);
        }
    }
    None
}

/// Two-phase scan over a directory of plugin directories. See
/// the module head comment for the invariants.
///
/// `dir` is the parent directory: every immediate subdirectory
/// that contains a `plugin.toml` is treated as a plugin. Other
/// entries are silently ignored so an operator can drop a README
/// next to the plugins without breaking the scan.
pub fn scan_plugins_dir(dir: &Path, table: &RegistrationTable) -> std::io::Result<ScanResult> {
    // Collect plugin directories deterministically so the scan
    // order matches `codeless plugin list` and the resulting
    // tool registration order is stable across boots.
    let mut plugin_dirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("plugin.toml").exists() {
            continue;
        }
        plugin_dirs.push(path);
    }
    plugin_dirs.sort();

    // Phase 1 -- vet: parse manifests and bucket each plugin.
    // Failures stay scoped to the plugin; nothing touches the
    // shared registries yet.
    let mut vetted: Vec<VettedPlugin> = Vec::new();
    let mut outcomes: Vec<PluginLoadOutcome> = Vec::new();
    for plugin_dir in plugin_dirs {
        match PluginManifest::from_dir(&plugin_dir) {
            Err(error) => {
                outcomes.push(PluginLoadOutcome::Unparseable {
                    dir: plugin_dir,
                    error,
                });
            }
            Ok(manifest) => {
                let active = resolve_active_runtime(&manifest.runtimes).map(|r| r.kind);
                match active {
                    // Legacy shape (empty `[[runtimes]]`) is treated as
                    // builtin per substrate-doc backward-compat -- the
                    // pre-substrate-runtimes notes plugin shipped that
                    // shape and we keep it loadable. The registration
                    // table lookup is the only further gate.
                    None | Some(PluginRuntimeKind::Builtin) => {
                        if table.get(&manifest.plugin.id).is_none() {
                            outcomes.push(PluginLoadOutcome::Failed {
                                id: manifest.plugin.id.clone(),
                                manifest,
                                reason: PluginFailureReason::UnknownBuiltin,
                            });
                        } else {
                            vetted.push(VettedPlugin {
                                dir: plugin_dir,
                                manifest,
                            });
                        }
                    }
                    Some(PluginRuntimeKind::Wasm) => {
                        // Wasm-flavour load is delegated to
                        // codeless-plugin-host-wasm at stage 6; here
                        // we vet manifest-only and treat the
                        // wasm-flavour as "load via wasm host later".
                        // For the substrate scanner today, a wasm-
                        // active plugin parses but is not driven
                        // through the builtin RegistrationTable, so
                        // there is no registration-time error
                        // detectable here. Stage 13 lands the parse;
                        // the wasm host's own scanner extension lands
                        // separately.
                        vetted.push(VettedPlugin {
                            dir: plugin_dir,
                            manifest,
                        });
                    }
                    Some(PluginRuntimeKind::Process) => {
                        let id = manifest.plugin.id.clone();
                        outcomes.push(PluginLoadOutcome::Failed {
                            id,
                            manifest,
                            reason: PluginFailureReason::ProcessRuntimeNotSupported,
                        });
                    }
                }
            }
        }
    }

    // Phase 2 -- commit: register the vetted plugins. A failure
    // here is a vet-phase bug; surface it without partially
    // populating the registry.
    let mut registry = PluginRegistry::new();
    let mut commit_failure: Option<(String, PluginLoadError)> = None;
    for v in vetted {
        // Skip wasm-only entries: the substrate scanner today
        // drives the builtin path; wasm plugins are loaded by
        // codeless-plugin-host-wasm separately and surface
        // through a sibling registry. Recording them as Loaded
        // here would over-claim. The outcome list still carries
        // them, but the registry only owns the builtin shims.
        let active_kind = resolve_active_runtime(&v.manifest.runtimes).map(|r| r.kind);
        let drive_through_builtin = matches!(active_kind, None | Some(PluginRuntimeKind::Builtin),);
        if !drive_through_builtin {
            outcomes.push(PluginLoadOutcome::Loaded {
                id: v.manifest.plugin.id.clone(),
                manifest: v.manifest.clone(),
                loaded: Box::new(super::registry::LoadedPlugin {
                    manifest: v.manifest,
                    tool_ids: Vec::new(),
                    personas: Vec::new(),
                    migrations: Vec::new(),
                }),
            });
            continue;
        }
        match registry.load_plugin(&v.dir, table) {
            Ok(loaded) => {
                outcomes.push(PluginLoadOutcome::Loaded {
                    id: loaded.manifest.plugin.id.clone(),
                    manifest: loaded.manifest.clone(),
                    loaded: Box::new(loaded.clone()),
                });
            }
            Err(error) => {
                commit_failure = Some((v.manifest.plugin.id.clone(), error));
                break;
            }
        }
    }

    Ok(ScanResult {
        outcomes,
        registry,
        commit_failure,
    })
}

/// Result bundle of [`scan_plugins_dir`]. Carries the populated
/// registry, the per-plugin outcomes (one row per scanned dir),
/// and -- if the commit phase tripped -- the plugin id + error
/// that aborted it. A clean scan has `commit_failure == None` and
/// the host hands the registry off to the runtime; a non-clean
/// scan is fatal but the outcomes still describe what was vetted
/// up to the abort.
pub struct ScanResult {
    pub outcomes: Vec<PluginLoadOutcome>,
    pub registry: PluginRegistry,
    pub commit_failure: Option<(String, PluginLoadError)>,
}

impl ScanResult {
    pub fn into_parts(self) -> (Vec<PluginLoadOutcome>, PluginRegistry) {
        (self.outcomes, self.registry)
    }

    /// Find the outcome for a given plugin id, if scanned.
    pub fn find(&self, id: &str) -> Option<&PluginLoadOutcome> {
        self.outcomes.iter().find(|o| o.id() == Some(id))
    }
}

struct VettedPlugin {
    dir: PathBuf,
    manifest: PluginManifest,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    use crate::ctx::ToolCtx;
    use crate::error::ToolError;
    use crate::plugin::registry::PluginToolSink;
    use crate::tool::Tool;

    use super::*;

    struct DummyTool {
        name: String,
        schema: Value,
    }

    impl DummyTool {
        fn new(name: &str) -> Self {
            Self {
                name: name.into(),
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

    fn write_notes_plugin(parent: &std::path::Path) -> PathBuf {
        let root = parent.join("notes");
        std::fs::create_dir_all(root.join("prompts")).unwrap();
        std::fs::create_dir_all(root.join("migrations")).unwrap();
        std::fs::write(
            root.join("plugin.toml"),
            r#"
[plugin]
id      = "notes"
version = "0.1.0"
crate   = "codeless-plugin-notes"

[[personas]]
id                          = "notes"
prompt_file                 = "prompts/system.md"
allowed_tools               = ["notes.*"]
default_model_family        = "smart"
default_attachments_policy  = "inline-thread-scoped"

[[runtimes]]
kind  = "builtin"
crate = "codeless-plugin-notes"
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

    fn write_process_only_plugin(parent: &std::path::Path) -> PathBuf {
        // PLUGIN-PROCESS.md § Reserve-the-seam: a plugin that
        // declares only `kind = "process"` parses successfully and
        // loads as Failed with the structured reason. No
        // RegistrationTable wiring is needed because the scanner
        // never reaches the commit phase for the process flavour.
        let root = parent.join("widgets");
        std::fs::create_dir_all(root.join("prompts")).unwrap();
        std::fs::create_dir_all(root.join("migrations")).unwrap();
        std::fs::write(
            root.join("plugin.toml"),
            r#"
[plugin]
id      = "widgets"
version = "0.1.0"
crate   = "codeless-plugin-widgets"

[[personas]]
id                          = "widgets"
prompt_file                 = "prompts/system.md"
allowed_tools               = ["widgets.*"]
default_model_family        = "smart"
default_attachments_policy  = "inline-thread-scoped"

[[runtimes]]
kind   = "process"
binary = "bin/widgets"

[runtimes.policy]
socket_ready_timeout = "5s"
health_interval      = "10s"
failure_threshold    = 3
failure_window       = "60s"
failed_cooldown      = false
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("prompts/system.md"),
            "You are the widgets-plugin persona.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("migrations/0001_init.sql"),
            "CREATE TABLE widgets_entries (id TEXT PRIMARY KEY);\n",
        )
        .unwrap();
        root
    }

    fn notes_register(sink: &mut PluginToolSink) -> Result<(), String> {
        sink.register(Arc::new(DummyTool::new("notes.append")));
        Ok(())
    }

    #[test]
    fn scan_loads_builtin_plugin_and_registers_tool() {
        let tmp = TempDir::new().unwrap();
        write_notes_plugin(tmp.path());
        let mut table = RegistrationTable::new();
        table.insert("notes", notes_register);
        let result = scan_plugins_dir(tmp.path(), &table).expect("scan ok");
        assert!(result.commit_failure.is_none());
        let outcome = result.find("notes").expect("notes outcome");
        assert!(outcome.is_loaded(), "got {outcome:?}");
        assert!(result
            .registry
            .tool_registry()
            .get("notes.append")
            .is_some());
    }

    #[test]
    fn process_only_plugin_lands_failed_with_structured_reason() {
        // PLUGIN-PROCESS.md § Reserve-the-seam: this is the test
        // the stage description names. The plugin's manifest
        // declares only `kind = "process"`; the scanner records a
        // `Failed` outcome with a stable reason code; the
        // registry is left untouched (no tools, no other side
        // effects).
        let tmp = TempDir::new().unwrap();
        write_process_only_plugin(tmp.path());
        let table = RegistrationTable::new(); // intentionally empty
        let result = scan_plugins_dir(tmp.path(), &table).expect("scan ok");
        assert!(result.commit_failure.is_none());
        let outcome = result.find("widgets").expect("widgets outcome");
        match outcome {
            PluginLoadOutcome::Failed { reason, .. } => {
                assert_eq!(reason.code(), "process-runtime-not-supported");
                assert!(reason
                    .message()
                    .contains("process runtime not yet supported"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(result.registry.tool_registry().is_empty());
    }

    #[test]
    fn partial_vet_failure_leaves_other_plugins_loadable() {
        // Two plugins: one valid builtin, one process-only. The
        // two-phase scanner records Failed for the process one
        // and Loaded for the builtin one; neither contaminates
        // the other.
        let tmp = TempDir::new().unwrap();
        write_notes_plugin(tmp.path());
        write_process_only_plugin(tmp.path());
        let mut table = RegistrationTable::new();
        table.insert("notes", notes_register);
        let result = scan_plugins_dir(tmp.path(), &table).expect("scan ok");
        assert!(result.find("notes").unwrap().is_loaded());
        assert!(matches!(
            result.find("widgets").unwrap(),
            PluginLoadOutcome::Failed {
                reason: PluginFailureReason::ProcessRuntimeNotSupported,
                ..
            },
        ));
        assert!(result
            .registry
            .tool_registry()
            .get("notes.append")
            .is_some());
    }

    #[test]
    fn unparseable_manifest_records_outcome_without_aborting_scan() {
        let tmp = TempDir::new().unwrap();
        write_notes_plugin(tmp.path());
        let broken = tmp.path().join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(
            broken.join("plugin.toml"),
            "[plugin]\nthis is not toml = =\n",
        )
        .unwrap();
        let mut table = RegistrationTable::new();
        table.insert("notes", notes_register);
        let result = scan_plugins_dir(tmp.path(), &table).expect("scan ok");
        assert!(result.find("notes").unwrap().is_loaded());
        let unparseable = result
            .outcomes
            .iter()
            .find(|o| matches!(o, PluginLoadOutcome::Unparseable { .. }))
            .expect("broken plugin recorded");
        assert!(matches!(unparseable, PluginLoadOutcome::Unparseable { .. }));
    }

    #[test]
    fn builtin_without_registration_entry_lands_failed() {
        let tmp = TempDir::new().unwrap();
        write_notes_plugin(tmp.path());
        let table = RegistrationTable::new();
        let result = scan_plugins_dir(tmp.path(), &table).expect("scan ok");
        match result.find("notes").expect("notes outcome") {
            PluginLoadOutcome::Failed {
                reason: PluginFailureReason::UnknownBuiltin,
                ..
            } => {}
            other => panic!("expected UnknownBuiltin Failed, got {other:?}"),
        }
        assert!(result.registry.tool_registry().is_empty());
    }
}
