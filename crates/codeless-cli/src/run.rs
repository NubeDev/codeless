use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use codeless_rpc::{AddRepoArgs, EventFilter, RpcServer, SubmitJobArgs};
use codeless_runtime::{
    drive_job, AnthropicRunnerAdapter, ClaudeRunnerAdapter, MockRunner, MockStep, Runner,
    RunnerOutcome,
};
use codeless_types::{Event, GitAuth, TaskId};
use futures_util::StreamExt;

use crate::rpc_open;
use crate::{RunArgs, RunnerKind};

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
pub fn handle(args: RunArgs, db: Option<PathBuf>) -> Result<ExitCode> {
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
    rt.block_on(run_once(args, repo_path, db))
}

async fn run_once(args: RunArgs, repo_path: PathBuf, db: Option<PathBuf>) -> Result<ExitCode> {
    let rpc = Arc::new(rpc_open::open(db.as_deref()).await?);

    let wire_runner = args.runner.as_wire();
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
            // Real runners that push their work will fail here until
            // git auth is threaded through — tracked separately.
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: Some(wire_runner.to_string()),
        })
        .await
        .map_err(|e| anyhow!("add_repo: {e}"))?;

    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some(args.prompt.clone()),
            template_yaml: None,
            runner: wire_runner.to_string(),
            branch: "codeless/job-once".into(),
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            // `codeless run` is a streaming one-shot CLI — the user
            // would never expect it to land a Draft they then have to
            // promote separately. Submit-and-run preserves intent.
            start_immediately: true,
        })
        .await
        .map_err(|e| anyhow!("submit_job: {e}"))?;

    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id: job.id }, None)
        .await
        .map_err(|e| anyhow!("subscribe: {e}"))?;

    let runner = build_runner(&args)?;
    let drive_rpc = Arc::clone(&rpc);
    let drive_job_id = job.id;
    let drive = tokio::spawn(async move {
        drive_job(&drive_rpc, drive_job_id, runner, None)
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

/// Build the concrete adapter for the chosen runner kind. The mock
/// path keeps the Phase 1 scripted transcript so tests asserting
/// `task-started` + `task-completed` framing keep passing without an
/// external binary. The claude and anthropic paths share a fresh
/// `TaskId` so AI events carry a stable correlator on the bus.
fn build_runner(args: &RunArgs) -> Result<Arc<dyn Runner>> {
    Ok(match args.runner {
        RunnerKind::Mock => Arc::new(MockRunner::new(scripted_for(&args.prompt))),
        RunnerKind::Claude => {
            Arc::new(ClaudeRunnerAdapter::new(args.prompt.clone(), TaskId::new()))
        }
        RunnerKind::Anthropic => {
            let key = args
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok());
            if key.is_none() {
                bail!("--runner anthropic requires --api-key or ANTHROPIC_API_KEY env var");
            }
            let mut adapter = AnthropicRunnerAdapter::new(args.prompt.clone(), TaskId::new());
            adapter.api_key = key;
            adapter.base_url = args.base_url.clone();
            Arc::new(adapter)
        }
    })
}

fn scripted_for(_prompt: &str) -> Vec<MockStep> {
    use codeless_types::TaskStatus;
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
