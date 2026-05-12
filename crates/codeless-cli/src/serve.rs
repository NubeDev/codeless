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
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use codeless_adapters_host::{SecretStore, WorktreeManager};
use codeless_rpc::{ClaudeStatus, RpcServer, RunnerInfo, ServerInfo};
use codeless_runtime::{
    spawn_job_driver_loop, spawn_notifier, AnthropicRunnerAdapter, ClaudeRunnerAdapter,
    InProcessRpc, MockRunner, MockStep, Runner, RunnerFactory, RunnerOutcome, WebhookConfig,
    WebhookNotifier,
};
use codeless_server::{
    load_bearer_token, serve_with_shutdown, AppState, AuthMode, TokenLoadError, TOKEN_SECRET_KEY,
};
use codeless_types::{CostCents, Event, Job, TaskId, TaskStatus};

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

    /// Directory under which per-job `git worktree` checkouts live.
    /// Each job gets `<worktree-root>/job-<job_id>` on a fresh
    /// `codeless/job-<job_id>` branch. When unset but `--fs-root` is
    /// set, defaults to `<fs-root>/.codeless/worktrees` — so the demo
    /// quickstart does not require the operator to invent a second
    /// flag. When both are unset, the driver runs jobs without
    /// provisioning a worktree (fine for `mock`; real AI runners that
    /// need a checkout will fail at run time).
    #[arg(long, env = "CODELESS_WORKTREE_ROOT")]
    pub worktree_root: Option<PathBuf>,

    /// Enable the `claude` runner in the background driver. Requires
    /// the `claude` binary on `PATH` or `CLAUDE_BINARY` env. Off by
    /// default because the binary may not be installed; the operator
    /// opts in once they've provisioned it.
    #[arg(long)]
    pub enable_claude: bool,

    /// Enable the `anthropic` runner in the background driver. Reads
    /// the API key from the secrets file under `anthropic_api_key`;
    /// missing keys are surfaced at runner-build time so the operator
    /// sees a clear error in the server log.
    #[arg(long)]
    pub enable_anthropic: bool,

    /// Root directory the `fs.*` RPC surface is allowed to read and
    /// write under. When unset, `fs_*` methods return `Internal` —
    /// useful for hosted deployments that want the runtime up before
    /// the editor surfaces are wired. The host adapter rejects any
    /// path that escapes this root after canonicalisation.
    #[arg(long, env = "CODELESS_FS_ROOT")]
    pub fs_root: Option<PathBuf>,

    /// Force bearer-token authentication even on loopback binds.
    /// Loopback is unauthenticated by default because the trust
    /// boundary is already the same-user same-host process; this
    /// flag exists for operators who want to enforce the token
    /// flow during local development or testing. Non-loopback
    /// binds (anything other than 127.0.0.1 / ::1) require this
    /// flag — the server refuses to boot otherwise so a careless
    /// `--bind 0.0.0.0:...` cannot accidentally expose an
    /// unauthenticated core.
    #[arg(long)]
    pub require_token: bool,
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

    let is_loopback = args.bind.ip().is_loopback();
    let auth_required = args.require_token || !is_loopback;
    if !is_loopback && !args.require_token {
        bail!(
            "refusing to bind {} without --require-token; a non-loopback bind \
             without auth would expose an unauthenticated core. Pass \
             --require-token (and run `codeless serve --init-token` first to \
             mint one) or bind to a loopback address.",
            args.bind
        );
    }

    let auth = if auth_required {
        let token = load_bearer_token(&store).map_err(|TokenLoadError::Missing| {
            anyhow!(
                "no `{TOKEN_SECRET_KEY}` in {}; run `codeless serve --init-token` first",
                secrets_path.display()
            )
        })?;
        AuthMode::required(token)
    } else {
        eprintln!(
            "codeless-server: loopback bind {}, auth disabled (pass --require-token to enforce)",
            args.bind
        );
        AuthMode::Open
    };

    let mut runtime = rpc_open::open(db.as_deref()).await?;
    if let Some(root) = &args.fs_root {
        let host_fs = codeless_adapters_host::HostFs::new(root)
            .map_err(|e| anyhow!("fs root {}: {e}", root.display()))?;
        runtime = runtime.with_fs(Arc::new(host_fs));
        eprintln!("codeless-server: fs root = {}", root.display());
    }
    let rpc: Arc<InProcessRpc> = Arc::new(runtime);
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    let claude_status = if args.enable_claude {
        let status = codeless_adapters_host::probe_claude().await;
        match &status {
            None => eprintln!(
                "codeless-server: claude runner enabled but the `claude` binary \
                 was not found on PATH, in CLAUDE_BINARY, or in any known install \
                 location. Jobs targeting it will fail; install Claude Code or \
                 unset --enable-claude."
            ),
            Some(s) => eprintln!(
                "codeless-server: claude detected at {} (version={}, auth={})",
                s.binary_path,
                s.version.as_deref().unwrap_or("unknown"),
                match s.authenticated {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "unknown",
                }
            ),
        }
        status
    } else {
        None
    };
    let worktree_root_effective = effective_worktree_root(&args);
    let server_info = Arc::new(build_server_info(
        &args,
        worktree_root_effective.clone(),
        claude_status,
    ));
    let state = AppState {
        rpc: rpc_dyn,
        auth,
        server_info,
    };

    // Outbound webhook notifier: when both `notifier_webhook_url`
    // and `notifier_webhook_hmac_key_hex` are present in the secrets
    // file, attach a `WebhookNotifier` to the event bus. The
    // notifier fires HMAC-signed POSTs on `JobFailed` and
    // `ReviewRequested` (see `codeless_runtime::notifier`). Missing
    // keys → silently skipped; partial config (one key without the
    // other) is a configuration error and refuses to boot.
    let _notifier = match maybe_webhook_config(&store)? {
        None => None,
        Some(cfg) => {
            let notifier = WebhookNotifier::from_config(cfg.clone())
                .map_err(|e| anyhow!("webhook notifier: {e}"))?;
            eprintln!(
                "codeless-server: webhook notifier configured (url={})",
                cfg.url
            );
            Some(
                spawn_notifier(rpc.bus().clone(), Arc::new(notifier))
                    .await
                    .map_err(|e| anyhow!("spawn_notifier: {e}"))?,
            )
        }
    };

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
        let worktrees = worktree_root_effective.as_ref().map(|root| {
            // Create the parent eagerly so the first job doesn't
            // race the directory creation with the `git worktree
            // add` call.
            if let Err(e) = std::fs::create_dir_all(root) {
                tracing::warn!(
                    error = %e,
                    root = %root.display(),
                    "could not create worktree root; provisioning will fail per job",
                );
            }
            Arc::new(WorktreeManager::new(root))
        });
        let mut enabled = vec!["mock"];
        if args.enable_claude {
            enabled.push("claude");
        }
        if args.enable_anthropic {
            enabled.push("anthropic");
        }
        eprintln!(
            "codeless-server: background driver enabled (concurrency={}, runners={}, worktrees={})",
            args.driver_concurrency,
            enabled.join(","),
            worktree_root_effective
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "disabled".into()),
        );
        let anthropic_api_key = store.get("anthropic_api_key").map(str::to_owned);
        let claude_system_prompt = store.get("claude_system_prompt").map(str::to_owned);
        let factory = Arc::new(DefaultRunnerFactory {
            enable_claude: args.enable_claude,
            enable_anthropic: args.enable_anthropic,
            anthropic_api_key,
            claude_system_prompt,
        });
        Some(
            spawn_job_driver_loop(rpc.clone(), factory, worktrees, args.driver_concurrency)
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

/// Resolve the effective `--worktree-root`. When the operator passes
/// `--worktree-root` explicitly it wins; otherwise `--fs-root` (when
/// set) seeds an implicit `<fs-root>/.codeless/worktrees` so the demo
/// quickstart works without a second flag. The directory is created
/// lazily at driver init.
fn effective_worktree_root(args: &ServeArgs) -> Option<PathBuf> {
    if let Some(explicit) = &args.worktree_root {
        return Some(explicit.clone());
    }
    args.fs_root
        .as_ref()
        .map(|root| root.join(".codeless").join("worktrees"))
}

/// Build the `/server/info` snapshot from the parsed `serve` flags.
/// The runner list mirrors `DefaultRunnerFactory`'s enabled set so the
/// UI dropdown reflects exactly what the driver will accept. The
/// default flag prefers real runners over `mock` when at least one is
/// enabled, so a `--enable-claude` server does not silently default
/// new jobs to the demo path; with only `mock`, the demo runner stays
/// the default.
fn build_server_info(
    args: &ServeArgs,
    worktree_root: Option<PathBuf>,
    claude: Option<ClaudeStatus>,
) -> ServerInfo {
    let mut runners = Vec::new();
    let real_runner_enabled = args.enable_claude || args.enable_anthropic;
    runners.push(RunnerInfo {
        id: "mock".to_owned(),
        default: !real_runner_enabled,
    });
    if args.enable_claude {
        runners.push(RunnerInfo {
            id: "claude".to_owned(),
            default: true,
        });
    }
    if args.enable_anthropic {
        runners.push(RunnerInfo {
            id: "anthropic".to_owned(),
            // `claude` wins the default when both flags are passed; the
            // anthropic REST runner is the secondary path.
            default: !args.enable_claude,
        });
    }
    ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        runners,
        fs_root: args.fs_root.as_ref().map(|p| p.display().to_string()),
        worktree_root: worktree_root.as_ref().map(|p| p.display().to_string()),
        claude,
    }
}

