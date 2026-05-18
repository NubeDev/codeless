//! `plugin.toml` parser for the substrate (DOCS/PLUGIN-SUBSTRATE.md
//! item 6).
//!
//! The manifest is the one place an operator (or `codeless plugin
//! list/info`) reads to understand what a plugin claims about itself
//! without invoking any plugin code. The canonical list of tools comes
//! from the registry — see `crate::plugin::registry` — so this file
//! deliberately does *not* enumerate tools; if it did, the two sources
//! would skew.
//!
//! The shape mirrors the substrate-doc example one-for-one. Fields are
//! validated at parse time so a malformed manifest fails at
//! `load_plugin` rather than at the first agent turn.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use codeless_types::allowed_tools::{validate_patterns, AllowedToolPatternError};

use super::model_family::is_known_family_alias;

/// Parsed `plugin.toml`. Paths inside (`prompt_file`, `migrations.dir`,
/// `data.dir`) are stored verbatim; resolution against the plugin
/// directory happens in `load_plugin` so the parser itself stays I/O
/// free and unit-testable from a `&str`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginManifest {
    pub plugin: PluginMetadata,
    pub personas: Vec<PluginPersona>,
    pub migrations: MigrationsDir,
    pub data: DataDir,
    /// Per `PLUGIN-WASM.md § Manifest extension (item 6 addendum)`: a
    /// plugin declares one or more `[[runtimes]]` blocks. v0.1
    /// accepts at most one entry; if both a `builtin` and a `wasm`
    /// block are present the manifest parser fails (the operator
    /// must pick one via codeless config, not by shipping both).
    /// Empty when the manifest omits the block, which is the
    /// legacy shape -- a plugin without a `[[runtimes]]` entry is
    /// treated as builtin for backward compatibility with the
    /// pre-substrate-runtimes notes plugin.
    pub runtimes: Vec<PluginRuntime>,
    /// Absolute path of the directory the manifest was loaded from.
    /// `None` when parsed from an in-memory string (test path).
    pub root: Option<PathBuf>,
}

/// One `[[runtimes]]` entry. The `capabilities` block is the
/// load-bearing piece for the WASM-flavour capability sandbox; the
/// builtin flavour ignores it (capabilities only apply to host-
/// linker-mediated access). Validated at parse time so a
/// malformed entry fails at `load_plugin` rather than at the first
/// agent turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntime {
    pub kind: PluginRuntimeKind,
    /// For `kind = "builtin"`: the crate that ships the
    /// registration entry. For `kind = "wasm"`: ignored. Optional
    /// so an operator-installed `.wasm` artefact does not need a
    /// crate name. Aliased to TOML `crate` to match the doc
    /// example (`[[runtimes]] kind = "builtin" crate = "..."`),
    /// while keeping the Rust field name out of the reserved-word
    /// space.
    #[serde(default, rename = "crate")]
    pub crate_name: Option<String>,
    /// For `kind = "wasm"`: the relative path under the plugin
    /// directory of the `.wasm` component artefact. For
    /// `kind = "builtin"`: ignored.
    #[serde(default)]
    pub artefact: Option<PathBuf>,
    /// For `kind = "process"`: the relative path of the supervised
    /// plugin binary. Process flavour is a manifest-only seam in
    /// v0.1 (PLUGIN-PROCESS.md item 11): the parser strict-validates
    /// the field's presence but no host adapter spawns it.
    #[serde(default)]
    pub binary: Option<PathBuf>,
    /// `[runtimes.capabilities]` sub-block. Defaults to the empty
    /// default-deny set; a manifest that omits the block gets
    /// `Capabilities::default()` which keeps every host-implemented
    /// interface unlinked from the per-plugin linker.
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    /// `[runtimes.policy]` sub-block. Only `kind = "process"`
    /// honours it (PLUGIN-PROCESS.md § Manifest): supervisor
    /// timeouts, circuit-breaker thresholds, opt-in retry cooldown.
    /// Builtin and wasm flavours reject every field
    /// (`deny_unknown_fields` on the empty subtype `()` keeps the
    /// rejection structural rather than a side check).
    #[serde(default)]
    pub policy: PluginRuntimePolicy,
}

/// Runtime flavour discriminant. The doc allows three kinds in
/// v0.1: `builtin`, `wasm`, and the reserved-but-not-implemented
/// `process`. Strict-validate per `PLUGIN-WASM.md § Manifest
/// extension`: unknown values are a parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginRuntimeKind {
    Builtin,
    Wasm,
    /// Reserved for PLUGIN-PROCESS.md item 11; the manifest parser
    /// accepts the value so a future plugin can declare it without
    /// breaking, but the registry rejects it at load time since no
    /// host adapter ships in this stage. Manifest-only seam per the
    /// job spec.
    Process,
}

