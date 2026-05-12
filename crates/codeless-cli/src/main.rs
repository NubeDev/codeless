//! The `codeless` CLI binary. Local-mode invocations call
//! `codeless-runtime` in-process and skip auth (same trust boundary as
//! the invoking user); hosted-mode invocations
//! (`codeless --core https://… [--token …]`) use `codeless-client` and
//! authenticate via the bearer token from `~/.config/codeless/auth.toml`.
//!
//! Today only the `secrets` subcommand is wired (SCOPE.md Phase 1).
//! The `run`, `tail`, `session` verbs land alongside the runtime
//! end-to-end stage.

use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use codeless_adapters_host::SecretStore;

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
        verb: SecretsVerb,
    },
}

#[derive(Debug, Subcommand)]
enum SecretsVerb {
    /// Set `KEY` to `VALUE`. With neither `value` nor `--from-env`,
    /// read the value from stdin (so secrets never appear in shell
    /// history).
    Set(SetArgs),
    /// Print the value for `KEY`. Refuses to print without `--reveal`
    /// so accidental `codeless secrets get` invocations are safe.
    Get(GetArgs),
    /// Remove `KEY`. Errors if the key is unknown.
    Rm(RmArgs),
    /// List secret names (never values).
    List,
}

#[derive(Debug, Args)]
struct SetArgs {
    key: String,
    /// Inline value. Mutually exclusive with `--from-env`.
    value: Option<String>,
    /// Read the value from this env var, e.g. `--from-env GITHUB_TOKEN`.
    #[arg(long)]
    from_env: Option<String>,
}

#[derive(Debug, Args)]
struct GetArgs {
    key: String,
    /// Acknowledge that the value will be printed to stdout.
    #[arg(long)]
    reveal: bool,
}

#[derive(Debug, Args)]
struct RmArgs {
    key: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let path = resolve_secrets_path(cli.secrets_file)?;
    match cli.cmd {
        Cmd::Secrets { verb } => handle_secrets(verb, &path),
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

fn handle_secrets(verb: SecretsVerb, path: &std::path::Path) -> Result<()> {
    let mut store = SecretStore::open(path)
        .with_context(|| format!("open secrets file at {}", path.display()))?;
    match verb {
        SecretsVerb::List => {
            for name in store.list() {
                println!("{name}");
            }
        }
        SecretsVerb::Get(args) => {
            let value = store
                .get(&args.key)
                .ok_or_else(|| anyhow!("no such secret: {}", args.key))?
                .to_string();
            if !args.reveal {
                bail!("refusing to print secret without --reveal");
            }
            println!("{value}");
        }
        SecretsVerb::Set(args) => {
            let value = resolve_set_value(&args)?;
            store.set(&args.key, value)?;
            store.save()?;
        }
        SecretsVerb::Rm(args) => {
            store.remove(&args.key)?;
            store.save()?;
        }
    }
    Ok(())
}

fn resolve_set_value(args: &SetArgs) -> Result<String> {
    if let Some(ref v) = args.value {
        if args.from_env.is_some() {
            bail!("--from-env conflicts with positional value");
        }
        return Ok(v.clone());
    }
    if let Some(ref env) = args.from_env {
        return std::env::var(env).with_context(|| format!("env var {env} is not set"));
    }
    let stdin = io::stdin();
    if stdin.is_terminal() {
        bail!("no value supplied; pass `value`, `--from-env NAME`, or pipe via stdin");
    }
    let mut buf = String::new();
    stdin.lock().read_to_string(&mut buf)?;
    // Strip a single trailing newline so the common
    // `printf '%s\n' "$v" | codeless secrets set k` shell idiom does
    // not store the trailing `\n`. Anything else is preserved verbatim.
    if buf.ends_with('\n') {
        buf.pop();
        if buf.ends_with('\r') {
            buf.pop();
        }
    }
    Ok(buf)
}
