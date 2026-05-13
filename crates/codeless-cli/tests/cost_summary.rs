//! `codeless cost summary` — local-mode rollup against a pre-seeded
//! DB. The verb is dual-mode by construction (built on the shared
//! `build_dual_mode` dispatcher); the hosted-mode round trip is
//! already covered by the `jobs_dual_mode` / `tail_hosted` tests, so
//! here we focus on the rollup semantics: totals match, empty cores
//! say so, and the per-runner / per-status breakdown surfaces.
//!
//! The `cost_cents` column is populated by the runner adapters at
//! run-time; seeding non-zero values directly would require a raw
//! SQL update path that's not exposed on the runtime API. We test
//! the rollup against the natural cost=0 state and verify the
//! totalling logic itself in the cost.rs unit tests.

use std::path::Path;

use assert_cmd::Command as TestCommand;
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::GitAuth;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_codeless");

async fn seed_three_jobs(db: &Path) {
    let rpc = InProcessRpc::with_file(db).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "cost-demo".into(),
            clone_url: "https://example.test/cost.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/cost-demo".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    for runner in ["mock", "anthropic", "claude"] {
        rpc.submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: runner.into(),
            branch: format!("feat/{runner}"),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summary_lists_three_runners() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    seed_three_jobs(&db).await;

    let out = TestCommand::new(BIN)
        .args(["--db", db.to_str().unwrap(), "cost", "summary"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("3 jobs"), "missing job count: {stdout}");
    assert!(stdout.contains("anthropic"), "stdout: {stdout}");
    assert!(stdout.contains("claude"), "stdout: {stdout}");
    assert!(stdout.contains("mock"), "stdout: {stdout}");
    assert!(stdout.contains("queued"), "stdout: {stdout}");
    // cost_cents is zero until a runner actually executes; the
    // totalling logic itself is unit-tested in cost.rs.
    assert!(stdout.contains("$0.00"), "stdout: {stdout}");
}

#[test]
fn summary_on_empty_core_reports_no_jobs() {
    let out = TestCommand::new(BIN)
        .args(["cost", "summary"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("no jobs"), "stdout: {stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summary_json_emits_rollup_object() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    seed_three_jobs(&db).await;

    let out = TestCommand::new(BIN)
        .args(["--db", db.to_str().unwrap(), "cost", "summary", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("not JSON");
    assert_eq!(v["total_cents"], 0);
    assert_eq!(v["job_count"], 3);
    assert!(v["by_status"].is_object());
    assert!(v["by_runner"].is_object());
}

#[test]
fn summary_rejects_bad_repo_id() {
    let out = TestCommand::new(BIN)
        .args(["cost", "summary", "--repo", "not-a-ulid"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}
