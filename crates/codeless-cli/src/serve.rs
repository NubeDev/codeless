//! `codeless serve` — bind the hosted axum server from the inner
//! `codeless-server` crate, read the bearer token from the shared
//! secrets file, and run until SIGINT. `--init-token` is a side
//! channel that generates a token, persists it, prints it once, and
//! exits without starting the server.
//!
//! The same secrets path the rest of the CLI uses (`--secrets-file`
//! or XDG default) backs `core_bearer_token`. Single-tenant per
//! SCOPE.md R5: one token for browser + future mobile clients.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use codeless_adapters_host::SecretStore;
use codeless_rpc::RpcServer;
use codeless_runtime::{
    spawn_job_driver_loop, InProcessRpc, MockRunner, MockStep, Runner, RunnerFactory, RunnerOutcome,
};
use codeless_server::{
    load_bearer_token, serve_with_shutdown, AppState, TokenLoadError, TOKEN_SECRET_KEY,
};

use crate::rpc_open;

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to bind. Defaults to `127.0.0.1:7777` — the demo
    /// quickstart documents that port; if a hosted deployment needs
    /// a public bind, supply `0.0.0.0:<port>` explicitly so the
    /// loopback default never accidentally exposes the core.
    #[arg(long, default_value = "127.0.0.1:7777")]
    pub bind: SocketAddr,

    /// Generate a random bearer token, write it to the secrets file
    /// under `core_bearer_token`, print it once on stdout, and exit
    /// without starting the server. Pair with `--force` to rotate an
    /// existing token; the previous token is invalidated.
    #[arg(long)]
    pub init_token: bool,

    /// Rotate an existing token rather than refusing. Only meaningful
    /// with `--init-token`.
    #[arg(long, requires = "init_token")]
    pub force: bool,

    /// Disable the background job driver. Without it, jobs submitted
    /// over RPC stay `Queued` forever — useful when the CLI's
    /// `codeless run` is the canonical driver and the server is just
    /// a read-only surface.
    #[arg(long)]
    pub no_driver: bool,

    /// Max concurrent jobs the background driver runs. Ignored when
    /// `--no-driver` is set.
    #[arg(long, default_value_t = 4)]
    pub driver_concurrency: usize,
}

pub fn handle(args: ServeArgs, secrets_path: PathBuf, db: Option<PathBuf>) -> Result<ExitCode> {
    if args.init_token {
        init_token(&secrets_path, args.force)?;
        return Ok(ExitCode::SUCCESS);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_server(args, secrets_path, db))
}

fn init_token(path: &Path, force: bool) -> Result<()> {
    let mut store = SecretStore::open(path)
        .with_context(|| format!("open secrets file at {}", path.display()))?;
    if store.get(TOKEN_SECRET_KEY).is_some() && !force {
        bail!(
            "bearer token already configured in {}; pass --force to rotate it",
            path.display()
        );
    }
    let token = random_hex_token(16);
    store.set(TOKEN_SECRET_KEY, token.clone())?;
    store.save()?;
    println!("{token}");
    Ok(())
}

async fn run_server(
    args: ServeArgs,
    secrets_path: PathBuf,
    db: Option<PathBuf>,
) -> Result<ExitCode> {
    init_tracing();

    let store = SecretStore::open(&secrets_path)
        .with_context(|| format!("open secrets at {}", secrets_path.display()))?;
    let token = load_bearer_token(&store).map_err(|TokenLoadError::Missing| {
        anyhow!(
            "no `{TOKEN_SECRET_KEY}` in {}; run `codeless serve --init-token` first",
            secrets_path.display()
        )
    })?;

    let rpc: Arc<InProcessRpc> = Arc::new(rpc_open::open(db.as_deref()).await?);
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    let state = AppState::new(rpc_dyn, token);

    // Background driver: pick up every `Queued` job and run it
    // through the in-process runtime. Default factory only enables
    // `mock`; real runners need the operator to opt in once
    // production driver scope (worktree provisioning, secrets-backed
    // API keys) catches up. The handle stays alive in this scope so
    // the spawned task is not dropped before `serve_with_shutdown`
    // returns; we don't join it because process exit on Ctrl-C is
    // the supported teardown path.
    let _driver = if args.no_driver {
        eprintln!("codeless-server: background driver disabled (--no-driver)");
        None
    } else {
        eprintln!(
            "codeless-server: background driver enabled (concurrency={}, runners=mock)",
            args.driver_concurrency
        );
        let factory = Arc::new(DefaultRunnerFactory::new());
        Some(
            spawn_job_driver_loop(rpc.clone(), factory, args.driver_concurrency)
                .await
                .map_err(|e| anyhow!("driver init: {e}"))?,
        )
    };

    serve_with_shutdown(args.bind, state, |addr| {
        // Stderr is unbuffered, so integration tests reading line-by-
        // line see this before the server starts accepting requests.
        eprintln!("codeless-server listening on http://{addr}");
    })
    .await
    .map_err(|e| anyhow!("serve: {e}"))?;

    Ok(ExitCode::SUCCESS)
}

/// Built-in runner factory. Always enables `mock` so the demo works
/// without external dependencies; `claude` and `anthropic` need
/// configuration the operator hasn't given us a way to provide yet
/// (worktree path resolution, anthropic key wiring), so we refuse
/// them rather than silently failing a queued job. A submitted job
/// with an unsupported runner stays `Queued` and a warning lands
/// in the server log — the operator can stop it via the CLI.
struct DefaultRunnerFactory;

impl DefaultRunnerFactory {
    fn new() -> Self {
        Self
    }
}

impl RunnerFactory for DefaultRunnerFactory {
    fn build(&self, runner_name: &str) -> Option<Arc<dyn Runner>> {
        match runner_name {
            "mock" => Some(Arc::new(MockRunner::new(vec![MockStep::Finish(
                RunnerOutcome::Completed,
            )]))),
            _ => None,
        }
    }
}

/// Set up the `tracing-subscriber` for the running server. Reads
/// `RUST_LOG` for the env-filter directive (defaults to
/// `info,tower_http=info`) so operators can dial into specific
/// targets without recompiling. Idempotent — set_global_default only
/// succeeds once per process, so a second `codeless serve` invocation
/// inside one process (the integration test path) does not panic.
fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .try_init();
}

/// 16 random bytes encoded as 32 lowercase hex chars — 128 bits of
/// entropy, short enough to copy-paste into a browser dialog. Sources
/// from the OS CSPRNG via `getrandom`; tying this to `ulid::Ulid`
/// would leak the generation timestamp in the high 48 bits.
fn random_hex_token(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    getrandom::getrandom(&mut buf).expect("OS CSPRNG unavailable");
    let mut out = String::with_capacity(n_bytes * 2);
    for b in buf {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
