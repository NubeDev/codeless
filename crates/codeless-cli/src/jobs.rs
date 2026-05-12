//! `codeless jobs <verb>` — basic read + stop surface for jobs that
//! works in both local-mode (`--db`) and hosted-mode
//! (`--core URL --token T`). Sister of `repos.rs`; the same
//! `rpc_open::build_dual_mode` helper picks the transport so neither
//! verb can drift from the other on auth or URL handling.
//!
//! Out of scope here: submission. `codeless run` and `codeless job
//! submit` already cover the two ergonomic shapes for that — the
//! point of `jobs` is to inspect and stop work the core is already
//! running, which is what the browser's JobsDashboard does too.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use codeless_rpc::{GetJobArgs, ListJobsArgs, ListJobsResult, RpcServer, StopJobArgs};
use codeless_types::{Job, JobId, RepoId};

use crate::rpc_open;

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// List jobs across every repo, or narrow to `--repo <repo-id>`.
    /// Output is one job per line: `<job_id>\t<status>\t<repo_id>\t<runner>\t<branch>`.
    List(ListArgs),
    /// Print the full Job row as a JSON object — useful for shell
    /// scripts that want to read individual fields with `jq`.
    Get { job_id: String },
    /// Mark a queued/running job as stopped (`reason: user`).
    /// Idempotent against already-terminal jobs surfaces as
    /// `RpcError::Conflict` so callers can distinguish "stopped" from
    /// "already stopped".
    Stop { job_id: String },
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Repo ULID to narrow the listing to. Without it, every job in
    /// the core is returned.
    #[arg(long)]
    pub repo: Option<String>,
}

pub fn handle(
    verb: Verb,
    core: Option<String>,
    token: Option<String>,
    db: Option<PathBuf>,
) -> Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let rpc = rpc_open::build_dual_mode(core, token, db).await?;
        match verb {
            Verb::List(args) => list(rpc.as_ref(), args).await,
            Verb::Get { job_id } => get(rpc.as_ref(), &job_id).await,
            Verb::Stop { job_id } => stop(rpc.as_ref(), &job_id).await,
        }
    })
}

async fn list(rpc: &dyn RpcServer, args: ListArgs) -> Result<ExitCode> {
    let repo_id = match args.repo {
        None => None,
        Some(s) => Some(RepoId::from_str(&s).map_err(|e| anyhow!("invalid --repo {s:?}: {e}"))?),
    };
    let ListJobsResult { jobs } = rpc
        .list_jobs(ListJobsArgs { repo_id })
        .await
        .map_err(|e| anyhow!("list_jobs: {e}"))?;
    if jobs.is_empty() {
        eprintln!("(no jobs)");
        return Ok(ExitCode::SUCCESS);
    }
    for job in &jobs {
        println!(
            "{}\t{:?}\t{}\t{}\t{}",
            job.id, job.status, job.repo_id, job.runner, job.branch,
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn get(rpc: &dyn RpcServer, raw: &str) -> Result<ExitCode> {
    let job_id = JobId::from_str(raw).map_err(|e| anyhow!("invalid job id {raw:?}: {e}"))?;
    let job: Job = rpc
        .get_job(GetJobArgs { job_id })
        .await
        .map_err(|e| anyhow!("get_job: {e}"))?;
    println!("{}", serde_json::to_string_pretty(&job)?);
    Ok(ExitCode::SUCCESS)
}

async fn stop(rpc: &dyn RpcServer, raw: &str) -> Result<ExitCode> {
    let job_id = JobId::from_str(raw).map_err(|e| anyhow!("invalid job id {raw:?}: {e}"))?;
    rpc.stop_job(StopJobArgs { job_id })
        .await
        .map_err(|e| anyhow!("stop_job: {e}"))?;
    println!("stopped {job_id}");
    Ok(ExitCode::SUCCESS)
}
