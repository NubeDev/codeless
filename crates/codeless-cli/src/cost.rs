//! `codeless cost <verb>` — rollups over the `Job.cost_cents` column.
//! Builds on `list_jobs`, so no new RPC method is needed and the
//! whole verb is dual-mode for free.
//!
//! Scope is intentionally small: total + per-repo + per-runner
//! breakdowns. Daily/monthly aggregates that require date math live
//! on the server side and are a follow-up; the client rollups here
//! cover the operator-facing "what did I just spend on this core"
//! question that the JobsDashboard would render in the corner.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use codeless_rpc::{ListJobsArgs, RpcServer};
use codeless_types::{Job, JobStatus, RepoId};
use serde::Serialize;

use crate::rpc_open;

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// Print a human-readable cost rollup: total, per-status,
    /// per-runner. Reads `--core` or `--db` like the other dual-mode
    /// verbs. Optional `--repo` narrows the rollup to one repo.
    Summary(SummaryArgs),
}

#[derive(Debug, Args)]
pub struct SummaryArgs {
    /// Repo ULID to narrow to. Without it, every job in the core is
    /// summed.
    #[arg(long)]
    pub repo: Option<String>,
    /// Emit the rollup as a JSON object instead of human-readable
    /// text. Useful for piping into `jq` or further aggregation.
    #[arg(long)]
    pub json: bool,
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
            Verb::Summary(args) => summary(rpc.as_ref(), args).await,
        }
    })
}

async fn summary(rpc: &dyn RpcServer, args: SummaryArgs) -> Result<ExitCode> {
    let repo_id = match args.repo {
        None => None,
        Some(s) => Some(
            s.parse::<RepoId>()
                .map_err(|e| anyhow!("invalid --repo {s:?}: {e}"))?,
        ),
    };
    let jobs = rpc
        .list_jobs(ListJobsArgs { repo_id })
        .await
        .map_err(|e| anyhow!("list_jobs: {e}"))?
        .jobs;

    let rollup = Rollup::from_jobs(&jobs);
    if args.json {
        println!("{}", serde_json::to_string(&rollup)?);
    } else {
        rollup.print();
    }
    Ok(ExitCode::SUCCESS)
}

/// Aggregated cost view. Kept as a small struct (rather than printing
/// inline in `summary`) so the same shape can be unit-tested without
/// going through the CLI; the `Display`-style print lives in
/// `print`.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
struct Rollup {
    total_cents: i64,
    job_count: usize,
    by_status: BTreeMap<String, (i64, usize)>,
    by_runner: BTreeMap<String, (i64, usize)>,
}

impl Rollup {
    fn from_jobs(jobs: &[Job]) -> Self {
        let mut r = Self::default();
        for job in jobs {
            r.total_cents += job.cost_cents.as_i64();
            r.job_count += 1;
            let status_key = status_label(&job.status);
            let s = r.by_status.entry(status_key).or_default();
            s.0 += job.cost_cents.as_i64();
            s.1 += 1;
            let runner_key = job.runner.clone();
            let v = r.by_runner.entry(runner_key).or_default();
            v.0 += job.cost_cents.as_i64();
            v.1 += 1;
        }
        r
    }

    fn print(&self) {
        if self.job_count == 0 {
            println!("(no jobs — total $0.00)");
            return;
        }
        println!(
            "total: {} across {} jobs",
            dollars(self.total_cents),
            self.job_count
        );
        println!("by status:");
        for (k, (cents, n)) in &self.by_status {
            println!("  {k:>10}: {} ({n} jobs)", dollars(*cents));
        }
        println!("by runner:");
        for (k, (cents, n)) in &self.by_runner {
            println!("  {k:>10}: {} ({n} jobs)", dollars(*cents));
        }
    }
}

/// `i64` cents → `$X.YY`. Negative values surface with a `-`
/// prefix; the codebase has no source of negative `cost_cents` but
/// printing rather than asserting keeps this function total.
fn dollars(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}${}.{:02}", abs / 100, abs % 100)
}

fn status_label(status: &JobStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{status:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{CostCents, JobId, RepoId, UnixMillis};

    fn mkjob(status: JobStatus, runner: &str, cents: i64) -> Job {
        Job {
            id: JobId::new(),
            repo_id: RepoId::new(),
            status,
            stop_reason: None,
            template_yaml: None,
            prompt: Some("hi".into()),
            runner: runner.into(),
            branch: "main".into(),
            worktree_path: None,
            cost_cap_cents: CostCents::ZERO,
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(cents),
            started_at: None,
            ended_at: None,
            created_at: UnixMillis(0),
        }
    }

    #[test]
    fn rollup_groups_by_status_and_runner() {
        let jobs = vec![
            mkjob(JobStatus::Completed, "anthropic", 250),
            mkjob(JobStatus::Completed, "anthropic", 750),
            mkjob(JobStatus::Failed, "claude", 100),
            mkjob(JobStatus::Queued, "mock", 0),
        ];
        let r = Rollup::from_jobs(&jobs);
        assert_eq!(r.total_cents, 1100);
        assert_eq!(r.job_count, 4);
        assert_eq!(r.by_status.get("completed"), Some(&(1000, 2)));
        assert_eq!(r.by_status.get("failed"), Some(&(100, 1)));
        assert_eq!(r.by_runner.get("anthropic"), Some(&(1000, 2)));
        assert_eq!(r.by_runner.get("mock"), Some(&(0, 1)));
    }

    #[test]
    fn dollars_format_handles_cents_under_ten() {
        assert_eq!(dollars(0), "$0.00");
        assert_eq!(dollars(5), "$0.05");
        assert_eq!(dollars(99), "$0.99");
        assert_eq!(dollars(100), "$1.00");
        assert_eq!(dollars(12345), "$123.45");
        assert_eq!(dollars(-250), "-$2.50");
    }
}