/// `[runtimes.capabilities]` block. Mirrors the doc's grant
/// vocabulary one-for-one. Every field defaults to the empty /
/// false default-deny value; the host crate's
/// `codeless_plugin_host_wasm::Capabilities` is the typed mirror.
///
/// Limits like `fuel`, `memory_max_bytes`, `deadline_ms` are
/// **deliberately absent** here per OQ-WASM-5: the plugin manifest
/// cannot enlarge its own sandbox. The codeless `config.toml`
/// `[plugins.<id>]` block carries those overrides, and a plugin
/// `[runtimes.capabilities]` block that tries to set any of them
/// trips `deny_unknown_fields` -> manifest parse error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCapabilities {
    /// Host filesystem path prefixes the plugin's
    /// `codeless:fs/probe.read-file` host implementation may open.
    /// Empty -> the interface is not linked at all (default-deny);
    /// a non-empty list links the interface and the host
    /// implementation gates each requested path against it.
    #[serde(default)]
    pub fs: Vec<String>,
    /// Outbound-HTTP grants. Reserved for the future `codeless:
    /// http/client` interface; today any non-empty value parses
    /// successfully but no host implementation backs it.
    #[serde(default)]
    pub http: Vec<String>,
    /// Whether the plugin may import `wasi:clocks/wall-clock`.
    /// Stored as an explicit bool because the doc treats it as a
    /// single switch, not a list.
    #[serde(default)]
    pub wall_clock: bool,
    /// Attachment scopes. Accepted values: `"read"` and `"write"`.
    /// The host's
    /// `codeless_plugin_host_wasm::Capabilities::attachments_*`
    /// fields are derived from this list; an empty list keeps the
    /// `codeless:attachments/store` interface out of the linker
    /// entirely.
    #[serde(default)]
    pub attachments: Vec<String>,
}

/// `[runtimes.policy]` sub-block. The process flavour's supervisor
/// reads these knobs; builtin and wasm reject them at validate-time
/// (kind-specific block rule). Duration fields accept the doc shape
/// (`"5s"`, `"100ms"`, `"2m"`) and parse into `std::time::Duration`
/// at manifest-load. Numeric fields keep raw integers. The fields
/// are individually optional so a manifest may declare the block
/// and omit values it wants supervisor defaults for; an entirely
/// absent `[runtimes.policy]` yields `PluginRuntimePolicy::default()`,
/// indistinguishable from "all defaults".
///
/// `failed_cooldown` is a TOML sum-type: literal `false` disables
/// the cooldown ("Failed is sticky", the doc's default), a duration
/// string switches to "retry forever with this cooldown".
/// `deny_unknown_fields` keeps a misspelt knob from silently being
/// the supervisor's default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimePolicy {
    #[serde(default)]
    pub socket_ready_timeout: Option<String>,
    #[serde(default)]
    pub health_interval: Option<String>,
    #[serde(default)]
    pub failure_threshold: Option<u32>,
    #[serde(default)]
    pub failure_window: Option<String>,
    #[serde(default)]
    pub failed_cooldown: Option<PluginFailedCooldown>,
}

