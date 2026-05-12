//! End-to-end exercise of `codeless job submit <file.yaml>`. Seeds a
//! repo via `InProcessRpc::with_file` so the YAML template can
//! reference a real repo id, invokes the CLI as a subprocess, and
//! asserts the printed job row plus the persisted `template_yaml`
//! round-trip. Also pins the unknown-field / syntax-error paths so
//! template typos surface loudly.

use assert_cmd::Command as TestCommand;
use codeless_rpc::{AddRepoArgs, ListJobsArgs, RpcServer};
use codeless_runtime::InProcessRpc;
use codeless_types::{GitAuth, RepoId};
use predicates::str::contains;
use tempfile::TempDir;

async fn seed_repo(db_path: &std::path::Path) -> RepoId {
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
    repo.id
}

fn write_yaml(dir: &TempDir, body: &str) -> std::path::PathBuf {
    let path = dir.path().join("job.yaml");
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn submit_round_trips_two_stage_template() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    let db_str = db.to_str().unwrap().to_string();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let repo_id = rt.block_on(seed_repo(&db));

    let yaml_body = format!(
        "repo: {repo_id}
runner: mock
prompt: refactor the parser
branch: codeless/refactor-parser
stages:
  - name: plan
  - name: verify
    verify_cmd: cargo test
caps:
  cost_cents: 500
  wall_clock_ms: 60000
"
    );
    let yaml_path = write_yaml(&dir, &yaml_body);

    let assert = TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args([
            "--db",
            &db_str,
            "job",
            "submit",
            yaml_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let job: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(job["runner"], "mock");
    assert_eq!(job["branch"], "codeless/refactor-parser");
    assert_eq!(job["cost_cap_cents"], 500);
    assert_eq!(job["wall_clock_cap_ms"], 60_000);

    let rpc = rt.block_on(InProcessRpc::with_file(&db)).unwrap();
    let listed = rt
        .block_on(rpc.list_jobs(ListJobsArgs { repo_id: None }))
        .unwrap();
    assert_eq!(listed.jobs.len(), 1, "exactly one job persisted");
    let stored_yaml = listed.jobs[0]
        .template_yaml
        .as_deref()
        .expect("template_yaml round-tripped onto the row");
    assert!(stored_yaml.contains("name: plan"));
    assert!(stored_yaml.contains("verify_cmd: cargo test"));
}

#[test]
fn submit_rejects_unknown_field_with_line_column() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    let db_str = db.to_str().unwrap().to_string();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _ = rt.block_on(seed_repo(&db));

    let yaml = "repo: 01JABCDEF0123456789ABCDEFG\n\
                runneer: mock\n\
                branch: feat/x\n";
    let yaml_path = write_yaml(&dir, yaml);

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args([
            "--db",
            &db_str,
            "job",
            "submit",
            yaml_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(contains("runneer"))
        .stderr(contains("line"));
}

#[test]
fn submit_rejects_missing_required_field() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("codeless.db");
    let db_str = db.to_str().unwrap().to_string();

    let yaml = "runner: mock\nbranch: feat/x\n";
    let yaml_path = write_yaml(&dir, yaml);

    TestCommand::cargo_bin("codeless")
        .expect("codeless binary")
        .args([
            "--db",
            &db_str,
            "job",
            "submit",
            yaml_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(contains("repo"));
}
