//! The `codeless` CLI binary. Local-mode invocations call
//! `codeless-runtime` in-process and skip auth (same trust boundary as
//! the invoking user); hosted-mode invocations
//! (`codeless --core https://… [--token …]`) use `codeless-client` and
//! authenticate via the bearer token from `~/.config/codeless/auth.toml`.
//!
//! Today only the `secrets` and `run` subcommands are wired.
//! Hosted-mode (`--core`, `--token`) and the `tail` / `session` verbs
//! land in later phases.

mod cost;
mod demo;
mod job;
mod jobs;
mod patches;
mod plugin;
mod repos;
mod review;
mod rpc_open;
mod run;
mod secrets;
mod serve;
mod tail;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

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

    /// SQLite file the local-mode runtime opens. When unset, each
    /// invocation runs against a fresh `:memory:` pool — useful for
    /// `run --once` style one-shots and the test suite, but useless
    /// for stateful subcommands like `review` since nothing persists
    /// between processes. Single-tenant: the same path is shared
    /// across all CLI invocations.
    #[arg(long, global = true, env = "CODELESS_DB")]
    db: Option<PathBuf>,

    /// Origin of a hosted `codeless-server`, e.g.
    /// `http://127.0.0.1:7777`. When set, supported subcommands use
    /// `codeless-client::HttpRpcClient` instead of the in-process
    /// runtime; `--db` is ignored. Pair with `--token` (or
    /// `CODELESS_TOKEN`) for authenticated cores.
    #[arg(long, global = true, env = "CODELESS_CORE")]
    core: Option<String>,

    /// Bearer token for hosted-mode RPC. Required when `--core` is
    /// set and the core has a `core_bearer_token` configured. The
    /// env var form keeps the token out of shell history.
    #[arg(long, global = true, env = "CODELESS_TOKEN")]
    token: Option<String>,

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
    /// Drive a review gate. Subcommands wrap the typed review RPC
    /// methods (list/approve/comment/stop); the global `--db` flag
    /// is required so successive invocations see each other.
    Review {
        #[command(subcommand)]
        verb: review::Verb,
    },
    /// Manage jobs by typed templates. `submit <file.yaml>` parses
    /// the YAML and calls `submit_job` against the local-mode
    /// runtime.
    Job {
        #[command(subcommand)]
        verb: job::Verb,
    },
    /// Stream the event log of a single job. Replays every persisted
    /// envelope and continues live until the job reaches a terminal
    /// state. Exit code is 0 on `job-completed`, non-zero on
    /// `job-failed` / `job-stopped`.
    Tail(tail::TailArgs),
    /// Run the hosted HTTP server (`codeless-server`). The browser
    /// and mobile shells reach the in-process runtime through this
    /// binary. `--init-token` generates the shared bearer token
    /// without starting the server.
    Serve(serve::ServeArgs),
    /// Manage repos. The `list` verb works in both local-mode
    /// (`--db <path>`) and hosted-mode (`--core URL --token T`) —
    /// it's the first dual-mode verb and exists primarily as the
    /// smoke test for the `HttpRpcClient` round-trip.
    Repos {
        #[command(subcommand)]
        verb: repos::Verb,
    },
    /// Inspect and stop jobs. Dual-mode like `repos`. For job
    /// submission see `codeless run` (one-shot) or `codeless job
    /// submit <file.yaml>` (YAML template).
    Jobs {
        #[command(subcommand)]
        verb: jobs::Verb,
    },
    /// Cost rollups across jobs. Reads `Job.cost_cents` via
    /// `list_jobs`; works against both local-mode and a hosted core.
    Cost {
        #[command(subcommand)]
        verb: cost::Verb,
    },
    /// Demo helpers: seed a fresh database with enough content that
    /// the browser UI has something to show on first paint.
    /// Local-mode only; pair with `codeless serve --db <same-path>`.
    Demo {
        #[command(subcommand)]
        verb: demo::Verb,
    },
    /// Walk the proposed scope-patch queue in
    /// `DOCS/SCOPE-PROPOSED.md`. `list` and `show` are read-only;
    /// `approve` / `reject` produce a human-authored git commit;
    /// `edit` opens the patch in `$EDITOR` and rewrites the queue.
    /// Step 6 of the SESSION-MUTABLE-SCOPE ramp.
    Patches {
        #[command(subcommand)]
        verb: patches::Verb,
    },
    /// Inspect statically-linked plugins (DOCS/PLUGIN-SUBSTRATE.md
    /// item 6). `list` enumerates plugins discovered under
    /// `--plugins-dir`; `info <id>` dumps the manifest plus the tools
    /// the plugin contributes to the shared registry.
    Plugin {
        #[command(subcommand)]
        verb: plugin::Verb,
    },
}

