//! The `codeless` CLI binary. Local-mode invocations call
//! `codeless-runtime` in-process and skip auth (same trust boundary as
//! the invoking user); hosted-mode invocations
//! (`codeless --core https://… [--token …]`) use `codeless-client` and
//! authenticate via the bearer token from `~/.config/codeless/auth.toml`.
//!
//! Today only the `secrets` and `run` subcommands are wired.
//! Hosted-mode (`--core`, `--token`) and the `tail` / `session` verbs
//! land in later phases.

mod run;
mod secrets;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "codeless",
    version,
    about = "Codeless CLI",
    propagate_version = true
)]
struct Cli {
    /// Override the path to `secrets.toml`. Defaults to the XDG-resolved
    /// `~/.config/codeless/secrets.toml` per SCOPE.md "Secrets store".
    #[arg(long, global = true, env = "CODELESS_SECRETS_FILE")]
    secrets_file: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Manage the secrets file. Names only are ever printed; values
    /// stay on disk and never reach logs.
    Secrets {
        #[command(subcommand)]
        verb: secrets::Verb,
    },
    /// Run a single job end-to-end against a chosen runner, streaming
    /// events to stdout as JSON lines. Exit code is 0 on
    /// `job-completed`, non-zero on `job-failed`.
    Run(RunArgs),
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Local path to a checked-out repository.
    #[arg(long)]
    repo: PathBuf,
    /// Runner kind. Only `mock` is wired in Phase 1.
    #[arg(long, default_value = "mock")]
    runner: String,
    /// Phase 1 only supports the `--once` shape (run a single job and
    /// exit). The flag is accepted to keep the documented invocation
    /// shape stable as daemon mode lands.
    #[arg(long, default_value_t = true)]
    once: bool,
    /// The user prompt to forward to the runner.
    prompt: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Secrets { verb } => {
            let path = resolve_secrets_path(cli.secrets_file)?;
            secrets::handle(verb, &path)?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Run(args) => run::handle(args),
    }
}

fn resolve_secrets_path(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p);
    }
    let home =
        std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set; pass --secrets-file"))?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("codeless")
        .join("secrets.toml"))
}
