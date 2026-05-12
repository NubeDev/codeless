//! `SecretStore` exercises:
//! - `open` against a missing path returns an empty store and does
//!   not touch the filesystem.
//! - `save` writes 0600 on Unix, round-trips through a fresh `open`.
//! - `set` rejects empty / whitespace keys.
//! - `remove` on an unknown key reports `UnknownKey`.
//! - `list` returns names only (the test inspects via `get` to
//!   confirm values still round-trip).

use codeless_adapters_host::secrets::{SecretError, SecretStore};
use tempfile::TempDir;

fn tmp_path(dir: &TempDir, name: &str) -> std::path::PathBuf {
    dir.path().join(name)
}

#[test]
fn open_missing_file_yields_empty_store_without_writing() {
    let dir = TempDir::new().unwrap();
    let path = tmp_path(&dir, "secrets.toml");
    let store = SecretStore::open(&path).expect("open");
    assert!(store.list().is_empty());
    assert!(!path.exists());
}

#[test]
fn save_round_trips_values() {
    let dir = TempDir::new().unwrap();
    let path = tmp_path(&dir, "secrets.toml");
    let mut store = SecretStore::open(&path).expect("open");
    store.set("ANTHROPIC_API_KEY", "sk-test").unwrap();
    store.set("github_token", "ghp_x").unwrap();
    store.save().unwrap();

    let reopened = SecretStore::open(&path).expect("reopen");
    let mut names = reopened.list();
    names.sort();
    assert_eq!(names, vec!["ANTHROPIC_API_KEY", "github_token"]);
    assert_eq!(reopened.get("ANTHROPIC_API_KEY"), Some("sk-test"));
    assert_eq!(reopened.get("github_token"), Some("ghp_x"));
}

#[cfg(unix)]
#[test]
fn save_sets_0600_permissions_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = tmp_path(&dir, "secrets.toml");
    let mut store = SecretStore::open(&path).expect("open");
    store.set("key", "value").unwrap();
    store.save().unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "expected 0600, got {mode:o}");
}

#[test]
fn set_rejects_invalid_keys() {
    let dir = TempDir::new().unwrap();
    let mut store = SecretStore::open(tmp_path(&dir, "secrets.toml")).unwrap();
    assert!(matches!(
        store.set("", "v"),
        Err(SecretError::InvalidKey { .. })
    ));
    assert!(matches!(
        store.set("has space", "v"),
        Err(SecretError::InvalidKey { .. })
    ));
    assert!(matches!(
        store.set("with=equals", "v"),
        Err(SecretError::InvalidKey { .. })
    ));
}

#[test]
fn remove_unknown_key_reports_unknown() {
    let dir = TempDir::new().unwrap();
    let mut store = SecretStore::open(tmp_path(&dir, "secrets.toml")).unwrap();
    match store.remove("nope") {
        Err(SecretError::UnknownKey(k)) => assert_eq!(k, "nope"),
        other => panic!("expected UnknownKey, got {other:?}"),
    }
}