#[derive(Debug, Args)]
struct RunArgs {
    /// Local path to a checked-out repository.
    #[arg(long)]
    repo: PathBuf,
    /// Runner kind. `mock` stays the default so existing JSON-line
    /// consumers and tests that do not provision external resources
    /// keep working unchanged; `claude` shells out via
    /// `ClaudeRunnerAdapter` (CLI wrapper, requires the `claude`
    /// binary on PATH or `CLAUDE_BINARY`); `anthropic` talks to the
    /// REST API via `AnthropicRunnerAdapter` and reads the key from
    /// `ANTHROPIC_API_KEY` or `--api-key`.
    #[arg(long, value_enum, default_value_t = RunnerKind::Mock)]
    runner: RunnerKind,
    /// Override for the Anthropic API key. Falls back to the
    /// `ANTHROPIC_API_KEY` env var. Ignored for non-anthropic runners.
    #[arg(long)]
    api_key: Option<String>,
    /// Optional REST base URL override (for testing against a local
    /// fixture). Ignored for non-anthropic runners.
    #[arg(long)]
    base_url: Option<String>,
    /// Phase 1 only supports the `--once` shape (run a single job and
    /// exit). The flag is accepted to keep the documented invocation
    /// shape stable as daemon mode lands.
    #[arg(long, default_value_t = true)]
    once: bool,
    /// The user prompt to forward to the runner.
    prompt: String,
}

/// Runner adapter selectable from the CLI. The string form
/// (`as_wire()`) is what lands on the `Job.runner` column and
/// `Repo.default_runner` — keeping it stable means existing rows
/// continue to round-trip through `SubmitJobArgs` after future
/// additions.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum RunnerKind {
    Mock,
    Claude,
    Anthropic,
}

impl RunnerKind {
    pub(crate) fn as_wire(self) -> &'static str {
        match self {
            RunnerKind::Mock => "mock",
            RunnerKind::Claude => "claude",
            RunnerKind::Anthropic => "anthropic",
        }
    }
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
        Cmd::Run(args) => run::handle(args, cli.db),
        Cmd::Review { verb } => review::handle(verb, cli.core, cli.token, cli.db),
        Cmd::Job { verb } => job::handle(verb, cli.db),
        Cmd::Tail(args) => tail::handle(args, cli.core, cli.token, cli.db),
        Cmd::Serve(args) => {
            let secrets_path = resolve_secrets_path(cli.secrets_file)?;
            serve::handle(args, secrets_path, cli.db)
        }
        Cmd::Repos { verb } => repos::handle(verb, cli.core, cli.token, cli.db),
        Cmd::Jobs { verb } => jobs::handle(verb, cli.core, cli.token, cli.db),
        Cmd::Cost { verb } => cost::handle(verb, cli.core, cli.token, cli.db),
        Cmd::Demo { verb } => demo::handle(verb, cli.db),
        Cmd::Patches { verb } => patches::handle(verb),
        Cmd::Plugin { verb } => plugin::handle(verb),
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
