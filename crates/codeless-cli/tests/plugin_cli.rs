//! `codeless plugin {list,info}` smoke test. Writes a `notes`-shaped
//! plugin to a tempdir, points `--plugins-dir` at it, and checks the
//! CLI output. The static registration table the binary ships with is
//! empty in this commit (plugin #0 lands in a follow-up stage), so
//! `list` should report the plugin as discoverable-but-not-compiled-in
//! rather than registering its tools. PS6 acceptance: read-only CLI
//! surfaces are backed by the registry, not the manifest.

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
        // Manifest parses + the static-registration table is empty so
        // load_plugin emits a warning on stderr. The plugin id should
        // still surface in the table (the warning is to stderr).
        .stdout(contains("ID"))
        .stdout(contains("notes"))
        .stdout(contains("model families"))
        .stdout(contains("smart"));
}

#[test]
fn plugin_info_reads_registry_after_failed_load() {
    // Without a registration entry, `info <id>` should error cleanly
    // rather than panic. This pins the contract that the substrate-doc
    // calls out: `info` reads the registry, not the manifest.
    let tmp = TempDir::new().unwrap();
    write_notes(tmp.path());
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["plugin", "info", "notes", "--plugins-dir"])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(contains("not loaded"));
}

#[test]
fn plugin_list_empty_dirs_and_empty_table() {
    let tmp = TempDir::new().unwrap();
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["plugin", "list", "--plugins-dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("no plugins compiled in"));
}
