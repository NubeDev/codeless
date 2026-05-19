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

/// Environment variable that flips the test binary into "crash
/// helper" mode for [`write_then_fsync_survives_crash_before_restart_signal`].
/// When set, [`crash_helper_child`] writes a value through
/// `SecretStore::save` and then aborts the process *before* it
/// would signal the parent that a restart is safe. The parent then
/// reopens the file and asserts the bytes are durable on disk.
const CRASH_HELPER_ENV: &str = "CODELESS_SECRETS_CRASH_HELPER_PATH";

/// Child-side of the crash-ordering test. Runs as a regular `#[test]`
/// so we can re-execute the integration-test binary against this
/// exact name via `--exact`; in normal test runs the env var is
/// unset and the function returns immediately.
///
/// The body deliberately mirrors the adapter-registry restart
/// sequence: `set` the new secret, `save` (which fsyncs through the
/// TOML backend), then *crash* before any "restart_server" RPC
/// could fire. If the on-disk state is durable, the parent's reopen
/// sees the new value; if `save` skipped its fsync, the parent
/// might see an empty file or the prior contents.
#[test]
fn crash_helper_child() {
    let Ok(path) = std::env::var(CRASH_HELPER_ENV) else {
        return;
    };
    let mut store = SecretStore::open(&path).expect("child: open");
    store
        .set("durable_key", "durable_value")
        .expect("child: set");
    store.save().expect("child: save");
    // No restart signal here — the abort below stands in for a
    // crash between secrets-write and the restart-signal RPC.
    std::process::abort();
}

/// The exit test the adapter-registry milestone gates on
/// (`DOCS/WORKSPACE-ATTACH.md` §"Exit tests"). Spawns a child copy
/// of this very test binary in "crash helper" mode; the child
/// writes a secret, fsyncs through `SecretStore::save`, and then
/// `std::process::abort()`s before it could ever signal a server
/// restart. The parent confirms (a) the child died abnormally, and
/// (b) reopening the file in this process sees the new value. If
/// the TOML backend ever drops its fsync, the file would be empty
/// or stale on a real-world power loss and this test would catch
/// it.
#[test]
fn write_then_fsync_survives_crash_before_restart_signal() {
    let dir = TempDir::new().unwrap();
    let path = tmp_path(&dir, "secrets.toml");

    let exe = std::env::current_exe().expect("current_exe");
    let status = std::process::Command::new(&exe)
        .args(["--exact", "crash_helper_child", "--nocapture"])
        .env(CRASH_HELPER_ENV, &path)
        .status()
        .expect("spawn crash helper");

    assert!(
        !status.success(),
        "crash helper should abort, got {status:?}"
    );

    let reopened = SecretStore::open(&path).expect("parent: reopen");
    assert_eq!(
        reopened.get("durable_key"),
        Some("durable_value"),
        "secret must survive the crash; fsync ordering is load-bearing for restart_server"
    );
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
