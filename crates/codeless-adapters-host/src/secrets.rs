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
/// The store is a thin facade over a `SecretBackend`: it owns an
/// in-memory `BTreeMap<String, String>` view (stable order in `list`,
/// deterministic disk output) that is loaded from the backend on
/// open and flushed back through `save`. Two backends ship: the
/// XDG TOML file (the default) and the OS keychain via the `keyring`
/// crate (behind the `keyring` Cargo feature, off by default).
/// Callers do not care which is in use — the boot path picks one at
/// construction time and hands the store down.
pub struct SecretStore {
    backend: Box<dyn SecretBackend>,
    entries: BTreeMap<String, String>,
}

/// Persistence seam under `SecretStore`. Implementations carry their
/// own addressing (path for TOML, service name for keyring) and
/// translate the whole-map snapshot in either direction. Save is a
/// full overwrite — there is no incremental key-by-key delta because
/// `SecretStore` never exposes one; the in-memory map is the truth
/// the caller is working against, and the backend is just where it
/// lives between processes.
pub trait SecretBackend: Send + Sync {
    /// Read every key the backend knows about. A missing TOML file
    /// or empty keyring returns `Ok(empty)`, never an error.
    fn load(&self) -> Result<BTreeMap<String, String>, SecretError>;

    /// Persist `entries` durably. Must be write-then-fsync — if the
    /// process dies after `save` returns, a fresh `load` must see
    /// the new state. The adapter-registry restart path relies on
    /// this: secrets-write completes, fsync settles, then (and only
    /// then) the UI fires the restart signal.
    fn save(&self, entries: &BTreeMap<String, String>) -> Result<(), SecretError>;

    /// Identifier for diagnostics. Path for TOML, `keyring:<service>`
    /// for keyring. Never written to user-facing surfaces; only logs.
    fn location(&self) -> String;
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
    #[error("backend: {0}")]
    Backend(String),
}

impl SecretStore {
    /// Default constructor: a TOML-file backend at `path`. Missing
    /// files yield an empty store; the file is materialised on the
    /// first `save`. Parent directories are created lazily by `save`
    /// rather than here so a read-only `open` does not write to disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        Self::with_backend(Box::new(TomlSecretBackend::new(
            path.as_ref().to_path_buf(),
        )))
    }

    /// Wrap an arbitrary backend. The boot path uses this when the
    /// operator opts into the keychain (or any future backend); the
    /// CLI / fixtures keep using `open` and the TOML default.
    pub fn with_backend(backend: Box<dyn SecretBackend>) -> Result<Self, SecretError> {
        let entries = backend.load()?;
        Ok(Self { backend, entries })
    }

    /// OS keychain backend (Secret Service on Linux, Keychain on
    /// macOS, Credential Manager on Windows). `service` is the
    /// keychain service name under which every key lives; pick one
    /// stable string per installation so a later `load` finds the
    /// same entries. Only available with the `keyring` Cargo
    /// feature; CI / headless installs stick with the TOML default.
    #[cfg(feature = "keyring")]
    pub fn open_keyring(service: impl Into<String>) -> Result<Self, SecretError> {
        Self::with_backend(Box::new(KeyringSecretBackend::new(service.into())))
    }

    /// Where the live data is stored. Path for the TOML backend;
    /// `keyring:<service>` for the keyring backend. Used by the CLI
    /// `codeless secrets` subcommand for the "writing to…" header
    /// and by tests that need to introspect the on-disk file.
    pub fn location(&self) -> String {
        self.backend.location()
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

    /// Flush the in-memory map through the backend. Backends must
    /// fsync before returning; the adapter-registry restart path
    /// relies on a successful return meaning the bytes are durable
    /// on disk before the UI fires `restart_server`.
    pub fn save(&self) -> Result<(), SecretError> {
        self.backend.save(&self.entries)
    }
}

/// TOML-file backend at an XDG-style path. Wire format is a single
/// TOML table of `key = "value"` pairs; round-trips through
/// `toml::to_string_pretty` without surprises (no nested tables).
/// 0600 on Unix on write — see [`write_private`].
pub struct TomlSecretBackend {
    path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default, flatten)]
    entries: BTreeMap<String, String>,
}