/// TOML sum-type for `failed_cooldown`. The PLUGIN-PROCESS.md
/// example uses literal `false` for the default (sticky-Failed)
/// case and a duration string (`"60s"`) for the headless-retry
/// case. `#[serde(untagged)]` is what makes TOML accept either
/// shape against the same field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginFailedCooldown {
    /// `false` -> the supervisor keeps `Failed` sticky; an
    /// operator must call `enable()` to retry. The doc's default.
    /// A literal `true` is not meaningful (a `true` cooldown is
    /// "retry instantly", which is the absence of a cooldown);
    /// validated below.
    Disabled(bool),
    /// Duration string. Validated via `parse_duration` at
    /// manifest-load.
    After(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMetadata {
    /// Stable identifier. Forms the namespace prefix for every SQL
    /// table the plugin owns (`<id>_<table>`) and the lookup key
    /// `codeless plugin info` accepts. Lowercase ASCII letters, digits,
    /// and underscore only; must start with a letter so a future
    /// numeric `<id>_…` lookup parser cannot mistake an id for a
    /// column.
    pub id: String,
    pub version: String,
    /// Crate name the registration entry lives in. Today this is a
    /// hint only — the static registration table the host binary
    /// builds at startup is keyed by `id`, not crate name. Stored so
    /// `codeless plugin info` can show the operator where the code
    /// lives without depending on the registry.
    #[serde(rename = "crate")]
    pub crate_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPersona {
    pub id: String,
    /// Path (relative to the plugin dir) of the system-prompt markdown
    /// file. `load_plugin` reads the contents into the persona row.
    pub prompt_file: PathBuf,
    pub allowed_tools: Vec<String>,
    /// Codeless-side family alias (`fast`, `smart`, `reasoning`). Must
    /// be one of the known aliases the runtime can resolve via
    /// `ModelFamilyConfig`. Plugin authors hardcoding a provider model
    /// id is exactly the failure the substrate doc rules out.
    pub default_model_family: String,
    /// Free-form policy string. The runtime accepts any non-empty
    /// value; the substrate-doc example is `inline-thread-scoped`.
    pub default_attachments_policy: String,
    /// Optional display fields. Defaulted so a plugin author can ship
    /// a minimal entry — the persona picker falls back on the id.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationsDir {
    pub dir: PathBuf,
}

impl Default for MigrationsDir {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("migrations"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataDir {
    pub dir: PathBuf,
}

impl Default for DataDir {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("domains"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("read plugin.toml at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse plugin.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("plugin id `{id}` is invalid: {reason}")]
    InvalidId { id: String, reason: &'static str },
    #[error("plugin version `{0}` is empty")]
    EmptyVersion(String),
    #[error("plugin crate name is empty")]
    EmptyCrate,
    #[error("persona `{persona}` allowed_tools[{index}]: {error}")]
    BadAllowedTool {
        persona: String,
        index: usize,
        error: AllowedToolPatternError,
    },
    #[error(
        "persona `{persona}`: default_model_family `{family}` is not a known codeless alias \
         (fast / smart / reasoning); plugins must not hardcode provider model ids"
    )]
    UnknownModelFamily { persona: String, family: String },
    #[error("persona `{persona}`: default_attachments_policy is empty")]
    EmptyAttachmentsPolicy { persona: String },
    #[error("persona id `{id}` is invalid: {reason}")]
    InvalidPersonaId { id: String, reason: &'static str },
    #[error("persona id `{0}` appears more than once in plugin.toml")]
    DuplicatePersona(String),
    #[error("no personas declared in plugin.toml")]
    NoPersonas,
    /// Per `PLUGIN-WASM.md § Manifest extension`: a plugin may
    /// declare at most one active runtime per server process. If a
    /// manifest ships both a `builtin` and a `wasm` block, codeless
    /// config picks one at server start -- but the manifest itself
    /// must not encode the ambiguity. Two entries of the same kind
    /// is also a flat error.
    /// Per `PLUGIN-WASM.md § Manifest extension`: a plugin may
    /// ship more than one `[[runtimes]]` entry (e.g. both a
    /// `builtin` crate and a `wasm` artefact), but each `kind` may
    /// appear at most once -- the codeless config picks the active
    /// runtime by kind, not by ordinal position.
    #[error("plugin runtime kind `{0:?}` appears more than once in `[[runtimes]]`")]
    DuplicateRuntimeKind(PluginRuntimeKind),
    /// `[runtimes.capabilities] attachments` accepts only `"read"`
    /// and `"write"`. Anything else is a manifest parse error.
    #[error(
        "runtime capabilities.attachments[{index}] = `{value}` is not one of \"read\" / \"write\""
    )]
    BadAttachmentsScope { index: usize, value: String },
    /// The `process` runtime kind is a manifest-only seam in v0.1
    /// (no host adapter ships). A plugin that declares it loads
    /// successfully at parse time -- so the operator gets a clear
    /// "this plugin would need a process runtime" signal from
    /// `codeless plugin info` -- but the registry refuses to wire
    /// it up. Surfaced here so a future `load_plugin` can return
    /// it without manifest-parse changes.
    #[error(
        "plugin runtime `process` is reserved (PLUGIN-PROCESS.md); v0.1 ships no host adapter"
    )]
    ProcessRuntimeReserved,
    /// Kind-specific block validation: a `[[runtimes]]` entry of
    /// the given kind declared a field that does not apply to it
    /// (e.g. `binary` on `kind = "builtin"`). The doc lists which
    /// fields each kind accepts; mixing them is a manifest error,
    /// not a silent ignore -- the substrate registry treats the
    /// manifest as the canonical declaration of intent.
    #[error(
        "runtime kind `{kind:?}` cannot carry field `{field}`; \
         see PLUGIN-WASM.md / PLUGIN-PROCESS.md `[[runtimes]]` shape"
    )]
    RuntimeFieldNotApplicable {
        kind: PluginRuntimeKind,
        field: &'static str,
    },
    /// Kind-specific block validation: a `[[runtimes]]` entry of
    /// the given kind is missing a field that is required for it
    /// (e.g. `binary` on `kind = "process"`).
    #[error(
        "runtime kind `{kind:?}` requires field `{field}`; \
         see PLUGIN-WASM.md / PLUGIN-PROCESS.md `[[runtimes]]` shape"
    )]
    RuntimeFieldMissing {
        kind: PluginRuntimeKind,
        field: &'static str,
    },
    /// `[runtimes.policy]` duration string failed to parse. The
    /// supervisor reads durations as `<integer><unit>` with units
    /// `ms`, `s`, `m`. Anything else is a manifest error rather
    /// than a runtime surprise.
    #[error("runtime policy field `{field}` = `{value}` is not a valid duration ({reason})")]
    BadRuntimePolicyDuration {
        field: &'static str,
        value: String,
        reason: &'static str,
    },
    /// `[runtimes.policy] failed_cooldown = true` is meaningless
    /// (a `true` cooldown is "retry instantly", indistinguishable
    /// from no cooldown). The PLUGIN-PROCESS.md example uses
    /// literal `false` for sticky-Failed or a duration string for
    /// headless retry; nothing else is accepted.
    #[error(
        "runtime policy `failed_cooldown` accepts `false` or a duration string \
         (e.g. \"60s\"); `true` is not meaningful"
    )]
    BadFailedCooldownLiteral,
    /// Cross-cutting: `[runtimes.policy]` only applies to
    /// `kind = "process"`. Builtin / wasm entries that declare it
    /// fail here so the manifest cannot pretend to configure a
    /// supervisor that does not exist for that flavour.
    #[error(
        "runtime kind `{0:?}` does not honour `[runtimes.policy]`; \
         policy applies to `kind = \"process\"` only"
    )]
    RuntimePolicyNotApplicable(PluginRuntimeKind),
}

