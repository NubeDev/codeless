//! Codeless-side model family aliases — the indirection between a
//! plugin's `default_model_family = "smart"` and the provider model id
//! the runner actually calls.
//!
//! The substrate doc (item 6) pins the rule: a plugin manifest declares
//! one of a small set of family aliases, never a provider model id.
//! The mapping is *codeless config*, not plugin data — when Anthropic
//! ships a new "smart" tier, the operator updates a single TOML file
//! and every plugin moves with it.
//!
//! Built-in aliases are `fast`, `smart`, `reasoning`. They are listed
//! by name in the substrate doc and round-trip through the manifest
//! reader unmodified. The operator can override the concrete provider
//! model for any alias via the config file the codeless binary reads
//! at startup; missing aliases fall back to the embedded defaults so a
//! fresh install resolves without an extra config step.
//!
//! This module is intentionally narrow: it does not know about
//! provider catalogues, capability profiles, or cost tiers. The runner
//! consumes a `provider_model` string and decides what to do with it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The known family aliases. Single source of truth for both the
/// manifest validator and the resolver — adding a new alias here is the
/// only place a plugin's `default_model_family` value can be extended.
pub const KNOWN_FAMILIES: &[&str] = &["fast", "smart", "reasoning"];

/// Is `family` one of the codeless-side aliases? Used by the manifest
/// reader (item 6) to reject hardcoded provider model ids at load time.
pub fn is_known_family_alias(family: &str) -> bool {
    KNOWN_FAMILIES.contains(&family)
}

/// Embedded defaults so a fresh codeless install resolves every alias
/// without a config file. Values point at Anthropic's current tiers as
/// of 2026-05; the operator overrides any of them via the config file
/// the binary loads at startup.
fn default_family_map() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("fast".into(), "claude-haiku-4-5".into());
    m.insert("smart".into(), "claude-sonnet-4-5".into());
    m.insert("reasoning".into(), "claude-opus-4-7".into());
    m
}

/// Resolves family aliases to provider model ids. Built from the
/// embedded defaults overlaid with operator-provided overrides.
///
/// `ModelFamilyConfig` is constructed at server boot from a TOML file
/// (`[model_families]` table — `<alias> = "<provider-model-id>"`) and
/// from then on the runner asks `resolve(alias) -> &str` to compose a
/// turn for any persona.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFamilyConfig {
    map: BTreeMap<String, String>,
}

impl Default for ModelFamilyConfig {
    fn default() -> Self {
        Self {
            map: default_family_map(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    #[serde(default)]
    model_families: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelFamilyError {
    #[error("read model family config at {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parse model family config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error(
        "model family config: unknown alias `{0}` (known aliases: fast, smart, reasoning); \
         add it to KNOWN_FAMILIES before referencing it from config"
    )]
    UnknownAlias(String),
    #[error("model family config: alias `{0}` maps to an empty model id")]
    EmptyModel(String),
}

impl ModelFamilyConfig {
    /// Built-in defaults with no operator overrides. Useful for tests
    /// and for the in-memory CLI invocations that don't have a config
    /// path on hand.
    pub fn builtin() -> Self {
        Self::default()
    }

    /// Build from a TOML string with the on-disk shape. Overrides
    /// applied on top of the embedded defaults so partial config files
    /// behave the way the operator expects (set one alias, leave the
    /// rest alone).
    pub fn from_toml(text: &str) -> Result<Self, ModelFamilyError> {
        let parsed: OnDisk = toml::from_str(text)?;
        let mut map = default_family_map();
        for (alias, model) in parsed.model_families {
            if !is_known_family_alias(&alias) {
                return Err(ModelFamilyError::UnknownAlias(alias));
            }
            if model.trim().is_empty() {
                return Err(ModelFamilyError::EmptyModel(alias));
            }
            map.insert(alias, model);
        }
        Ok(Self { map })
    }

    /// Build from a file path. Missing file yields the embedded
    /// defaults — config is opt-in so the happy path stays one-step.
    pub fn from_file(path: &Path) -> Result<Self, ModelFamilyError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).map_err(|source| ModelFamilyError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml(&text)
    }

    /// Resolve a family alias to a provider model id. Returns `None`
    /// for an unknown alias; callers that have validated via
    /// `is_known_family_alias` and seeded defaults can `.unwrap()`,
    /// but the surface is fallible because operator config can in
    /// principle drop an alias.
    pub fn resolve(&self, family: &str) -> Option<&str> {
        self.map.get(family).map(String::as_str)
    }

    /// Iterate over all alias → model pairs. `codeless plugin info`
    /// uses this to render the active mapping in the operator's
    /// terminal so they can spot a missing override.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_aliases_round_trip() {
        for family in KNOWN_FAMILIES {
            assert!(is_known_family_alias(family));
        }
        assert!(!is_known_family_alias("magic"));
        assert!(!is_known_family_alias(""));
    }

    #[test]
    fn defaults_cover_every_known_family() {
        let cfg = ModelFamilyConfig::builtin();
        for family in KNOWN_FAMILIES {
            let resolved = cfg
                .resolve(family)
                .unwrap_or_else(|| panic!("alias {family} unresolved"));
            assert!(!resolved.is_empty());
        }
    }

    #[test]
    fn overrides_layer_on_defaults() {
        let cfg = ModelFamilyConfig::from_toml(
            r#"
[model_families]
smart = "claude-sonnet-4-7-preview"
"#,
        )
        .unwrap();
        assert_eq!(cfg.resolve("smart"), Some("claude-sonnet-4-7-preview"));
        // Other aliases keep the embedded default.
        assert!(cfg.resolve("fast").is_some());
        assert!(cfg.resolve("reasoning").is_some());
    }

    #[test]
    fn unknown_alias_in_config_rejected() {
        let err = ModelFamilyConfig::from_toml(
            r#"
[model_families]
magic = "x"
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ModelFamilyError::UnknownAlias(_)));
    }

    #[test]
    fn empty_model_rejected() {
        let err = ModelFamilyConfig::from_toml(
            r#"
[model_families]
smart = ""
"#,
        )
        .unwrap_err();
        assert!(matches!(err, ModelFamilyError::EmptyModel(_)));
    }
}