impl TomlSecretBackend {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SecretBackend for TomlSecretBackend {
    fn load(&self) -> Result<BTreeMap<String, String>, SecretError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let text = fs::read_to_string(&self.path)?;
        let parsed: OnDisk = toml::from_str(&text)?;
        Ok(parsed.entries)
    }

    /// Atomic write with 0600 permissions. Strategy: write to
    /// `<path>.tmp` next to the target, fsync the temp file, rename
    /// over the target. The temp file inherits 0600 from creation
    /// flags on Unix so the value is never world-readable
    /// mid-rename. Non-Unix targets skip the chmod and rely on
    /// filesystem ACLs — the personal-hosted MVP runs Unix.
    ///
    /// The fsync-before-rename is what makes
    /// write-then-fsync-then-restart safe: when this returns, the
    /// file contents are durable. The adapter-registry restart RPC
    /// fires only after this returns; a process crash between save
    /// and the restart signal still leaves the new secrets on disk
    /// for the next boot.
    fn save(&self, entries: &BTreeMap<String, String>) -> Result<(), SecretError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let text = toml::to_string_pretty(&OnDisk {
            entries: entries.clone(),
        })?;
        let tmp = with_tmp_suffix(&self.path);
        write_private(&tmp, text.as_bytes())?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn location(&self) -> String {
        self.path.display().to_string()
    }
}

#[cfg(feature = "keyring")]
mod keyring_backend {
    use super::{SecretBackend, SecretError};
    use std::collections::BTreeMap;

    /// Marker entry name used to enumerate the set of stored keys.
    /// The `keyring` crate has no `list()` — only get/set/delete on
    /// a known `(service, user)` pair — so we shadow the index in a
    /// dedicated entry and write it alongside the real values.
    const INDEX_USER: &str = "__codeless_index__";

    pub struct KeyringSecretBackend {
        service: String,
    }

    impl KeyringSecretBackend {
        pub fn new(service: String) -> Self {
            Self { service }
        }

        fn entry(&self, user: &str) -> Result<keyring::Entry, SecretError> {
            keyring::Entry::new(&self.service, user)
                .map_err(|e| SecretError::Backend(format!("keyring open: {e}")))
        }
    }

    impl SecretBackend for KeyringSecretBackend {
        fn load(&self) -> Result<BTreeMap<String, String>, SecretError> {
            let index_entry = self.entry(INDEX_USER)?;
            let index_blob = match index_entry.get_password() {
                Ok(s) => s,
                Err(keyring::Error::NoEntry) => return Ok(BTreeMap::new()),
                Err(e) => return Err(SecretError::Backend(format!("keyring read index: {e}"))),
            };
            let keys: Vec<String> = serde_json::from_str(&index_blob)
                .map_err(|e| SecretError::Backend(format!("index parse: {e}")))?;
            let mut out = BTreeMap::new();
            for k in keys {
                let entry = self.entry(&k)?;
                match entry.get_password() {
                    Ok(v) => {
                        out.insert(k, v);
                    }
                    Err(keyring::Error::NoEntry) => continue,
                    Err(e) => return Err(SecretError::Backend(format!("keyring read {k}: {e}"))),
                }
            }
            Ok(out)
        }

        fn save(&self, entries: &BTreeMap<String, String>) -> Result<(), SecretError> {
            // Delete keys that disappeared since the last load, so a
            // remove() actually leaves the keyring. Best-effort: a
            // missing entry is fine.
            let existing: Vec<String> = match self.entry(INDEX_USER)?.get_password() {
                Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            for old in &existing {
                if !entries.contains_key(old) {
                    let _ = self.entry(old)?.delete_credential();
                }
            }
            for (k, v) in entries {
                self.entry(k)?
                    .set_password(v)
                    .map_err(|e| SecretError::Backend(format!("keyring write {k}: {e}")))?;
            }
            let keys: Vec<&String> = entries.keys().collect();
            let index_blob = serde_json::to_string(&keys)
                .map_err(|e| SecretError::Backend(format!("index serialise: {e}")))?;
            self.entry(INDEX_USER)?
                .set_password(&index_blob)
                .map_err(|e| SecretError::Backend(format!("keyring write index: {e}")))?;
            Ok(())
        }

        fn location(&self) -> String {
            format!("keyring:{}", self.service)
        }
    }
}

#[cfg(feature = "keyring")]
pub use keyring_backend::KeyringSecretBackend;

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
