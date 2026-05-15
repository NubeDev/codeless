//! End-to-end `codeless review` exercise. Seeds a review row via the
//! same `InProcessRpc::with_file` helper the CLI uses, then drives
//! each subcommand as a fresh subprocess against the same SQLite
//! file. Asserts both the JSON-line stdout shape and the persisted
//! status transitions.

use assert_cmd::Command as TestCommand;
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{
    GitAuth, Review, ReviewId, ReviewStatus, Stage, StageId, StageStatus, UnixMillis,
};
use predicates::str::contains;
use tempfile::TempDir;

async fn seed(db_path: &std::path::Path) -> ReviewId {
    let rpc = InProcessRpc::with_file(db_path).await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/codeless-demo-not-used".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/job-review-cli".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    let stage = Stage {
        id: StageId::new(),
        job_id: job.id,
        ordinal: 0,
        name: "verify".into(),
        status: StageStatus::AwaitingReview,
        verify_cmd: None,
        started_at: None,
        ended_at: None,
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
        persona_id: None,
    };
    rpc.store().insert_stage(&stage).await.unwrap();
    let review = Review {
        id: ReviewId::new(),
        stage_id: stage.id,
        status: ReviewStatus::Pending,
        comment: None,
        requested_at: UnixMillis(1_000),
        resolved_at: None,
    };
    rpc.store().insert_review(&review).await.unwrap();
    review.id
}

fn db_file(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("codeless.db")
}

#[test]
fn review_list_approve_comment_stop_round_trip() {
    let dir = TempDir::new().unwrap();
    let db = db_file(&dir);
    let db_str = db.to_str().unwrap().to_string();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let review_id = rt.block_on(seed(&db));
    let id_str = review_id.to_string();

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "list"])
        .assert()
        .success()
        .stdout(contains(&id_str))
        .stdout(contains("\"status\":\"pending\""));

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args([
            "--db",
            &db_str,
            "review",
            "comment",
            &id_str,
            "needs another look",
        ])
        .assert()
        .success()
        .stdout(contains("needs another look"))
        .stdout(contains("\"status\":\"pending\""));

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "approve", &id_str])
        .assert()
        .success()
        .stdout(contains("\"status\":\"approved\""))
        .stdout(contains("needs another look"));

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "approve", &id_str])
        .assert()
        .failure()
        .stderr(contains("already resolved"));

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "stop", &id_str])
        .assert()
        .failure()
        .stderr(contains("already resolved"));
}

#[test]
fn review_list_filters_by_status() {
    let dir = TempDir::new().unwrap();
    let db = db_file(&dir);
    let db_str = db.to_str().unwrap().to_string();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let review_id = rt.block_on(seed(&db));
    let id_str = review_id.to_string();

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "list", "--status", "approved"])
        .assert()
        .success()
        .stdout(predicates::str::is_empty());

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "approve", &id_str])
        .assert()
        .success();

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "list", "--status", "approved"])
        .assert()
        .success()
        .stdout(contains(&id_str));
}

#[test]
fn review_rejects_unknown_status_filter() {
    let dir = TempDir::new().unwrap();
    let db = db_file(&dir);
    let db_str = db.to_str().unwrap().to_string();

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args(["--db", &db_str, "review", "list", "--status", "bogus"])
        .assert()
        .failure()
        .stderr(contains("unknown review status"));
}