/// Pull webhook-notifier config out of the secrets file. Two flat
/// keys (no nested TOML table because `SecretStore` is a flat
/// key-value map): `notifier_webhook_url` and
/// `notifier_webhook_hmac_key_hex`. Both must be present or both
/// absent; a partial config is rejected so the operator finds the
/// typo at boot instead of wondering why webhooks never fire.
fn maybe_webhook_config(store: &SecretStore) -> Result<Option<WebhookConfig>> {
    let url = store.get("notifier_webhook_url");
    let key = store.get("notifier_webhook_hmac_key_hex");
    match (url, key) {
        (None, None) => Ok(None),
        (Some(url), Some(hmac_key_hex)) => Ok(Some(WebhookConfig {
            url: url.to_string(),
            hmac_key_hex: hmac_key_hex.to_string(),
        })),
        _ => bail!(
            "partial webhook config: `notifier_webhook_url` and \
             `notifier_webhook_hmac_key_hex` must both be set or both absent",
        ),
    }
}

/// Built-in runner factory. `mock` is always on so the demo works
/// without external dependencies; `claude` and `anthropic` are
/// opt-in via `--enable-claude` / `--enable-anthropic` because each
/// needs configuration the operator must provide (the `claude`
/// binary on PATH, the Anthropic API key in the secrets file). An
/// opt-in runner with missing config (e.g. anthropic without
/// `anthropic_api_key`) still builds — the runner adapter surfaces
/// the auth failure at run time as `RunnerOutcome::Failed`, which
/// the driver maps to `Event::JobFailed`. That's more useful than
/// silently leaving the job `Queued` forever.
struct DefaultRunnerFactory {
    enable_claude: bool,
    enable_anthropic: bool,
    anthropic_api_key: Option<String>,
    /// Optional override for the claude headless system prompt. When
    /// the secrets file carries `claude_system_prompt`, it replaces
    /// the built-in default in `ClaudeRunnerAdapter::DEFAULT_SYSTEM_PROMPT`.
    /// An empty string disables the prompt entirely.
    claude_system_prompt: Option<String>,
}

