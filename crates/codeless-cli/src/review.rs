//! `codeless review {list,approve,comment,stop}`. Each subcommand
//! drives one of the `RpcServer` review methods added in stage 2a;
//! results print as JSON lines on stdout so the existing
//! `codeless ... | jq` idiom keeps working across subcommands.
//!
//! The CLI deliberately holds no in-memory review state — it builds a
//! fresh `InProcessRpc`, calls one method, and exits. Persistent
//! review state lives in SQLite (R4); the CLI is just a typed front
//! door.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use codeless_rpc::{ApproveReviewArgs, CommentReviewArgs, ListReviewsArgs, StopReviewArgs};
use codeless_types::{ReviewId, ReviewStatus, StageId};

use crate::rpc_open;

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// List reviews, optionally narrowed by `--stage` and/or
    /// `--status`. Output is one JSON object per line, ordered by
    /// requested_at ascending.
    List(ListArgs),
    /// Resolve a `Pending` review to `Approved`. Errors with a
    /// non-zero exit code if the review is already resolved.
    Approve(ResolveArgs),
    /// Attach a free-form comment to a review without changing its
    /// status. Useful for both Pending iteration and post-mortem
    /// notes on already-resolved reviews.
    Comment(CommentArgs),
    /// Resolve a `Pending` review to `Stopped`. Same conflict
    /// semantics as `approve`.
    Stop(ResolveArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Filter to a single stage id (ULID).
    #[arg(long)]
    pub stage: Option<String>,
    /// Filter to a single review status: pending / approved /
    /// rejected / stopped / rerun-requested.
    #[arg(long)]
    pub status: Option<String>,
}

#[derive(Debug, Args)]
pub struct ResolveArgs {
    /// Review id (ULID).
    pub id: String,
}

#[derive(Debug, Args)]
pub struct CommentArgs {
    pub id: String,
    /// Comment body. Passed inline — multi-line bodies should be
    /// quoted by the shell.
    pub comment: String,
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
    rt.block_on(dispatch(verb, core, token, db))
}

async fn dispatch(
    verb: Verb,
    core: Option<String>,
    token: Option<String>,
    db: Option<PathBuf>,
) -> Result<ExitCode> {
    let rpc = rpc_open::build_dual_mode(core, token, db).await?;
    match verb {
        Verb::List(args) => {
            let stage = args.stage.as_deref().map(parse_stage_id).transpose()?;
            let status = args
                .status
                .as_deref()
                .map(parse_review_status)
                .transpose()?;
            let result = rpc
                .list_reviews(ListReviewsArgs {
                    job_id: None,
                    stage_id: stage,
                    status,
                })
                .await
                .map_err(|e| anyhow!("list_reviews: {e}"))?;
            for r in &result.reviews {
                println!("{}", serde_json::to_string(r)?);
            }
        }
        Verb::Approve(args) => {
            let review = rpc
                .approve_review(ApproveReviewArgs {
                    review_id: parse_review_id(&args.id)?,
                })
                .await
                .map_err(|e| anyhow!("approve_review: {e}"))?;
            println!("{}", serde_json::to_string(&review)?);
        }
        Verb::Comment(args) => {
            let review = rpc
                .comment_review(CommentReviewArgs {
                    review_id: parse_review_id(&args.id)?,
                    comment: args.comment,
                })
                .await
                .map_err(|e| anyhow!("comment_review: {e}"))?;
            println!("{}", serde_json::to_string(&review)?);
        }
        Verb::Stop(args) => {
            let review = rpc
                .stop_review(StopReviewArgs {
                    review_id: parse_review_id(&args.id)?,
                })
                .await
                .map_err(|e| anyhow!("stop_review: {e}"))?;
            println!("{}", serde_json::to_string(&review)?);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn parse_review_id(s: &str) -> Result<ReviewId> {
    ReviewId::from_str(s).map_err(|e| anyhow!("invalid review id {s:?}: {e}"))
}

fn parse_stage_id(s: &str) -> Result<StageId> {
    StageId::from_str(s).map_err(|e| anyhow!("invalid stage id {s:?}: {e}"))
}

fn parse_review_status(s: &str) -> Result<ReviewStatus> {
    Ok(match s {
        "pending" => ReviewStatus::Pending,
        "approved" => ReviewStatus::Approved,
        "rejected" => ReviewStatus::Rejected,
        "stopped" => ReviewStatus::Stopped,
        "rerun-requested" => ReviewStatus::RerunRequested,
        other => return Err(anyhow!("unknown review status {other:?}")),
    })
}