/// On-disk TOML shape. The public manifest layers parsed-and-validated
/// data on top so callers can rely on invariants (e.g. ids are valid)
/// instead of re-checking everywhere.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    plugin: PluginMetadata,
    #[serde(default, rename = "personas")]
    personas: Vec<PluginPersona>,
    #[serde(default)]
    migrations: Option<MigrationsDir>,
    #[serde(default)]
    data: Option<DataDir>,
    #[serde(default, rename = "runtimes")]
    runtimes: Vec<PluginRuntime>,
}

impl PluginManifest {
    /// Parse a TOML string. `root` is optional; populated when the
    /// caller has a real on-disk directory and `None` for unit tests
    /// that work off in-memory strings.
    pub fn from_str(text: &str, root: Option<PathBuf>) -> Result<Self, ManifestError> {
        let parsed: OnDisk = toml::from_str(text)?;
        validate_plugin_id(&parsed.plugin.id)?;
        if parsed.plugin.version.trim().is_empty() {
            return Err(ManifestError::EmptyVersion(parsed.plugin.version));
        }
        if parsed.plugin.crate_name.trim().is_empty() {
            return Err(ManifestError::EmptyCrate);
        }
        if parsed.personas.is_empty() {
            return Err(ManifestError::NoPersonas);
        }

        let mut seen = std::collections::HashSet::new();
        for persona in &parsed.personas {
            validate_persona_id(&persona.id)?;
            if !seen.insert(persona.id.clone()) {
                return Err(ManifestError::DuplicatePersona(persona.id.clone()));
            }
            if let Err((idx, err)) = validate_patterns(&persona.allowed_tools) {
                return Err(ManifestError::BadAllowedTool {
                    persona: persona.id.clone(),
                    index: idx,
                    error: err,
                });
            }
            if !is_known_family_alias(&persona.default_model_family) {
                return Err(ManifestError::UnknownModelFamily {
                    persona: persona.id.clone(),
                    family: persona.default_model_family.clone(),
                });
            }
            if persona.default_attachments_policy.trim().is_empty() {
                return Err(ManifestError::EmptyAttachmentsPolicy {
                    persona: persona.id.clone(),
                });
            }
        }

        let mut seen_kinds = std::collections::HashSet::new();
        for runtime in &parsed.runtimes {
            if !seen_kinds.insert(runtime.kind) {
                return Err(ManifestError::DuplicateRuntimeKind(runtime.kind));
            }
            validate_runtime_kind_block(runtime)?;
            for (idx, scope) in runtime.capabilities.attachments.iter().enumerate() {
                if scope != "read" && scope != "write" {
                    return Err(ManifestError::BadAttachmentsScope {
                        index: idx,
                        value: scope.clone(),
                    });
                }
            }
        }

        Ok(Self {
            plugin: parsed.plugin,
            personas: parsed.personas,
            migrations: parsed.migrations.unwrap_or_default(),
            data: parsed.data.unwrap_or_default(),
            runtimes: parsed.runtimes,
            root,
        })
    }

    /// Read `plugin.toml` from a plugin directory.
    pub fn from_dir(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&path).map_err(|source| ManifestError::Read {
            path: path.clone(),
            source,
        })?;
        Self::from_str(&text, Some(dir.to_path_buf()))
    }

    /// Resolve a relative path inside the plugin dir against `root`.
    /// Returns the raw path when no root is set (test path).
    pub fn resolve(&self, rel: &Path) -> PathBuf {
        match self.root.as_deref() {
            Some(root) => root.join(rel),
            None => rel.to_path_buf(),
        }
    }
}

