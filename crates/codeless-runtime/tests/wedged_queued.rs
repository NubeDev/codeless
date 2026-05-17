//! Regression gate for the wedged-Queued failure mode.
//!
//! Three bugs combine to leave a real job pinned in `Queued` with no
//! escape:
//!
//! 1. `WorktreeManager::create` returns `AlreadyExists` on path
//!    collision with no adoption or prune path.
//! 2. The driver loop is event-only — when `drive_job` returns an
//!    error the loop logs and walks away; the `Queued` row never gets
//!    another attempt.
//! 3. The state machine has no edge out of `Queued`, so the operator
//!    has no way to reset the row manually either.
//!
//! This test reproduces the failure end-to-end: a stale directory at
//! `<base>/job-<id>` is planted before the driver loop ticks, so
//! `drive_job`'s first try hits the `AlreadyExists` error path. After
//! the fix lands, the job must still reach a terminal status within a
//! bounded timeout (the worktree path is self-healed and/or the loop
//! retries). Today the job stays `Queued` and the test times out —
//! that is the failure this gate locks down.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use codeless_adapters_host::WorktreeManager;
use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{
    spawn_job_driver_loop_with_retry, InProcessRpc, MockRunner, MockStep, RetryPolicy, Runner,
    RunnerFactory, RunnerOutcome,
};
use codeless_types::{GitAuth, Job, JobStatus, WorkspaceMode};
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

/// Factory that hands every job the same scripted `MockRunner`. The
/// runner returns `Completed` immediately, so any non-terminal final
/// state is the driver loop's fault, not the runner's.
struct AlwaysMock;

impl RunnerFactory for AlwaysMock {
    fn build(&self, _job: &Job, _pending_operator_comment: Option<String>) -> Option<Arc<dyn Runner>> {
        Some(Arc::new(MockRunner::new(vec![MockStep::Finish(
            RunnerOutcome::Completed,
        )])))
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
        branch: "codeless/job-wedge".into(),
        workspace_mode: Some(WorkspaceMode::Worktree),
        cost_cap_cents: 500,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        auto_bypass_policy: None,
        start_immediately: true,
    })
    .await
    .expect("submit_job")
    .id
}

async fn wait_until_not_queued(
    rpc: &InProcessRpc,
    job_id: codeless_types::JobId,
    timeout: Duration,
) -> JobStatus {
    let deadline = Instant::now() + timeout;
    loop {
        let job = rpc
            .get_job(codeless_rpc::GetJobArgs { job_id })
            .await
            .expect("get_job");
        if job.status != JobStatus::Queued {
            return job.status;
        }
        if Instant::now() >= deadline {
            return job.status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Reproducer for the wedged-Queued failure. Plants a stale directory
/// at the worktree path the driver will try to create, then waits for
/// the job to reach a terminal status. With today's bugs the driver
/// loop errors once on `AlreadyExists`, never retries, and the row
/// stays `Queued` for the whole timeout — this assertion is the
/// regression gate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn driver_loop_recovers_from_preexisting_worktree_path() {
    let (_repo_dir, repo_path) = fresh_repo();
    let base = TempDir::new().unwrap();
    let mgr = Arc::new(WorktreeManager::new(base.path()));

    let rpc = Arc::new(InProcessRpc::new().await.unwrap());
    let job_id = submit(&rpc, &repo_path).await;

    // Plant a stale directory at the exact path `drive_job` will ask
    // `WorktreeManager::create` to use. Today this trips `AlreadyExists`
    // and the loop walks away without retrying.
    let stale = mgr.path_for(&job_id.to_string());
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("leftover.txt"), b"from a previous crash\n").unwrap();

    let handle = spawn_job_driver_loop_with_retry(
        Arc::clone(&rpc),
        Arc::new(AlwaysMock),
        Some(Arc::clone(&mgr)),
        1,
        RetryPolicy::test_fast(),
    )
    .await
    .expect("spawn driver loop");

    let final_status = wait_until_not_queued(&rpc, job_id, Duration::from_secs(3)).await;
    handle.cancel();
    let _ = handle.join().await;

    assert_ne!(
        final_status,
        JobStatus::Queued,
        "driver loop must self-heal a stale worktree path collision instead of wedging the job in Queued",
    );
}

/// Companion assertion: a job that the driver loop has given up on
/// must still be recoverable through a user-driven path. Stage 6
/// lands a `reset_job` RPC for the `Queued | Failed | Stopped ->
/// Draft` edge; this test pins the contract so the implementation
/// can't silently drift back to "no escape from Queued."
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_job_has_a_state_machine_escape_hatch() {
    use codeless_runtime::transition_job;
    use codeless_types::JobStatus;

    // The reset path: Queued -> Draft must become legal. Today this
    // edge is missing, so the assertion fails and Stage 6's RPC has a
    // gate to hit. The check is on the pure state-machine function so
    // the failure mode is unambiguous regardless of any RPC surface
    // changes the fix introduces.
    transition_job(JobStatus::Queued, JobStatus::Draft)
        .expect("Queued must transition to Draft for the reset_job recovery hatch");
}
