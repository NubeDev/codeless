//! `codeless demo <verb>` — seed helpers so a fresh checkout has
//! something to look at in the browser. Local-mode only: hits the
//! in-process runtime against `--db`. The convention is "run
//! `codeless demo bootstrap --db <path>` once, then point `codeless
//! serve --db <path>` at the same file."

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use codeless_rpc::{AddRepoArgs, ListReposResult, RpcServer, SubmitJobArgs};
use codeless_types::GitAuth;

use crate::rpc_open;

#[derive(Debug, Subcommand)]
pub enum Verb {
    /// Seed one repo + one queued mock job so the browser demo has
    /// content on first paint. Idempotent: a repo named "demo"
    /// already in the db short-circuits; nothing is duplicated.
    Bootstrap(BootstrapArgs),
}

#[derive(Debug, Args)]
pub struct BootstrapArgs {
    /// Display name for the seeded repo. The idempotency check
    /// matches on this string.
    #[arg(long, default_value = "demo")]
    pub name: String,

    /// Local path to record on the repo row. Defaults to the current
    /// working directory so the demo runs against a real checkout
    /// without the user thinking about paths.
    #[arg(long)]
    pub local_path: Option<PathBuf>,

    /// Clone URL recorded on the repo row. The mock runner never
    /// resolves it; a placeholder keeps the column non-null so real
    /// runners that probe the row do not panic.
    #[arg(long, default_value = "https://example.test/demo.git")]
    pub clone_url: String,

    /// Default branch on the repo row. When unset, the bootstrap
    /// probes `local_path` with `git symbolic-ref --short HEAD` so a
    /// repo initialised with `master` (or any non-`main` default)
    /// doesn't silently break later RPCs that diff against this name.
    /// Falls back to `main` only when probing fails (no `.git` dir,
    /// detached HEAD, missing git binary).
    #[arg(long)]
    pub default_branch: Option<String>,

    /// Prompt the seeded mock job carries. The mock runner echoes
    /// the prompt as an `AiMessageComplete` event — having something
    /// recognisable here makes the demo visibly real on the UI side.
    #[arg(long, default_value = "demo: list the files in this directory")]
    pub prompt: String,
}

pub fn handle(verb: Verb, db: Option<PathBuf>) -> Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(verb, db))
}

async fn run(verb: Verb, db: Option<PathBuf>) -> Result<ExitCode> {
    match verb {
        Verb::Bootstrap(args) => bootstrap(args, db).await,
    }
}

async fn bootstrap(args: BootstrapArgs, db: Option<PathBuf>) -> Result<ExitCode> {
    if db.is_none() {
        return Err(anyhow!(
            "`codeless demo bootstrap` needs --db (or CODELESS_DB); an in-memory \
             database would vanish between this command and `codeless serve`"
        ));
    }

    let rpc = rpc_open::open(db.as_deref()).await?;

    let listed: ListReposResult = rpc
        .list_repos()
        .await
        .map_err(|e| anyhow!("list_repos: {e}"))?;
    if let Some(existing) = listed.repos.iter().find(|r| r.name == args.name) {
        println!(
            "demo bootstrap: repo `{}` (id {}) already present; skipping seed",
            existing.name, existing.id
        );
        return Ok(ExitCode::SUCCESS);
    }

    let local_path = match args.local_path {
        Some(p) => p,
        None => std::env::current_dir()
            .map_err(|e| anyhow!("read current dir for default --local-path: {e}"))?,
    };

    let default_branch = args
        .default_branch
        .unwrap_or_else(|| detect_head_branch(&local_path).unwrap_or_else(|| "main".into()));

    let repo = rpc
        .add_repo(AddRepoArgs {
            name: args.name.clone(),
            clone_url: args.clone_url,
            default_branch,
            local_path: local_path.to_string_lossy().into_owned(),
            // Token-shaped auth keeps the wire shape live without
            // requiring a real secret; the mock runner never reads
            // it. Real runners would surface "missing env var" only
            // when actually invoked.
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
            prompt: Some(args.prompt),
            template_yaml: None,
            runner: "mock".into(),
            branch: format!("codeless/{}-demo", args.name),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            // CLI demo bootstrap is a "submit and run" command, not a
            // user-driven editing flow — keep the legacy behaviour of
            // queueing the job for immediate driver pickup.
            system_prompt: None,
            persona_id: None,
            start_immediately: true,
        })
        .await
        .map_err(|e| anyhow!("submit_job: {e}"))?;

    println!(
        "demo bootstrap: seeded repo `{}` (id {}) and queued mock job (id {})",
        repo.name, repo.id, job.id
    );
    Ok(ExitCode::SUCCESS)
}

/// Probe the actual HEAD branch of a checkout. Returns None when the
/// path is not a git repo, HEAD is detached, or git is unavailable —
/// the caller falls back to a static default.
fn detect_head_branch(local_path: &Path) -> Option<String> {
    let out = Command::new("git")
        .current_dir(local_path)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}
