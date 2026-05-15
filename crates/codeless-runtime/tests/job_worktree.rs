//! `drive_job` with a real `WorktreeManager` — pins the per-job
//! `git worktree` lifecycle promised by SCOPE.md "Workspace = one
//! `git worktree` per job" and "Crash recovery":
//!
//! - The worktree exists on disk while the runner is executing.
//! - It is **preserved** on every terminal status (completed, failed,
//!   stopped) so the user can inspect or re-run from where the job
//!   left off. SCOPE.md: "The reaper either preserves it (default —
//!   user can inspect / re-run from where it was) or removes it
//!   (configurable). It does not silently delete user-visible work."
//! - The path is persisted on the job row, so the UI and a future
//!   user-driven `gc_worktrees` action can both find it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use codeless_adapters_host::WorktreeManager;
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, InProcessRpc, Runner, RunnerContext, RunnerOutcome};
use codeless_types::{GitAuth, JobStatus, WorkspaceMode};
use parking_lot::Mutex;
use tempfile::TempDir;

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example")
        .args(args)
        .output()
        .expect("git binary");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fresh_repo() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_path_buf();
    git(&repo, &["init", "--initial-branch=main", "."]);
    std::fs::write(repo.join("seed.txt"), "seed\n").unwrap();
    git(&repo, &["add", "seed.txt"]);
    git(&repo, &["commit", "-m", "seed"]);
    (dir, repo)
}

/// Snapshot of `ctx.worktree_path` and whether it pointed at a real
/// directory at the moment the runner ran. Captured atomically so the
/// test can assert both fields together after the run.
#[derive(Debug, Default, Clone)]
struct Probe {
    seen_path: Option<PathBuf>,
    existed_during_run: bool,
}

/// Runner that records what it saw on its `RunnerContext` then
/// returns a pre-scripted outcome. Optionally parks before returning,
/// so `stop_during_run_wins` can race the stop RPC against it.
struct ProbeRunner {
    probe: Arc<Mutex<Probe>>,
    outcome: RunnerOutcome,
    delay: Duration,
}

#[async_trait]
impl Runner for ProbeRunner {
    async fn run(&self, ctx: RunnerContext) -> RunnerOutcome {
        let exists = ctx
            .worktree_path
            .as_ref()
            .map(|p| p.is_dir())
            .unwrap_or(false);
        *self.probe.lock() = Probe {
            seen_path: ctx.worktree_path.clone(),
            existed_during_run: exists,
        };
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.outcome.clone()
    }
}

async fn submit(rpc: &InProcessRpc, repo_path: &Path) -> codeless_types::JobId {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: repo_path.to_string_lossy().into_owned(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .expect("add_repo");
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("anything".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/job-wt".into(),
        workspace_mode: Some(WorkspaceMode::Worktree),
        cost_cap_cents: 500,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        start_immediately: true,
    })
    .await
    .expect("submit_job")
    .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worktree_preserved_after_completion() {
    let (_repo_dir, repo_path) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = Arc::new(WorktreeManager::new(base.path()));

    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = submit(&rpc, &repo_path).await;

    let probe = Arc::new(Mutex::new(Probe::default()));
    let runner = Arc::new(ProbeRunner {
        probe: Arc::clone(&probe),
        outcome: RunnerOutcome::Completed,
        delay: Duration::ZERO,
    });

    drive_job(&rpc, job_id, runner, Some(Arc::clone(&mgr)))
        .await
        .expect("drive_job");

    let p = probe.lock().clone();
    let seen = p.seen_path.expect("runner saw a worktree path");
    assert_eq!(seen, mgr.path_for(&job_id.to_string()));
    assert!(p.existed_during_run, "worktree must exist while running");
    assert!(
        seen.exists(),
        "worktree must be preserved at terminal status so the user can inspect / re-run",
    );

    let job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .expect("get_job");
    assert_eq!(job.status, JobStatus::Completed);
    assert_eq!(
        job.worktree_path.as_deref(),
        Some(seen.to_string_lossy().as_ref()),
        "worktree path is persisted for crash recovery"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worktree_preserved_on_failure() {
    let (_repo_dir, repo_path) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = Arc::new(WorktreeManager::new(base.path()));

    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = submit(&rpc, &repo_path).await;

    let probe = Arc::new(Mutex::new(Probe::default()));
    let runner = Arc::new(ProbeRunner {
        probe: Arc::clone(&probe),
        outcome: RunnerOutcome::Failed {
            reason: "scripted".into(),
        },
        delay: Duration::ZERO,
    });

    drive_job(&rpc, job_id, runner, Some(Arc::clone(&mgr)))
        .await
        .expect("drive_job");

    let seen = probe.lock().seen_path.clone().unwrap();
    assert!(
        seen.exists(),
        "worktree must be preserved on Failed terminal — debugging needs it",
    );
    let job = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .expect("get_job");
    assert_eq!(job.status, JobStatus::Failed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worktree_preserved_when_stop_wins_against_completion() {
    let (_repo_dir, repo_path) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = Arc::new(WorktreeManager::new(base.path()));

    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = submit(&rpc, &repo_path).await;

    let probe = Arc::new(Mutex::new(Probe::default()));
    let runner = Arc::new(ProbeRunner {
        probe: Arc::clone(&probe),
        outcome: RunnerOutcome::Completed,
        delay: Duration::from_millis(80),
    });

    let rpc_ref: &'static InProcessRpc = Box::leak(Box::new(rpc));
    let mgr_clone = Arc::clone(&mgr);
    let runner_clone: Arc<dyn Runner> = runner;
    let drive_handle =
        tokio::spawn(
            async move { drive_job(rpc_ref, job_id, runner_clone, Some(mgr_clone)).await },
        );

    tokio::time::sleep(Duration::from_millis(20)).await;
    rpc_ref
        .stop_job(codeless_rpc::StopJobArgs { job_id })
        .await
        .expect("stop_job");

    drive_handle.await.expect("join").expect("drive_job");

    let seen = probe.lock().seen_path.clone().unwrap();
    assert!(
        seen.exists(),
        "worktree must be preserved when stop wins — the user stopped it precisely to inspect the partial state",
    );
    let job = rpc_ref
        .get_job(codeless_rpc::GetJobArgs { job_id })
        .await
        .expect("get_job");
    assert_eq!(job.status, JobStatus::Stopped);
}
