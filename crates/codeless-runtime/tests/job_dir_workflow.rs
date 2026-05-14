//! Integration tests for the job-as-directory file surface. Each test
//! drives a real `InProcessRpc` against a real on-disk repo so the
//! `commit_paths` invocations from `write_job_file` / `delete_job_file`
//! / `migrate_flat_to_directory` produce real commits in real git
//! history — the same shape an end user gets when editing files from
//! the Spec pane.
//!
//! The test repo is initialised with a default branch + one root
//! commit so `git commit` has somewhere to anchor; otherwise the first
//! call into `commit_paths` would fail with "does not have any
//! commits yet" on stock git.

use std::fs;
use std::path::Path;
use std::process::Command;

use codeless_rpc::{
    AddRepoArgs, DeleteJobFileArgs, ListJobFilesArgs, ReadJobFileArgs, RpcError, RpcServer,
    SubmitJobArgs, WriteJobFileArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_types::GitAuth;
use tempfile::TempDir;

fn init_repo(dir: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "test@example.com"][..],
        &["config", "user.name", "test"][..],
        &["commit", "--allow-empty", "-q", "-m", "root"][..],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args.iter().copied())
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?}: {:?}", out);
    }
}

fn git_log_subjects(repo: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "--pretty=%s"])
        .output()
        .expect("git log");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

const TEMPLATE_YAML: &str = "name: alpha\ngoal: g\nstages:\n  - one\n";

async fn fixture_with_job(
    template_yaml: Option<&str>,
) -> (InProcessRpc, TempDir, codeless_types::JobId) {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());

    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: tmp.path().to_string_lossy().into_owned(),
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
            prompt: None,
            template_yaml: template_yaml.map(|s| s.to_string()),
            runner: "mock".into(),
            branch: "codeless/job-x".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    (rpc, tmp, job.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_seeds_directory_with_template_scope_workflow() {
    // The new submit-time scaffold contract: a job whose
    // `template_yaml` parses as a `JobTemplate` lands with its
    // directory already on disk (template.yaml + SCOPE.md +
    // WORKFLOW.md), all three committed in one `scaffold job: <name>`
    // commit. The user never has to "promote" — the spec is editable
    // from the moment the job exists.
    let (rpc, tmp, job_id) = fixture_with_job(Some(TEMPLATE_YAML)).await;
    let listed = rpc
        .list_job_files(ListJobFilesArgs { job_id })
        .await
        .unwrap();
    assert_eq!(listed.layout, "directory");
    let names: Vec<&str> = listed.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"template.yaml"), "got {names:?}");
    assert!(names.contains(&"SCOPE.md"), "got {names:?}");
    assert!(names.contains(&"WORKFLOW.md"), "got {names:?}");
    let dir = tmp.path().join(".codeless/jobs/alpha");
    assert!(dir.join("template.yaml").exists());
    assert!(dir.join("SCOPE.md").exists());
    assert!(dir.join("WORKFLOW.md").exists());
    let subjects = git_log_subjects(tmp.path());
    assert!(
        subjects.iter().any(|s| s == "scaffold job: alpha"),
        "missing scaffold commit; got {subjects:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_job_file_appends_into_seeded_directory() {
    let (rpc, tmp, job_id) = fixture_with_job(Some(TEMPLATE_YAML)).await;
    let result = rpc
        .write_job_file(WriteJobFileArgs {
            job_id,
            filename: "design".into(),
            content: "hello".into(),
        })
        .await
        .unwrap();
    assert_eq!(result.name, "design.md");

    let listed = rpc
        .list_job_files(ListJobFilesArgs { job_id })
        .await
        .unwrap();
    assert_eq!(listed.layout, "directory");
    let names: Vec<&str> = listed.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"design.md"), "got {names:?}");

    let on_disk = tmp.path().join(".codeless/jobs/alpha/design.md");
    assert_eq!(fs::read_to_string(&on_disk).unwrap(), "hello");
    let subjects = git_log_subjects(tmp.path());
    assert!(subjects
        .iter()
        .any(|s| s == "update job-file: alpha/design.md"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_job_file_adds_scope_md_and_commits() {
    let (rpc, tmp, job_id) = fixture_with_job(Some(TEMPLATE_YAML)).await;
    rpc.write_job_file(WriteJobFileArgs {
        job_id,
        filename: "SCOPE.md".into(),
        content: "# Scope\n\nWhat the job is for.\n".into(),
    })
    .await
    .unwrap();

    let listed = rpc
        .list_job_files(ListJobFilesArgs { job_id })
        .await
        .unwrap();
    let scope = listed
        .entries
        .iter()
        .find(|e| e.name == "SCOPE.md")
        .unwrap();
    assert!(scope.is_scope);
    assert!(!scope.is_workflow);
    assert!(!scope.is_template);

    let read = rpc
        .read_job_file(ReadJobFileArgs {
            job_id,
            filename: "SCOPE.md".into(),
        })
        .await
        .unwrap();
    assert!(read.content.contains("What the job is for"));

    let subjects = git_log_subjects(tmp.path());
    assert!(subjects
        .iter()
        .any(|s| s == "update job-file: alpha/SCOPE.md"));
}

// The flat-layout migration path (`migrate_flat_to_directory` in
// `rpc.rs`) is kept for legacy DBs whose `.codeless/jobs/<name>.yaml`
// predates the directory layout. With submit-time scaffolding, no
// fresh job ever lands in the flat shape, so the integration test
// here would have to fight the scaffold to recreate the scenario.
// The migration helper itself stays defended by direct unit tests on
// `job_dir::resolve` rather than this end-to-end fixture.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_job_file_refuses_template_yaml() {
    let (rpc, _tmp, job_id) = fixture_with_job(Some(TEMPLATE_YAML)).await;
    rpc.write_job_file(WriteJobFileArgs {
        job_id,
        filename: "design".into(),
        content: "hello".into(),
    })
    .await
    .unwrap();

    let err = rpc
        .delete_job_file(DeleteJobFileArgs {
            job_id,
            filename: "template.yaml".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_job_file_rejects_path_traversal() {
    let (rpc, _tmp, job_id) = fixture_with_job(Some(TEMPLATE_YAML)).await;
    let err = rpc
        .read_job_file(ReadJobFileArgs {
            job_id,
            filename: "../escape.md".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
}
