//! `codeless tail <job-id>`. Subscribes to the event stream for a
//! single job, streams every envelope as a JSON line on stdout, and
//! exits on the terminal framing event (`job-completed`,
//! `job-failed`, `job-stopped`). Mirrors the drain loop the existing
//! `run --once` subcommand uses so the on-the-wire shape stays
//! identical: `codeless tail <id> | jq` is the same idiom as
//! `codeless run | jq`.
//!
//! `since: Some(EventCursor(0))` so the subscription replays every
//! event already persisted for the job before going live — a re-run
//! after a crash sees the same trace as the original invocation. The
//! bus interprets `None` as "live tail only", which is the wrong
//! choice for a `tail` command: a job that already reached terminal
//! status would emit nothing.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::Args;
use codeless_rpc::{EventFilter, RpcServer};
use codeless_types::{Event, EventCursor, JobId};
use futures_util::StreamExt;

use crate::rpc_open;

#[derive(Debug, Args)]
pub struct TailArgs {
    /// Job id (ULID) to follow.
    pub job_id: String,
    /// Hard upper bound on time spent waiting for events, in seconds.
    /// 0 disables the timeout. Defaults to 600s so a hung daemon
    /// can't keep the CLI alive forever, while still leaving room
    /// for a real long-running job to finish.
    #[arg(long, default_value_t = 600)]
    pub timeout_secs: u64,
}

pub fn handle(args: TailArgs, db: Option<PathBuf>) -> Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(args, db))
}

async fn run(args: TailArgs, db: Option<PathBuf>) -> Result<ExitCode> {
    let job_id = JobId::from_str(&args.job_id)
        .map_err(|e| anyhow!("invalid job id {:?}: {e}", args.job_id))?;
    let rpc = rpc_open::open(db.as_deref()).await?;
    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id }, Some(EventCursor(0)))
        .await
        .map_err(|e| anyhow!("subscribe: {e}"))?;

    let mut exit = ExitCode::SUCCESS;
    let drain = async {
        while let Some(item) = stream.next().await {
            let env = item.map_err(|e| anyhow!("event stream: {e}"))?;
            println!("{}", serde_json::to_string(&env)?);
            match env.event {
                Event::JobCompleted { .. } => return Ok::<_, anyhow::Error>(()),
                Event::JobFailed { .. } | Event::JobStopped { .. } => {
                    exit = ExitCode::FAILURE;
                    return Ok(());
                }
                _ => {}
            }
        }
        Ok(())
    };

    if args.timeout_secs == 0 {
        drain.await?;
    } else {
        tokio::time::timeout(Duration::from_secs(args.timeout_secs), drain)
            .await
            .map_err(|_| {
                anyhow!(
                    "tail timed out after {}s without a terminal event",
                    args.timeout_secs
                )
            })??;
    }
    Ok(exit)
}
