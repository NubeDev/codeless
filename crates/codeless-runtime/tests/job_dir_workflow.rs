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
            cost_cap_cents: 0,
            wall_clock_cap_ms: 0,
        })
        .await
        .unwrap();
    (rpc, tmp, job.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_job_files_reports_none_layout_until_first_save() {
    let (rpc, _tmp, job_id) = fixture_with_job(Some(TEMPLATE_YAML)).await;
    let listed = rpc
        .list_job_files(ListJobFilesArgs { job_id })
        .await
        .unwrap();
    assert_eq!(listed.layout, "none");
    assert!(listed.entries.is_empty());
    assert!(listed.directory_path.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_job_file_creates_directory_layout() {
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
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(listed.entries[0].name, "design.md");

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_job_file_migrates_flat_layout_in_place() {
    let (rpc, tmp, job_id) = fixture_with_job(Some(TEMPLATE_YAML)).await;
    let flat_dir = tmp.path().join(".codeless/jobs");
    fs::create_dir_all(&flat_dir).unwrap();
    let flat = flat_dir.join("alpha.yaml");
    fs::write(&flat, TEMPLATE_YAML).unwrap();
    let out = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["add", "."])
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["commit", "-q", "-m", "seed flat layout"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let listed = rpc
        .list_job_files(ListJobFilesArgs { job_id })
        .await
        .unwrap();
    assert_eq!(listed.layout, "flat");

    rpc.write_job_file(WriteJobFileArgs {
        job_id,
        filename: "WORKFLOW.md".into(),
        content: "# Workflow\n".into(),
    })
    .await
    .unwrap();

    let listed = rpc
        .list_job_files(ListJobFilesArgs { job_id })
        .await
        .unwrap();
    assert_eq!(listed.layout, "directory");
    assert!(!flat.exists());
    let tpl = tmp.path().join(".codeless/jobs/alpha/template.yaml");
    assert!(tpl.exists());

    let subjects = git_log_subjects(tmp.path());
    let want_subjects = [
        "migrate template: alpha → directory layout",
        "migrate template: alpha (remove flat YAML)",
        "update job-file: alpha/WORKFLOW.md",
    ];
    for w in want_subjects {
        assert!(
            subjects.iter().any(|s| s == w),
            "missing subject {w}; got {subjects:?}"
        );
    }
}

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