/// Build a `MockRunner` script that emits enough events to be visibly
/// alive in the UI's JobTimeline. Real AI runners drive these same
/// event variants through `ctx.bus`; the mock script mirrors that
/// shape so the dashboard renders the same way for both. The
/// `FAIL` prompt is a sentinel for tests that need the failure path
/// without provisioning a real runner; everything else completes.
fn demo_mock_script(prompt: &str) -> Vec<MockStep> {
    if prompt == "FAIL" {
        return vec![MockStep::Finish(RunnerOutcome::Failed {
            reason: "mock runner: FAIL sentinel".into(),
        })];
    }

    let task_id = TaskId::new();
    let echo = if prompt.is_empty() {
        "demo: mock runner ran end-to-end".to_owned()
    } else {
        format!("mock-echo: {prompt}")
    };
    let mut steps = Vec::new();
    steps.push(MockStep::Emit(Event::TaskStarted { task_id }));
    // Split into a few token chunks so the timeline's `ai-token`
    // coalescing path has something to render incrementally, then
    // close with the message-complete envelope the dashboard reads
    // for cost rollups (zeros here; real runners populate them).
    for chunk in chunk_for_stream(&echo) {
        steps.push(MockStep::Emit(Event::AiToken {
            task_id,
            delta: chunk,
        }));
        steps.push(MockStep::Sleep(Duration::from_millis(120)));
    }
    steps.push(MockStep::Emit(Event::AiMessageComplete {
        task_id,
        input_tokens: 0,
        output_tokens: 0,
        cost_cents: CostCents(0),
    }));
    steps.push(MockStep::Emit(Event::TaskCompleted {
        task_id,
        status: TaskStatus::Completed,
    }));
    steps.push(MockStep::Finish(RunnerOutcome::Completed));
    steps
}

fn chunk_for_stream(s: &str) -> Vec<String> {
    // Word-sized chunks keep the demo lively without flooding the
    // event log. Empty strings between words would be valid but
    // produce no visible token in the UI, so they are skipped.
    s.split_inclusive(' ')
        .filter(|w| !w.trim().is_empty())
        .map(|w| w.to_owned())
        .collect()
}

impl RunnerFactory for DefaultRunnerFactory {
    fn build(&self, job: &Job) -> Option<Arc<dyn Runner>> {
        // `prompt` is documented as Optional on `SubmitJobArgs`; a
        // missing prompt is most likely a YAML-template job whose
        // first stage holds the real prompt. Until template-driven
        // runs land on the server-driver path, fall back to an
        // empty string so the AI adapters don't panic.
        let prompt = job.prompt.clone().unwrap_or_default();
        match job.runner.as_str() {
            "mock" => Some(Arc::new(MockRunner::new(demo_mock_script(&prompt)))),
            "claude" if self.enable_claude => {
                let mut adapter = ClaudeRunnerAdapter::new(prompt, TaskId::new());
                if let Some(sp) = &self.claude_system_prompt {
                    adapter = adapter.with_system_prompt(sp);
                }
                Some(Arc::new(adapter))
            }
            "anthropic" if self.enable_anthropic => {
                let mut adapter = AnthropicRunnerAdapter::new(prompt, TaskId::new());
                adapter.api_key = self.anthropic_api_key.clone();
                Some(Arc::new(adapter))
            }
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
