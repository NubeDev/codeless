//! End-to-end CLI exercise of `codeless secrets`. Each test points
//! `--secrets-file` at a tempdir so `$HOME` resolution never races
//! with developer state.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

fn cmd(secrets_file: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("codeless").expect("codeless binary");
    c.arg("--secrets-file").arg(secrets_file);
    c
}

#[test]
fn set_then_list_then_get_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("secrets.toml");

    cmd(&path)
        .args(["secrets", "set", "ANTHROPIC_API_KEY", "sk-test"])
        .assert()
        .success();

    cmd(&path)
        .args(["secrets", "list"])
        .assert()
        .success()
        .stdout(contains("ANTHROPIC_API_KEY"));

    cmd(&path)
        .args(["secrets", "get", "ANTHROPIC_API_KEY", "--reveal"])
        .assert()
        .success()
        .stdout(contains("sk-test"));
}

#[test]
fn get_without_reveal_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("secrets.toml");
    cmd(&path)
        .args(["secrets", "set", "k", "v"])
        .assert()
        .success();
    cmd(&path)
        .args(["secrets", "get", "k"])
        .assert()
        .failure()
        .stderr(contains("--reveal"));
}

#[test]
fn rm_removes_key() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("secrets.toml");
    cmd(&path)
        .args(["secrets", "set", "k", "v"])
        .assert()
        .success();
    cmd(&path).args(["secrets", "rm", "k"]).assert().success();
    cmd(&path)
        .args(["secrets", "get", "k", "--reveal"])
        .assert()
        .failure()
        .stderr(contains("no such secret"));
}

#[test]
fn rm_unknown_reports_unknown() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("secrets.toml");
    cmd(&path)
        .args(["secrets", "rm", "absent"])
        .assert()
        .failure()
        .stderr(contains("unknown key"));
}