/// Kind-specific block validator. Builtin accepts `crate` and no
/// other kind-specific fields; wasm accepts `artefact` and no
/// other; process accepts `binary` (required) plus an optional
/// `[runtimes.policy]` block. Anything else trips an error rather
/// than a silent ignore -- a manifest that declares `binary` under
/// `kind = "builtin"` would otherwise leave the operator wondering
/// why their plugin won't spawn.
fn validate_runtime_kind_block(runtime: &PluginRuntime) -> Result<(), ManifestError> {
    let policy_empty = runtime.policy == PluginRuntimePolicy::default();
    match runtime.kind {
        PluginRuntimeKind::Builtin => {
            if runtime.crate_name.is_none() {
                return Err(ManifestError::RuntimeFieldMissing {
                    kind: runtime.kind,
                    field: "crate",
                });
            }
            if runtime.artefact.is_some() {
                return Err(ManifestError::RuntimeFieldNotApplicable {
                    kind: runtime.kind,
                    field: "artefact",
                });
            }
            if runtime.binary.is_some() {
                return Err(ManifestError::RuntimeFieldNotApplicable {
                    kind: runtime.kind,
                    field: "binary",
                });
            }
            if !policy_empty {
                return Err(ManifestError::RuntimePolicyNotApplicable(runtime.kind));
            }
        }
        PluginRuntimeKind::Wasm => {
            if runtime.artefact.is_none() {
                return Err(ManifestError::RuntimeFieldMissing {
                    kind: runtime.kind,
                    field: "artefact",
                });
            }
            if runtime.crate_name.is_some() {
                return Err(ManifestError::RuntimeFieldNotApplicable {
                    kind: runtime.kind,
                    field: "crate",
                });
            }
            if runtime.binary.is_some() {
                return Err(ManifestError::RuntimeFieldNotApplicable {
                    kind: runtime.kind,
                    field: "binary",
                });
            }
            if !policy_empty {
                return Err(ManifestError::RuntimePolicyNotApplicable(runtime.kind));
            }
        }
        PluginRuntimeKind::Process => {
            if runtime.binary.is_none() {
                return Err(ManifestError::RuntimeFieldMissing {
                    kind: runtime.kind,
                    field: "binary",
                });
            }
            if runtime.crate_name.is_some() {
                return Err(ManifestError::RuntimeFieldNotApplicable {
                    kind: runtime.kind,
                    field: "crate",
                });
            }
            if runtime.artefact.is_some() {
                return Err(ManifestError::RuntimeFieldNotApplicable {
                    kind: runtime.kind,
                    field: "artefact",
                });
            }
            // Process accepts a capabilities block (attachments
            // read/write is the only reverse interface in the
            // process v0.1 wire shape -- see PLUGIN-PROCESS.md
            // § Capability surface); validation of attachment
            // scopes happens in the shared loop above.
            validate_policy(&runtime.policy)?;
        }
    }
    Ok(())
}

/// Parse-time validation of the `[runtimes.policy]` block. The
/// parsed values are not stored -- callers re-parse via
/// [`PluginRuntimePolicy::resolve`] when they need typed durations
/// -- but running the parse here means a malformed `"5seconds"`
/// fails at `load_plugin`, not at the first supervisor tick.
fn validate_policy(policy: &PluginRuntimePolicy) -> Result<(), ManifestError> {
    if let Some(v) = &policy.socket_ready_timeout {
        parse_duration("socket_ready_timeout", v)?;
    }
    if let Some(v) = &policy.health_interval {
        parse_duration("health_interval", v)?;
    }
    if let Some(v) = &policy.failure_window {
        parse_duration("failure_window", v)?;
    }
    if let Some(cooldown) = &policy.failed_cooldown {
        match cooldown {
            PluginFailedCooldown::Disabled(false) => {}
            PluginFailedCooldown::Disabled(true) => {
                return Err(ManifestError::BadFailedCooldownLiteral);
            }
            PluginFailedCooldown::After(s) => {
                parse_duration("failed_cooldown", s)?;
            }
        }
    }
    Ok(())
}

