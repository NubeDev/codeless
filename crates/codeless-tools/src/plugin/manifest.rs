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
    /// Absolute path of the directory the manifest was loaded from.
    /// `None` when parsed from an in-memory string (test path).
    pub root: Option<PathBuf>,
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

        Ok(Self {
            plugin: parsed.plugin,
            personas: parsed.personas,
            migrations: parsed.migrations.unwrap_or_default(),
            data: parsed.data.unwrap_or_default(),
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
    fn rejects_unknown_top_level_field() {
        let bad = format!("{SAMPLE}\n[ohno]\nx = 1\n");
        let err = PluginManifest::from_str(&bad, None).unwrap_err();
        assert!(matches!(err, ManifestError::Parse(_)));
    }
}
