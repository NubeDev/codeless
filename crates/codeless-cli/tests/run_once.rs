//! End-to-end `codeless run --once` exercise. Spins up a tempdir,
//! `git init`s it so the `--repo` argument is a real directory, and
//! checks the streamed JSON-line output for the expected framing
//! events. The mock runner is the only Phase 1 wiring; this test
//! pins that contract.

use std::path::Path;
use std::process::Command;

use assert_cmd::Command as TestCommand;
use predicates::str::contains;
use tempfile::TempDir;

fn git_init(dir: &Path) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["init", "--initial-branch=main", "."])
        .output()
        .expect("git binary");
    assert!(out.status.success(), "git init failed: {out:?}");
}

#[test]
fn run_once_emits_started_task_and_completed_events() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args([
            "run",
            "--repo",
            dir.path().to_str().unwrap(),
            "--runner",
            "mock",
            "hello",
        ])
        .assert()
        .success()
        .stdout(contains("\"type\":\"job-started\""))
        .stdout(contains("\"type\":\"task-started\""))
        .stdout(contains("\"type\":\"task-completed\""))
        .stdout(contains("\"type\":\"job-completed\""));
}

#[test]
fn run_rejects_unknown_runner() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args([
            "run",
            "--repo",
            dir.path().to_str().unwrap(),
            "--runner",
            "claude",
            "hello",
        ])
        .assert()
        .failure()
        .stderr(contains("not wired in Phase 1"));
}

#[test]
fn run_rejects_missing_repo() {
    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["run", "--repo", "/nonexistent/path/xyz", "hello"])
        .assert()
        .failure();
}