/// Minimal duration parser for `<integer><unit>` with `ms`, `s`,
/// `m`. Modeled on the doc's examples (`"5s"`, `"10s"`, `"60s"`)
/// and intentionally narrow: keeping the substrate manifest
/// duration grammar tiny means the operator's mental model of
/// "what works" matches the parser one-for-one, with no surprises
/// hiding inside humantime's broader accept-set. A future bump to
/// humantime can subsume this without manifest changes.
pub fn parse_duration(field: &'static str, value: &str) -> Result<Duration, ManifestError> {
    let trimmed = value.trim();
    let (digits, unit) = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| trimmed.split_at(i))
        .unwrap_or((trimmed, ""));
    if digits.is_empty() {
        return Err(ManifestError::BadRuntimePolicyDuration {
            field,
            value: value.into(),
            reason: "missing numeric prefix",
        });
    }
    let n: u64 = digits
        .parse()
        .map_err(|_| ManifestError::BadRuntimePolicyDuration {
            field,
            value: value.into(),
            reason: "numeric prefix too large",
        })?;
    let d = match unit.trim() {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        "" => {
            return Err(ManifestError::BadRuntimePolicyDuration {
                field,
                value: value.into(),
                reason: "missing unit (expected ms / s / m)",
            });
        }
        _ => {
            return Err(ManifestError::BadRuntimePolicyDuration {
                field,
                value: value.into(),
                reason: "unknown unit (expected ms / s / m)",
            });
        }
    };
    Ok(d)
}

impl PluginRuntimePolicy {
    /// Resolve the raw manifest strings into the typed durations
    /// the supervisor consumes. Returns `None` for any field the
    /// manifest omits; the supervisor substitutes its own defaults
    /// (PLUGIN-PROCESS.md `HostPolicy::*` defaults). Pure function;
    /// failures here would have been caught at manifest parse.
    pub fn resolve(&self) -> ResolvedPluginRuntimePolicy {
        ResolvedPluginRuntimePolicy {
            socket_ready_timeout: self
                .socket_ready_timeout
                .as_deref()
                .map(|v| parse_duration("socket_ready_timeout", v).expect("validated at parse")),
            health_interval: self
                .health_interval
                .as_deref()
                .map(|v| parse_duration("health_interval", v).expect("validated at parse")),
            failure_threshold: self.failure_threshold,
            failure_window: self
                .failure_window
                .as_deref()
                .map(|v| parse_duration("failure_window", v).expect("validated at parse")),
            failed_cooldown: match &self.failed_cooldown {
                None | Some(PluginFailedCooldown::Disabled(_)) => None,
                Some(PluginFailedCooldown::After(s)) => {
                    Some(parse_duration("failed_cooldown", s).expect("validated at parse"))
                }
            },
        }
    }
}

/// Typed mirror of [`PluginRuntimePolicy`]. The supervisor reads
/// this; the manifest writes the string shape. Splitting raw vs
/// resolved lets the manifest still round-trip through Serialize
/// without losing operator-typed `"5s"` form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPluginRuntimePolicy {
    pub socket_ready_timeout: Option<Duration>,
    pub health_interval: Option<Duration>,
    pub failure_threshold: Option<u32>,
    pub failure_window: Option<Duration>,
    pub failed_cooldown: Option<Duration>,
}

fn validate_plugin_id(id: &str) -> Result<(), ManifestError> {
    // Plugin id is the table-name prefix, so we restrict it to what
    // SQLite will accept as an unquoted identifier component: lowercase
    // ASCII letter or underscore start, then letters/digits/underscore.
    // The migration runner builds string-compares against `<id>_`, so
    // any character outside this set would silently turn into a
    // sqlinjection-shaped foot-gun the next time someone tried to
    // generalise the namespace check.
    if id.is_empty() {
        return Err(ManifestError::InvalidId {
            id: id.into(),
            reason: "empty",
        });
    }
    let mut chars = id.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_lowercase() || first == '_') {
        return Err(ManifestError::InvalidId {
            id: id.into(),
            reason: "must start with a lowercase ASCII letter or underscore",
        });
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(ManifestError::InvalidId {
                id: id.into(),
                reason: "only lowercase ASCII letters, digits, and underscore",
            });
        }
    }
    Ok(())
}

