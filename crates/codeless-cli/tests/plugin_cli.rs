//! `codeless plugin {list,info}` smoke test. Writes a `notes`-shaped
//! plugin to a tempdir, points `--plugins-dir` at it, and checks the
//! CLI output. With plugin #0 (`notes`) now compiled into the host
//! binary (PS-NOTES), `list` registers the plugin's tool through the
//! static `RegistrationTable` and `info notes` reads back the loaded
//! registry. PS6 acceptance: read-only CLI surfaces are backed by the
//! registry, not the manifest.

use std::fs;

use assert_cmd::Command as TestCommand;
use predicates::str::contains;
use tempfile::TempDir;

fn write_notes(plugins_dir: &std::path::Path) {
    let root = plugins_dir.join("notes");
    fs::create_dir_all(root.join("prompts")).unwrap();
    fs::create_dir_all(root.join("migrations")).unwrap();
    fs::write(
        root.join("plugin.toml"),
        r#"
[plugin]
id = "notes"
version = "0.1.0"
crate = "codeless-plugin-notes"

[[personas]]
id = "notes"
prompt_file = "prompts/system.md"
allowed_tools = ["notes.*", "attachments.read"]
default_model_family = "smart"
default_attachments_policy = "inline-thread-scoped"
"#,
    )
    .unwrap();
    fs::write(root.join("prompts/system.md"), "notes prompt").unwrap();
    fs::write(
        root.join("migrations/0001.sql"),
        "CREATE TABLE notes_entries (id TEXT PRIMARY KEY);",
    )
    .unwrap();
}

#[test]
fn plugin_list_reports_discovered_dirs() {
    let tmp = TempDir::new().unwrap();
    write_notes(tmp.path());
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["plugin", "list", "--plugins-dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("ID"))
        .stdout(contains("notes"))
        // PS-NOTES: the static registration table now carries the
        // notes plugin, so `list` should report it as `loaded` once
        // the manifest parses against the registered id.
        .stdout(contains("loaded"))
        .stdout(contains("model families"))
        .stdout(contains("smart"));
}

#[test]
fn plugin_info_returns_registered_plugin() {
    // With plugin #0 compiled in, `info notes` reads back the loaded
    // registry rather than failing. Pin the persona id + tool
    // listing here so a substrate refactor that breaks the manifest
    // -> registry bridge surfaces in the CLI smoke first.
    let tmp = TempDir::new().unwrap();
    write_notes(tmp.path());
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["plugin", "info", "notes", "--plugins-dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("id:      notes"))
        .stdout(contains("notes.append"))
        .stdout(contains("notes:notes"));
}

#[test]
fn plugin_info_unknown_id_errors() {
    // The substrate-doc contract: `info` reads the registry, so an
    // id that does not resolve is a clean error rather than a panic.
    let tmp = TempDir::new().unwrap();
    write_notes(tmp.path());
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["plugin", "info", "does-not-exist", "--plugins-dir"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(contains("not loaded"));
}

#[test]
fn plugin_list_empty_dirs_reports_registered_but_missing() {
    // Empty plugins dir + compiled-in registration table: the host
    // has the notes registration entry but no plugin.toml to load.
    // The CLI should make the gap visible rather than silently
    // succeeding with no output.
    let tmp = TempDir::new().unwrap();
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["plugin", "list", "--plugins-dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("notes"))
        .stdout(contains("no plugin.toml"));
}
