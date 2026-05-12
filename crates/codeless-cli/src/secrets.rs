use std::io::{self, IsTerminal, Read};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use codeless_adapters_host::SecretStore;

#[derive(Debug, Subcommand)]
pub enum Verb {
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
pub struct SetArgs {
    pub key: String,
    /// Inline value. Mutually exclusive with `--from-env`.
    pub value: Option<String>,
    /// Read the value from this env var, e.g. `--from-env GITHUB_TOKEN`.
    #[arg(long)]
    pub from_env: Option<String>,
}

#[derive(Debug, Args)]
pub struct GetArgs {
    pub key: String,
    /// Acknowledge that the value will be printed to stdout.
    #[arg(long)]
    pub reveal: bool,
}

#[derive(Debug, Args)]
pub struct RmArgs {
    pub key: String,
}

pub fn handle(verb: Verb, path: &Path) -> Result<()> {
    let mut store = SecretStore::open(path)
        .with_context(|| format!("open secrets file at {}", path.display()))?;
    match verb {
        Verb::List => {
            for name in store.list() {
                println!("{name}");
            }
        }
        Verb::Get(args) => {
            let value = store
                .get(&args.key)
                .ok_or_else(|| anyhow!("no such secret: {}", args.key))?
                .to_string();
            if !args.reveal {
                bail!("refusing to print secret without --reveal");
            }
            println!("{value}");
        }
        Verb::Set(args) => {
            let value = resolve_set_value(&args)?;
            store.set(&args.key, value)?;
            store.save()?;
        }
        Verb::Rm(args) => {
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
