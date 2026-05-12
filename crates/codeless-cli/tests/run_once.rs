//! End-to-end `codeless run --once` exercise. Spins up a tempdir,
//! `git init`s it so the `--repo` argument is a real directory, and
//! checks the streamed JSON-line output for the expected framing
//! events. Exercises both the mock runner and the `ClaudeRunnerAdapter`
//! via the workspace-standard fake-claude binary on an explicit
//! `CLAUDE_BINARY` (per SCOPE.md "Testing strategy" — never the
//! developer's host install).

use std::os::unix::fs::PermissionsExt;
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

const FAKE_CLAUDE: &str = r#"#!/usr/bin/env bash
cat <<'JSON'
{"type":"system","subtype":"init","session_id":"sess-fake","model":"claude-opus-4-5"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello "}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"world"}]}}
{"type":"result","subtype":"success","total_cost_usd":0.0123,"session_id":"sess-fake"}
JSON
"#;

fn install_fake_claude(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("fake-claude");
    std::fs::write(&path, FAKE_CLAUDE).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
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
fn run_with_claude_runner_streams_ai_events() {
    let repo = TempDir::new().unwrap();
    git_init(repo.path());
    let bin_dir = TempDir::new().unwrap();
    let fake = install_fake_claude(bin_dir.path());

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .env("CLAUDE_BINARY", &fake)
        .args([
            "run",
            "--repo",
            repo.path().to_str().unwrap(),
            "--runner",
            "claude",
            "hi",
        ])
        .assert()
        .success()
        .stdout(contains("\"type\":\"job-started\""))
        .stdout(contains("\"type\":\"ai-token\""))
        .stdout(contains("\"type\":\"ai-message-complete\""))
        .stdout(contains("\"type\":\"job-completed\""));
}

#[test]
fn run_rejects_invalid_runner_value() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args([
            "run",
            "--repo",
            dir.path().to_str().unwrap(),
            "--runner",
            "bogus",
            "hello",
        ])
        .assert()
        .failure()
        .stderr(contains("invalid value 'bogus'"));
}

#[test]
fn run_anthropic_requires_api_key() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .env_remove("ANTHROPIC_API_KEY")
        .args([
            "run",
            "--repo",
            dir.path().to_str().unwrap(),
            "--runner",
            "anthropic",
            "hello",
        ])
        .assert()
        .failure()
        .stderr(contains("ANTHROPIC_API_KEY"));
}

#[test]
fn run_rejects_missing_repo() {
    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["run", "--repo", "/nonexistent/path/xyz", "hello"])
        .assert()
        .failure();
}
