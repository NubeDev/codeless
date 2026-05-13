//! `codeless job submit <file.yaml>`. Parses a typed job-template
//! YAML, hands the raw text through as `SubmitJobArgs.template_yaml`
//! (so the runtime keeps an unambiguous record), and prints the
//! resulting `Job` as a JSON line on stdout.
//!
//! The template loader is deliberately strict — `serde_yaml` is
//! configured to surface unknown fields as errors so a typo in
//! `runneer:` does not silently fall back to a default. Line/column
//! information from the parser is preserved verbatim in the error
//! message; the existing `codeless run | jq` idiom continues to work
//! because every successful invocation still prints one JSON object.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use codeless_rpc::{RpcServer, SubmitJobArgs};
use codeless_types::RepoId;
use serde::{Deserialize, Serialize};

use crate::rpc_open;

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// Submit a job whose shape is described by a YAML template
    /// (`{repo, runner, prompt, branch, stages, caps}`). The template
    /// text itself is forwarded to the runtime so the original
    /// description is recoverable from `jobs.template_yaml`.
    Submit(SubmitArgs),
}

#[derive(Debug, Args)]
pub struct SubmitArgs {
    /// Path to the YAML template.
    pub file: PathBuf,
}

/// Typed view of a job template. Fields are explicit and validated:
/// unknown YAML keys cause a parse error so silent fallbacks on a
/// misspelled key cannot mask configuration bugs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobTemplate {
    /// Repo id (ULID) the job runs against. Repos are addressed by
    /// id rather than name so the template is unambiguous even when
    /// two repos share a display name.
    pub repo: String,
    /// Runner kind. Matches the wire labels produced by the CLI's
    /// `--runner` enum: `mock`, `claude`, or `anthropic`.
    pub runner: String,
    /// Top-level user prompt forwarded to the runner. Optional so
    /// stage-only templates remain valid.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Branch the runner publishes work onto. Required even for the
    /// mock runner so submitted jobs always have an unambiguous
    /// target.
    pub branch: String,
    /// Ordered stage list. Stage rows aren't materialised here yet —
    /// the runtime persists the verbatim template, and Phase 3+ will
    /// expand it into the `stages` table. Validated for non-empty
    /// names so a typo doesn't silently produce an empty stage.
    #[serde(default)]
    pub stages: Vec<StageTemplate>,
    /// Caps. `cost_cents: 0` and `wall_clock_ms: 0` mean unlimited
    /// (matches the existing `submit_job` contract).
    #[serde(default)]
    pub caps: Caps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageTemplate {
    pub name: String,
    /// Optional verify command — runs as the post-stage gate.
    #[serde(default)]
    pub verify_cmd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Caps {
    #[serde(default)]
    pub cost_cents: i64,
    #[serde(default)]
    pub wall_clock_ms: i64,
}

pub fn handle(verb: Verb, db: Option<PathBuf>) -> Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(dispatch(verb, db))
}

async fn dispatch(verb: Verb, db: Option<PathBuf>) -> Result<ExitCode> {
    match verb {
        Verb::Submit(args) => submit(args, db).await,
    }
}

async fn submit(args: SubmitArgs, db: Option<PathBuf>) -> Result<ExitCode> {
    let yaml = std::fs::read_to_string(&args.file)
        .with_context(|| format!("read {}", args.file.display()))?;
    let template = parse_template(&yaml, &args.file)?;
    validate(&template)?;
    let repo_id = RepoId::from_str(&template.repo)
        .map_err(|e| anyhow!("invalid repo id {:?}: {e}", template.repo))?;

    let rpc = rpc_open::open(db.as_deref()).await?;
    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id,
            prompt: template.prompt.clone(),
            template_yaml: Some(yaml),
            runner: template.runner.clone(),
            branch: template.branch.clone(),
            cost_cap_cents: template.caps.cost_cents,
            wall_clock_cap_ms: template.caps.wall_clock_ms,
            model: None,
            permission_mode: None,
            effort: None,
        })
        .await
        .map_err(|e| anyhow!("submit_job: {e}"))?;
    println!("{}", serde_json::to_string(&job)?);
    Ok(ExitCode::SUCCESS)
}

fn parse_template(yaml: &str, file: &std::path::Path) -> Result<JobTemplate> {
    serde_yaml::from_str::<JobTemplate>(yaml).map_err(|e| anyhow!("parse {}: {e}", file.display()))
}

fn validate(template: &JobTemplate) -> Result<()> {
    if template.branch.trim().is_empty() {
        bail!("template branch must be a non-empty string");
    }
    for (i, stage) in template.stages.iter().enumerate() {
        if stage.name.trim().is_empty() {
            bail!("template stage #{i} has an empty name");
        }
    }
    Ok(())
}
