//! End-to-end exercise of `codeless patches {list,show,approve,reject}`.
//! Step 6 of the SESSION-MUTABLE-SCOPE ramp. Builds a temp git repo
//! with a hand-crafted `DOCS/SCOPE-PROPOSED.md`, runs each subcommand
//! as the real CLI binary against the temp repo, and asserts both
//! stdout output and the resulting git log / working-tree state.
//!
//! `edit` is not exercised here: it would require driving an
//! interactive `$EDITOR`. The `shell_split` helper it relies on has
//! coverage in `codeless-adapters-host::editor::tests`, and the
//! queue-rewrite path is exercised through `approve` and `reject`.

use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command as TestCommand;
use predicates::str::contains;
use tempfile::TempDir;

fn init_repo(dir: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "test@example.com"][..],
        &["config", "user.name", "test"][..],
        &["commit", "--allow-empty", "-q", "-m", "root"][..],
    ] {
        let out = StdCommand::new("git")
            .arg("-C")
            .arg(dir)
            .args(args.iter().copied())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {:?}", out);
    }
}

const PATCH_ID: &str = "01HV3F8N8C0KHX0M7CKJK3K9XX";
const SAMPLE_PROPOSED: &str = "\
# Proposed scope patches

Queue of REVIEW-emitted patches.

## 01HV3F8N8C0KHX0M7CKJK3K9XX

- kind: tighten
- target: claude-md
- target-path: codeless/CLAUDE.md
- has_predicate: true
- predicate-ref: no-emojis-in-source

### Rationale

R4 should explicitly auto-FAIL stages that edit files outside Done.

### Body

append the sentence to R4
";

fn seed_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let codeless_dir = tmp.path().join("codeless");
    std::fs::create_dir_all(&codeless_dir).unwrap();
    std::fs::write(codeless_dir.join("CLAUDE.md"), "original rule R4 stub\n").unwrap();
    let docs = tmp.path().join("DOCS");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("SCOPE-PROPOSED.md"), SAMPLE_PROPOSED).unwrap();
    // Commit the seed so HEAD is clean, then leave the human edit
    // un-committed for the approve flow.
    let out = StdCommand::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["add", "."])
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = StdCommand::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["commit", "-q", "-m", "seed"])
        .output()
        .unwrap();
    assert!(out.status.success());
    tmp
}

#[test]
fn list_prints_one_line_per_patch() {
    let tmp = seed_repo();
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "list", "--repo"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains(PATCH_ID))
        .stdout(contains("tighten"))
        .stdout(contains("codeless/CLAUDE.md"));
}

#[test]
fn list_against_missing_proposals_file_is_success() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "list", "--repo"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(contains("no proposed patches"));
}

#[test]
fn show_prints_block_for_known_id() {
    let tmp = seed_repo();
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "show", "--repo"])
        .arg(tmp.path())
        .arg(PATCH_ID)
        .assert()
        .success()
        .stdout(contains("kind: tighten"))
        .stdout(contains("predicate-ref: no-emojis-in-source"))
        .stdout(contains("R4 should explicitly auto-FAIL"));
}

#[test]
fn show_unknown_id_errors() {
    let tmp = seed_repo();
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "show", "--repo"])
        .arg(tmp.path())
        .arg("01HV3F8N8C0KHX0M7CKJK3K9YY")
        .assert()
        .failure()
        .stderr(contains("no proposed patch"));
}

#[test]
fn approve_removes_entry_and_commits_with_evidence_metadata() {
    let tmp = seed_repo();
    // Simulate the human's rulebook edit.
    let target = tmp.path().join("codeless/CLAUDE.md");
    std::fs::write(
        &target,
        "R4 must auto-FAIL stages editing files outside Done\n",
    )
    .unwrap();

    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "approve", "--repo"])
        .arg(tmp.path())
        .arg(PATCH_ID)
        .assert()
        .success()
        .stdout(contains(format!("approved {PATCH_ID}")));

    // Queue entry is gone.
    let queue = std::fs::read_to_string(tmp.path().join("DOCS/SCOPE-PROPOSED.md")).unwrap();
    assert!(
        !queue.contains(PATCH_ID),
        "queue should no longer contain the approved id, got: {queue}"
    );

    // Commit was produced and cites both the patch id and the
    // predicate-ref in its body.
    let log = StdCommand::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .unwrap();
    let msg = String::from_utf8_lossy(&log.stdout);
    assert!(msg.contains("scope-patch tighten"), "subject: {msg}");
    assert!(msg.contains(PATCH_ID), "id missing: {msg}");
    assert!(
        msg.contains("predicate-ref: no-emojis-in-source"),
        "predicate-ref missing: {msg}"
    );

    // The committed change set includes both the queue edit and the
    // target rulebook file.
    let show = StdCommand::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["show", "--stat", "--pretty=", "HEAD"])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&show.stdout);
    assert!(stat.contains("DOCS/SCOPE-PROPOSED.md"), "stat: {stat}");
    assert!(stat.contains("codeless/CLAUDE.md"), "stat: {stat}");
}

#[test]
fn approve_with_include_commits_extra_path() {
    let tmp = seed_repo();
    let target = tmp.path().join("codeless/CLAUDE.md");
    std::fs::write(&target, "edited rule\n").unwrap();

    // A new predicate file alongside the rulebook edit.
    let predicate_dir = tmp.path().join("crates/codeless-predicates/src/probes");
    std::fs::create_dir_all(&predicate_dir).unwrap();
    let predicate_file = predicate_dir.join("new_probe.rs");
    std::fs::write(&predicate_file, "// new predicate\n").unwrap();

    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "approve", "--repo"])
        .arg(tmp.path())
        .arg(PATCH_ID)
        .arg("--include")
        .arg("crates/codeless-predicates/src/probes/new_probe.rs")
        .assert()
        .success();

    let show = StdCommand::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["show", "--stat", "--pretty=", "HEAD"])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&show.stdout);
    assert!(stat.contains("new_probe.rs"), "stat: {stat}");
}

#[test]
fn approve_errors_when_target_missing() {
    let tmp = seed_repo();
    std::fs::remove_file(tmp.path().join("codeless/CLAUDE.md")).unwrap();
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "approve", "--repo"])
        .arg(tmp.path())
        .arg(PATCH_ID)
        .assert()
        .failure()
        .stderr(contains("does not exist"));
}

#[test]
fn reject_removes_entry_and_commits_reason() {
    let tmp = seed_repo();
    TestCommand::cargo_bin("codeless")
        .unwrap()
        .args(["patches", "reject", "--repo"])
        .arg(tmp.path())
        .arg(PATCH_ID)
        .arg("--reason")
        .arg("overconstrains R4")
        .assert()
        .success()
        .stdout(contains(format!("rejected {PATCH_ID}")));

    let queue = std::fs::read_to_string(tmp.path().join("DOCS/SCOPE-PROPOSED.md")).unwrap();
    assert!(!queue.contains(PATCH_ID));

    let log = StdCommand::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .unwrap();
    let msg = String::from_utf8_lossy(&log.stdout);
    assert!(msg.starts_with("scope-patch reject"), "subject: {msg}");
    assert!(msg.contains("overconstrains R4"));
    assert!(msg.contains(PATCH_ID));
}
