use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// On-disk store for provider keys and the bearer token, modelled on
/// SCOPE.md "Secrets store (Phase 1)". The default path is the
/// XDG-respecting `~/.config/codeless/secrets.toml`; tests and
/// alternative installs point at any writable path.
///
/// Wire format is a single TOML table of `key = "value"` pairs. The
/// inner map is a `BTreeMap` (stable order in `list`, deterministic
/// disk output) and round-trips through `toml::to_string_pretty`
/// without surprises — there are no nested tables to worry about.
pub struct SecretStore {
    path: PathBuf,
    entries: BTreeMap<String, String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default, flatten)]
    entries: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("toml parse: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("toml serialise: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("unknown key: {0}")]
    UnknownKey(String),
    #[error("invalid key: {reason}")]
    InvalidKey { reason: &'static str },
}

impl SecretStore {
    /// Load or create the secrets file at `path`. Missing files yield
    /// an empty store; the file is materialised on the first `save`.
    /// Parent directories are created lazily by `save` rather than
    /// here so a read-only `open` does not write to disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        let path = path.as_ref().to_path_buf();
        let entries = if path.exists() {
            let text = fs::read_to_string(&path)?;
            let parsed: OnDisk = toml::from_str(&text)?;
            parsed.entries
        } else {
            BTreeMap::new()
        };
        Ok(Self { path, entries })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Names only — never the values. The frontend lists configured
    /// secrets through this surface; values are reachable only via
    /// `get` and never serialised over the RPC wire.
    pub fn list(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), SecretError> {
        let key = key.into();
        validate_key(&key)?;
        self.entries.insert(key, value.into());
        Ok(())
    }

    /// Returns `Err(UnknownKey)` when the key is absent — callers can
    /// distinguish "removed" from "never present" without re-reading.
    pub fn remove(&mut self, key: &str) -> Result<(), SecretError> {
        if self.entries.remove(key).is_none() {
            return Err(SecretError::UnknownKey(key.to_string()));
        }
        Ok(())
    }

    /// Atomic write with 0600 permissions. Strategy: write to
    /// `<path>.tmp` next to the target, fsync, rename. The temp file
    /// inherits 0600 from creation flags on Unix so the value is
    /// never world-readable mid-rename. Non-Unix targets skip the
    /// chmod and rely on filesystem ACLs — the personal-hosted MVP
    /// runs Unix; documented limitation otherwise.
    pub fn save(&self) -> Result<(), SecretError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let text = toml::to_string_pretty(&OnDisk {
            entries: self.entries.clone(),
        })?;
        let tmp = with_tmp_suffix(&self.path);
        write_private(&tmp, text.as_bytes())?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn with_tmp_suffix(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

fn validate_key(key: &str) -> Result<(), SecretError> {
    if key.is_empty() {
        return Err(SecretError::InvalidKey {
            reason: "must not be empty",
        });
    }
    if key
        .chars()
        .any(|c| c.is_whitespace() || c == '=' || c == '\n')
    {
        return Err(SecretError::InvalidKey {
            reason: "must not contain whitespace or '='",
        });
    }
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
