//! End-to-end check on the binary's CLI contract. The runtime will
//! pipe `git diff --name-only` into the binary's stdin and key its
//! REVIEW verdict off the exit code; the contract is "exit 0 clean,
//! exit 1 violations, exit 2 tooling". This test pins all three.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_codeless-predicates")
}

fn run_with(worktree: &Path, paths: &str) -> (i32, String, String) {
    let mut child = Command::new(bin_path())
        .arg("--worktree")
        .arg(worktree)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn binary");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(paths.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn exits_zero_for_clean_diff() {
    let dir = tempfile::tempdir().unwrap();
    let rel = "crates/codeless-runtime/src/clean.rs";
    let abs = dir.path().join(rel);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(&abs, "fn main() {}\n").unwrap();

    let (code, stdout, _) = run_with(dir.path(), &format!("{rel}\n"));
    assert_eq!(code, 0, "stdout was: {stdout}");
    assert!(stdout.is_empty());
}

#[test]
fn exits_one_for_violating_diff() {
    let dir = tempfile::tempdir().unwrap();
    let rel = "crates/codeless-runtime/src/bad.rs";
    let abs = dir.path().join(rel);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(&abs, "use tokio::process::Command;\n").unwrap();

    let (code, stdout, _) = run_with(dir.path(), &format!("{rel}\n"));
    assert_eq!(code, 1, "stdout was: {stdout}");
    assert!(stdout.contains("no-process-spawn-outside-adapters-host"));
}

#[test]
fn exits_two_for_missing_worktree_arg() {
    let mut child = Command::new(bin_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.as_mut().unwrap().write_all(b"").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn skips_paths_that_no_longer_exist() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout, _) = run_with(dir.path(), "deleted/file.rs\n");
    assert_eq!(code, 0, "stdout was: {stdout}");
}
