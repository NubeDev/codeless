use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use codeless_rpc::{AddRepoArgs, EventFilter, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, InProcessRpc, MockRunner, MockStep, RunnerOutcome};
use codeless_types::{Event, GitAuth};
use futures_util::StreamExt;

use crate::RunArgs;

/// End-to-end runner for `codeless run --once`. Builds an
/// in-process RPC, registers the repo, submits the job, subscribes to
/// its event stream, and runs the chosen `Runner` to completion while
/// echoing each `EventEnvelope` as a JSON line on stdout.
///
/// Streaming choice: events are flushed line-by-line so a piped
/// `codeless run | jq` works the same as a TTY invocation. The
/// driver also publishes `job-started` / `job-completed` /
/// `job-failed`; the loop terminates on those final framing events,
/// not on the runner's `RunnerOutcome` directly, so the exit code
/// reflects what an outside observer would see on the wire.
pub fn handle(args: RunArgs) -> Result<ExitCode> {
    if args.runner != "mock" {
        bail!(
            "runner {:?} is not wired in Phase 1; only `mock` is available",
            args.runner
        );
    }
    let repo_path = args
        .repo
        .canonicalize()
        .map_err(|e| anyhow!("--repo {}: {e}", args.repo.display()))?;
    if !repo_path.is_dir() {
        bail!("--repo {} is not a directory", repo_path.display());
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_once(args.prompt, repo_path))
}

async fn run_once(prompt: String, repo_path: std::path::PathBuf) -> Result<ExitCode> {
    let rpc = Arc::new(InProcessRpc::new());

    let repo = rpc
        .add_repo(AddRepoArgs {
            name: repo_path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repo".into()),
            clone_url: format!("file://{}", repo_path.display()),
            default_branch: "main".into(),
            local_path: repo_path.display().to_string(),
            // The mock runner never reaches a remote; the token env
            // var below is a placeholder that satisfies the wire
            // contract without requiring `GITHUB_TOKEN` to be set.
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: Some("mock".into()),
        })
        .await
        .map_err(|e| anyhow!("add_repo: {e}"))?;

    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some(prompt.clone()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/job-once".into(),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
        })
        .await
        .map_err(|e| anyhow!("submit_job: {e}"))?;

    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id: job.id }, None)
        .await
        .map_err(|e| anyhow!("subscribe: {e}"))?;

    let runner = Arc::new(MockRunner::new(scripted_for(&prompt)));
    let drive_rpc = Arc::clone(&rpc);
    let drive_job_id = job.id;
    let drive = tokio::spawn(async move {
        drive_job(&drive_rpc, drive_job_id, runner)
            .await
            .map_err(|e| anyhow!("drive_job: {e}"))
    });

    let mut exit = ExitCode::SUCCESS;
    let drain = async {
        while let Some(item) = stream.next().await {
            let env = match item {
                Ok(env) => env,
                Err(e) => return Err(anyhow!("event stream: {e}")),
            };
            let line = serde_json::to_string(&env)?;
            println!("{line}");
            match env.event {
                Event::JobCompleted { .. } => break,
                Event::JobFailed { .. } | Event::JobStopped { .. } => {
                    exit = ExitCode::FAILURE;
                    break;
                }
                _ => {}
            }
        }
        Ok::<_, anyhow::Error>(())
    };

    tokio::time::timeout(Duration::from_secs(120), drain)
        .await
        .map_err(|_| anyhow!("timed out after 120s without a terminal event"))??;

    drive.await??;
    Ok(exit)
}

fn scripted_for(_prompt: &str) -> Vec<MockStep> {
    // Phase 1 mock script: emit a single task-started + task-completed
    // pair so the streamed output has structure beyond just the
    // framing events, then finish clean. Real runners replace this
    // with their own event stream.
    use codeless_types::{TaskId, TaskStatus};
    let task = TaskId::new();
    vec![
        MockStep::Emit(Event::TaskStarted { task_id: task }),
        MockStep::Emit(Event::TaskCompleted {
            task_id: task,
            status: TaskStatus::Completed,
        }),
        MockStep::Finish(RunnerOutcome::Completed),
    ]
}