fn validate_persona_id(id: &str) -> Result<(), ManifestError> {
    // Persona ids land in `personas.id` (TEXT PRIMARY KEY); they are
    // also referenced from `assistant_threads.persona_id`. We require
    // them to be ASCII printable, no whitespace, no quotes, no NULs --
    // the substrate-doc convention is `<plugin_id>:<slug>` or
    // `builtin:<slug>`, but enforcing the prefix here is too strict
    // (plugin #0 `notes` will likely ship as just `notes`). The plugin
    // loader prefixes with `<plugin_id>:` when registering if the
    // manifest entry lacks a colon -- see registry.rs.
    if id.is_empty() {
        return Err(ManifestError::InvalidPersonaId {
            id: id.into(),
            reason: "empty",
        });
    }
    for c in id.chars() {
        if c.is_whitespace() || c == '"' || c == '\'' || c == '\0' {
            return Err(ManifestError::InvalidPersonaId {
                id: id.into(),
                reason: "no whitespace, quotes, or NUL",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[plugin]
id        = "notes"
version   = "0.1.0"
crate     = "codeless-plugin-notes"

[[personas]]
id                          = "notes"
prompt_file                 = "prompts/system.md"
allowed_tools               = ["notes.*", "attachments.read"]
default_model_family        = "smart"
default_attachments_policy  = "inline-thread-scoped"
"#;

    #[test]
    fn parses_substrate_doc_shape() {
        let m = PluginManifest::from_str(SAMPLE, None).expect("valid manifest");
        assert_eq!(m.plugin.id, "notes");
        assert_eq!(m.plugin.crate_name, "codeless-plugin-notes");
        assert_eq!(m.personas.len(), 1);
        assert_eq!(m.personas[0].allowed_tools, ["notes.*", "attachments.read"]);
        // Defaults landed:
        assert_eq!(m.migrations.dir, PathBuf::from("migrations"));
        assert_eq!(m.data.dir, PathBuf::from("domains"));
    }

    #[test]
    fn rejects_uppercase_plugin_id() {
        let bad = SAMPLE.replace(r#"id        = "notes""#, r#"id        = "Notes""#);
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidId { .. }));
    }

    #[test]
    fn rejects_unknown_model_family() {
        let bad = SAMPLE.replace("\"smart\"", "\"claude-opus-4-7\"");
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::UnknownModelFamily { .. }));
    }

    #[test]
    fn rejects_regex_in_allowed_tools() {
        let bad = SAMPLE.replace("\"notes.*\"", "\"notes.(read|write)\"");
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::BadAllowedTool { .. }));
    }

    #[test]
    fn rejects_duplicate_persona_id() {
        let bad = format!(
            "{SAMPLE}\n[[personas]]\nid = \"notes\"\nprompt_file = \"p.md\"\n\
             allowed_tools = []\ndefault_model_family = \"smart\"\n\
             default_attachments_policy = \"inline-thread-scoped\"\n"
        );
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::DuplicatePersona(_)));
    }

    #[test]
    fn parses_runtimes_block_with_capabilities() {
        let extended = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"wasm\"\nartefact = \"wasm/notes.wasm\"\n\
             [runtimes.capabilities]\n\
             attachments = [\"read\", \"write\"]\n\
             fs = [\"/etc/codeless/\"]\n\
             http = []\n\
             wall_clock = false\n",
        );
        let m = PluginManifest::from_str(&extended, None).expect("valid manifest");
        assert_eq!(m.runtimes.len(), 1);
        let r = &m.runtimes[0];
        assert_eq!(r.kind, PluginRuntimeKind::Wasm);
        assert_eq!(
            r.artefact.as_deref(),
            Some(std::path::Path::new("wasm/notes.wasm"))
        );
        assert_eq!(r.capabilities.attachments, vec!["read", "write"]);
        assert_eq!(r.capabilities.fs, vec!["/etc/codeless/".to_string()]);
        assert!(!r.capabilities.wall_clock);
    }

    #[test]
    fn defaults_runtimes_block_to_empty_default_deny() {
        // A manifest without `[[runtimes]]` parses cleanly -- the
        // pre-substrate-runtimes notes plugin shipped that shape and
        // we keep it loadable. Default-deny is "no runtimes
        // declared", which the registry treats as builtin in stage
        // 13's hookup.
        let m = PluginManifest::from_str(SAMPLE, None).expect("valid manifest");
        assert!(m.runtimes.is_empty());
    }

    #[test]
    fn rejects_unknown_capability_field() {
        // OQ-WASM-5: the plugin manifest cannot enlarge its sandbox.
        // `fuel` belongs on the codeless config, not the manifest,
        // so a `[runtimes.capabilities] fuel = ...` entry trips
        // `deny_unknown_fields` here.
        let bad = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"wasm\"\nartefact = \"x.wasm\"\n\
             [runtimes.capabilities]\nfuel = 1\n",
        );
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::Parse(_)));
    }

    #[test]
    fn rejects_bad_attachments_scope() {
        let bad = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"wasm\"\nartefact = \"x.wasm\"\n\
             [runtimes.capabilities]\nattachments = [\"admin\"]\n",
        );
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::BadAttachmentsScope { .. }));
    }

    #[test]
    fn rejects_duplicate_runtime_kind() {
        let bad = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"wasm\"\nartefact = \"a.wasm\"\n\
             [[runtimes]]\nkind = \"wasm\"\nartefact = \"b.wasm\"\n",
        );
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::DuplicateRuntimeKind(PluginRuntimeKind::Wasm),
        ));
    }

    #[test]
    fn process_kind_parses_for_future_seam() {
        // PLUGIN-PROCESS.md item 11 is design-only in this job; the
        // manifest still parses so a future plugin can declare it.
        // The registry refuses to wire it up until item 11 ships.
        let extended =
            format!("{SAMPLE}\n[[runtimes]]\nkind = \"process\"\nbinary = \"bin/notes\"\n",);
        let m = PluginManifest::from_str(&extended, None).expect("valid manifest");
        assert_eq!(m.runtimes[0].kind, PluginRuntimeKind::Process);
        assert_eq!(
            m.runtimes[0].binary.as_deref(),
            Some(std::path::Path::new("bin/notes"))
        );
    }

    #[test]
    fn process_kind_accepts_policy_block_with_durations_and_threshold() {
        // PLUGIN-PROCESS.md § Manifest: the policy knobs are
        // optional individually; the parser validates duration
        // strings up-front so a misspelt `"5seconds"` fails at
        // manifest-load, not at the first supervisor tick.
        let extended = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"process\"\nbinary = \"bin/notes\"\n\
             [runtimes.policy]\n\
             socket_ready_timeout = \"5s\"\n\
             health_interval      = \"10s\"\n\
             failure_threshold    = 3\n\
             failure_window       = \"60s\"\n\
             failed_cooldown      = false\n",
        );
        let m = PluginManifest::from_str(&extended, None).expect("valid manifest");
        let resolved = m.runtimes[0].policy.resolve();
        assert_eq!(resolved.socket_ready_timeout, Some(Duration::from_secs(5)));
        assert_eq!(resolved.health_interval, Some(Duration::from_secs(10)));
        assert_eq!(resolved.failure_threshold, Some(3));
        assert_eq!(resolved.failure_window, Some(Duration::from_secs(60)));
        assert_eq!(resolved.failed_cooldown, None);
    }

    #[test]
    fn process_kind_policy_failed_cooldown_accepts_duration_string() {
        let extended = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"process\"\nbinary = \"bin/notes\"\n\
             [runtimes.policy]\nfailed_cooldown = \"60s\"\n",
        );
        let m = PluginManifest::from_str(&extended, None).expect("valid manifest");
        assert_eq!(
            m.runtimes[0].policy.resolve().failed_cooldown,
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn process_kind_rejects_failed_cooldown_true() {
        let extended = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"process\"\nbinary = \"bin/notes\"\n\
             [runtimes.policy]\nfailed_cooldown = true\n",
        );
        let err = PluginManifest::from_str(&extended, None).unwrap_err();
        assert!(matches!(err, ManifestError::BadFailedCooldownLiteral));
    }

    #[test]
    fn process_kind_rejects_bad_duration_string() {
        let extended = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"process\"\nbinary = \"bin/notes\"\n\
             [runtimes.policy]\nhealth_interval = \"5seconds\"\n",
        );
        let err = PluginManifest::from_str(&extended, None).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::BadRuntimePolicyDuration { field, .. } if field == "health_interval"
        ));
    }

    #[test]
    fn process_kind_rejects_missing_binary() {
        let extended = format!("{SAMPLE}\n[[runtimes]]\nkind = \"process\"\n");
        let err = PluginManifest::from_str(&extended, None).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::RuntimeFieldMissing {
                kind: PluginRuntimeKind::Process,
                field: "binary",
            },
        ));
    }

    #[test]
    fn builtin_kind_rejects_binary_field() {
        // Kind-specific block validation: a `binary` field on a
        // builtin entry would silently be ignored by a permissive
        // parser; the substrate-doc rule is to fail so the operator
        // sees the mistake at boot rather than at first call.
        let extended = format!(
            "{SAMPLE}\n[[runtimes]]\nkind = \"builtin\"\ncrate = \"x\"\nbinary = \"bin/y\"\n",
        );
        let err = PluginManifest::from_str(&extended, None).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::RuntimeFieldNotApplicable {
                kind: PluginRuntimeKind::Builtin,
                field: "binary",
            },
        ));
    }

    #[test]
    fn wasm_kind_rejects_policy_block() {
        // `[runtimes.policy]` applies to process only; declaring
        // it on a wasm runtime means the manifest pretends to
        // configure a supervisor that does not exist for that
        // flavour.
        let extended = format!(
            "{SAMPLE}\n\
             [[runtimes]]\nkind = \"wasm\"\nartefact = \"x.wasm\"\n\
             [runtimes.policy]\nhealth_interval = \"10s\"\n",
        );
        let err = PluginManifest::from_str(&extended, None).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::RuntimePolicyNotApplicable(PluginRuntimeKind::Wasm),
        ));
    }

    #[test]
    fn builtin_kind_requires_crate_field() {
        let extended = format!("{SAMPLE}\n[[runtimes]]\nkind = \"builtin\"\n");
        let err = PluginManifest::from_str(&extended, None).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::RuntimeFieldMissing {
                kind: PluginRuntimeKind::Builtin,
                field: "crate",
            },
        ));
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let bad = format!("{SAMPLE}\n[ohno]\nx = 1\n");
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::Parse(_)));
    }
}
