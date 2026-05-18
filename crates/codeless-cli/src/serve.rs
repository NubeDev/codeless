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
use codeless_adapters_host::{SecretStore, WorktreeManager};
use codeless_rpc::{ClaudeStatus, RpcServer, RunnerInfo, ServerFeatureFlags, ServerInfo};
use codeless_runtime::{
    spawn_job_driver_loop, spawn_notifier, spawn_plan_engine_subscriber, spawn_stage_recorder,
    DefaultRunnerFactory, InProcessRpc, WebhookConfig, WebhookNotifier,
};
use codeless_server::{
    load_bearer_token, serve_with_shutdown, AppState, AuthMode, TokenLoadError, TOKEN_SECRET_KEY,
};
use codeless_tools::plan::{LogJobSpawner, PlanEngine};

use crate::rpc_open;

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to bind. Defaults to `127.0.0.1:0` which lets the OS
    /// pick a free port — multiple sessions can run in parallel
    /// without conflicts. The chosen port is printed to stderr on
    /// startup. Pin a specific port with e.g. `--bind 127.0.0.1:7777`
    /// when a stable URL is needed (reverse proxy, bookmarks). If a
    /// hosted deployment needs a public bind, supply
    /// `0.0.0.0:<port>` explicitly so the loopback default never
    /// accidentally exposes the core.
    #[arg(long, default_value = "127.0.0.1:0")]
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

    /// Enable the `codex` runner. Requires the `codex` CLI on `PATH`
    /// (`npm install -g @openai/codex`) and `OPENAI_API_KEY` in the
    /// server's environment — the upstream binary reads the key
    /// directly.
    #[arg(long)]
    pub enable_codex: bool,

    /// Enable the `copilot` runner. Requires the `copilot` CLI on
    /// `PATH` with a completed GitHub device-flow login (state under
    /// `~/.copilot/`).
    #[arg(long)]
    pub enable_copilot: bool,

    /// Root directory the `fs.*` RPC surface is allowed to read and
    /// write under. When unset, `fs_*` methods return `Internal` —
    /// useful for hosted deployments that want the runtime up before
    /// the editor surfaces are wired. The host adapter rejects any
    /// path that escapes this root after canonicalisation.
    #[arg(long, env = "CODELESS_FS_ROOT")]
    pub fs_root: Option<PathBuf>,

    /// Enable the Slack control-plane adapter. When set, the server
    /// opens a Slack Socket Mode connection at boot using the
    /// `slack_app_token` + `slack_bot_token` keys from the secrets
    /// file. Stage 2 of the slack-integration job lands only the
    /// transport: later stages wire the command parser and the
    /// outbound failure notifications. Missing-token errors are
    /// surfaced as a warning so the rest of the server still boots —
    /// the bot is additive, not load-bearing for the runtime.
    #[arg(long)]
    pub enable_slack: bool,

    /// Enable the Telegram control-plane adapter. When set, the
    /// server opens a Bot API long-poll loop at boot using the
    /// `telegram_bot_token` key from the secrets file (and the
    /// optional `telegram_chat_id` key for outbound failure cards).
    /// Missing-token errors are surfaced as a warning so the rest of
    /// the server still boots — the bot is additive, not
    /// load-bearing for the runtime.
    #[arg(long)]
    pub enable_telegram: bool,

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

    /// Write the bound address (host:port) to this file once the
    /// listener is ready. Useful for scripts and orchestrators that
    /// need to discover the auto-selected port programmatically.
    /// The file is overwritten on each boot and deleted on shutdown.
    #[arg(long, env = "CODELESS_PORT_FILE")]
    pub port_file: Option<PathBuf>,
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

    // Adapter registry boot wiring (see DOCS/WORKSPACE-ATTACH.md "TODO
    // — adapter registry"). Each `--enable-*` flag upserts a single
    // row keyed on `(kind, "default")` for chat adapters or `runner_id`
    // for runners, exactly the way `--fs-root` upserts
    // `attached_workspaces`. After the upserts run we read the table
    // back: the rows are now the source of truth, so a boot with no
    // flags still picks up the set the UI last persisted, and a flag
    // re-passed on top wins over whatever was stored.
    if args.enable_slack {
        if let Err(e) = codeless_runtime::adapter_registry::upsert_chat_adapter(
            runtime.pool(),
            "slack",
            codeless_runtime::adapter_registry::DEFAULT_INSTANCE_ID,
            true,
        )
        .await
        {
            eprintln!("codeless-server: chat_adapters upsert for slack failed: {e}");
        }
    }
    if args.enable_telegram {
        if let Err(e) = codeless_runtime::adapter_registry::upsert_chat_adapter(
            runtime.pool(),
            "telegram",
            codeless_runtime::adapter_registry::DEFAULT_INSTANCE_ID,
            true,
        )
        .await
        {
            eprintln!("codeless-server: chat_adapters upsert for telegram failed: {e}");
        }
    }
    for (id, enabled) in [
        ("claude", args.enable_claude),
        ("anthropic", args.enable_anthropic),
        ("codex", args.enable_codex),
        ("copilot", args.enable_copilot),
    ] {
        if !enabled {
            continue;
        }
        if let Err(e) =
            codeless_runtime::adapter_registry::upsert_runner(runtime.pool(), id, true).await
        {
            eprintln!("codeless-server: runner_config upsert for {id} failed: {e}");
        }
    }
    let effective = codeless_runtime::adapter_registry::load_effective(runtime.pool())
        .await
        .unwrap_or_else(|e| {
            eprintln!("codeless-server: adapter registry read failed, falling back to CLI flags: {e}");
            codeless_runtime::adapter_registry::EffectiveAdapterRegistry {
                slack_enabled: args.enable_slack,
                telegram_enabled: args.enable_telegram,
                claude_enabled: args.enable_claude,
                anthropic_enabled: args.enable_anthropic,
                codex_enabled: args.enable_codex,
                copilot_enabled: args.enable_copilot,
            }
        });
    // Resolve the effective worktree root up front so the HostFs
    // adapter can grant read access to per-job `runs/*/handover.md`
    // and notes through the same `fs_*` surface. Without this, the
    // UI's Handover pane errors with `path escapes root` because the
    // worktree root sits outside `--fs-root` (the source repo).
    let worktree_root_effective = effective_worktree_root(&args);
    let mut host_fs_for_liveness: Option<Arc<codeless_adapters_host::HostFs>> = None;
    if let Some(root) = &args.fs_root {
        // WORKSPACE-ATTACH milestone 2 — `--fs-root` is now a
        // bootstrap convenience: canonicalise the path and upsert it
        // into `attached_workspaces`. Repeated boots with `/a/b`,
        // `/a/b/`, or a symlink resolving to `/a/b` all collapse to
        // the one row keyed on canonical (see migration 0007).
        match codeless_runtime::attached_workspaces::upsert_boot_workspace(
            runtime.pool(),
            root,
        )
        .await
        {
            Ok(outcome) => eprintln!(
                "codeless-server: attached_workspaces upsert ok (canonical={}, created_repo={}, created_attachment={})",
                outcome.canonical, outcome.created_repo, outcome.created_attachment,
            ),
            Err(e) => eprintln!(
                "codeless-server: attached_workspaces upsert for {} failed: {e}",
                root.display(),
            ),
        }
        let host_fs = codeless_adapters_host::HostFs::new(root)
            .map_err(|e| anyhow!("fs root {}: {e}", root.display()))?;
        // Rehydrate every other `attached_workspaces` row into the
        // adapter so previously-attached workspaces remain reachable
        // through `fs.*` across a restart without an explicit
        // re-attach. The bootstrap `--fs-root` already lives in
        // `host_fs.roots()` as the first entry; canonical equality is
        // how `add_root` collapses duplicates.
        match codeless_runtime::attached_workspaces::list_canonical_roots(runtime.pool()).await {
            Ok(rows) => {
                for canonical in rows {
                    if let Err(e) = host_fs.add_root(std::path::PathBuf::from(&canonical)) {
                        tracing::warn!(
                            error = %e,
                            path = %canonical,
                            "skipping stale attached_workspaces row at boot",
                        );
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not rehydrate attached_workspaces"),
        }
        if let Some(wt) = &worktree_root_effective {
            // The worktree root may not exist yet on first boot —
            // create it so `add_root` finds a directory to
            // canonicalize. Subsequent boots are no-ops.
            std::fs::create_dir_all(wt).ok();
            host_fs
                .add_root(wt)
                .map_err(|e| anyhow!("worktree fs root {}: {e}", wt.display()))?;
            eprintln!("codeless-server: fs extra root = {}", wt.display());
        }
        let host_fs_arc = Arc::new(host_fs);
        // Keep a clone aside so the liveness sweep (spawned after the
        // runtime is sealed into an Arc) can stat the same allowed-roots
        // set the `fs.*` surface serves. The runtime holds the
        // authoritative reference; this is a peer handle, not a fork.
        host_fs_for_liveness = Some(Arc::clone(&host_fs_arc));
        runtime = runtime.with_fs(host_fs_arc);
        eprintln!("codeless-server: fs root = {}", root.display());
    }
    // Same `WorktreeManager` Arc is given to both the runtime (so
    // `gc_worktrees` sees the on-disk state) and the driver below
    // (which provisions per-job trees). Creating it here, before the
    // runtime is sealed into an Arc, lets both halves share the
    // single source of truth. `worktree_root_effective` was resolved
    // above so the HostFs adapter could mark it as an extra root.
    let worktrees_arc: Option<Arc<WorktreeManager>> =
        worktree_root_effective.as_ref().map(|root| {
            if let Err(e) = std::fs::create_dir_all(root) {
                tracing::warn!(
                    error = %e,
                    root = %root.display(),
                    "could not create worktree root; provisioning will fail per job",
                );
            }
            Arc::new(WorktreeManager::new(root))
        });
    if let Some(worktrees) = worktrees_arc.clone() {
        runtime = runtime.with_worktrees(worktrees);
    }
    // CLI-runner registry powering the footer agent panel. The footer
    // is chat-only (no job, no worktree); it runs in whichever
    // directory the operator launched the server from. A future
    // "pick folder" UI surface lands as an arg to `with_agent_chat`.
    let agent_chat_registry = Arc::new(ai_runner::Registry::with_defaults());
    let agent_chat_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    runtime = runtime.with_agent_chat(Arc::clone(&agent_chat_registry), agent_chat_cwd.clone());
    let available_cli_runners =
        codeless_adapters_host::probe_available_cli_runners(&agent_chat_registry).await;
    if !available_cli_runners.is_empty() {
        eprintln!(
            "codeless-server: agent_chat CLI runners ready: {} (cwd={})",
            available_cli_runners.join(","),
            agent_chat_cwd.display()
        );
    }
    let rpc: Arc<InProcessRpc> = Arc::new(runtime);
    let rpc_dyn: Arc<dyn RpcServer> = rpc.clone();
    if effective.claude_enabled {
        scrub_caller_session_env();
    }
    let claude_status = if effective.claude_enabled {
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
    let server_info = Arc::new(build_server_info(
        &args,
        &effective,
        worktree_root_effective.clone(),
        claude_status,
        available_cli_runners,
    ));
    let state = AppState {
        rpc: rpc_dyn,
        auth,
        server_info,
        // `ai-ui` surface left unconfigured at this CLI for now —
        // wiring `CodelessProvider` + skills dir + components.json is
        // a follow-up. The route mount is opt-in via `with_ai_ui`.
        ai_ui: None,
        // Plugin catalog left unattached here; production wiring will
        // hand a built `PluginCatalog` to `AppState::with_plugins`
        // once the CLI grows a `--plugins-dir` (or equivalent)
        // surface that scans the plugin registry on boot. `None`
        // means `/plugins` and `/plugins/<id>/ui/*` are not
        // registered — byte-for-byte identical to a server compiled
        // without plugin support.
        plugins: None,
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

    // StageRecorder: persist Stage / Task rows from the event stream
    // so the UI's Stages tab can render rolled-up per-stage cost and
    // duration without reconstructing it from events. Best-effort —
    // a failure here logs and the loop continues.
    // PlanEngine (P1 of JOB-WORKFLOW chaining): construct once at
    // boot, subscribe to the runtime event bus so terminal Job events
    // can drive registered PlanRuns. In-memory only — restart drops
    // both registered plans and in-flight runs alongside everything
    // else in the engine's HashMaps. P1 ships a `LogJobSpawner` so
    // the boundary compiles; real job submission is a P2 follow-up.
    let plan_engine = Arc::new(PlanEngine::new(Arc::new(LogJobSpawner)));
    let _plan_subscriber = spawn_plan_engine_subscriber(rpc.bus().clone(), plan_engine.clone());
    eprintln!("codeless-server: plan engine wired (in-memory, LogJobSpawner)");

    let _stage_recorder = spawn_stage_recorder(rpc.bus().clone(), rpc.store().clone())
        .await
        .map_err(|e| anyhow!("spawn_stage_recorder: {e}"))?;
    eprintln!("codeless-server: stage recorder enabled");

    // 30 s liveness sweep: every tick stats the canonical roots the host
    // adapter is serving and publishes `workspace-unhealthy` /
    // `workspace-recovered` envelopes on the event bus when state flips.
    // Without an adapter (no `--fs-root`, nothing rehydrated from
    // `attached_workspaces`) there is nothing to watch, so the sweep is
    // only spawned when at least one root is registered. The handle
    // stays alive in this scope — process exit is the teardown path.
    let _liveness_sweep = host_fs_for_liveness.map(|fs| {
        let handle = codeless_runtime::spawn_workspace_liveness_sweep(
            fs,
            rpc.bus().clone(),
            rpc.pool().clone(),
            codeless_runtime::WORKSPACE_LIVENESS_PERIOD,
        );
        eprintln!(
            "codeless-server: workspace liveness sweep enabled (period={}s)",
            codeless_runtime::WORKSPACE_LIVENESS_PERIOD.as_secs(),
        );
        handle
    });

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
        let worktrees = worktrees_arc.clone();
        // Match the published runner list: mock is only registered
        // when no real runner is enabled. Logging the wrong set was
        // confusing — operators would see `runners=mock,claude` and
        // assume mock jobs would run, when in fact a `runner: mock`
        // submit returns None from the factory.
        let mut enabled: Vec<&str> = Vec::new();
        if effective.claude_enabled {
            enabled.push("claude");
        }
        if effective.anthropic_enabled {
            enabled.push("anthropic");
        }
        if effective.codex_enabled {
            enabled.push("codex");
        }
        if effective.copilot_enabled {
            enabled.push("copilot");
        }
        if enabled.is_empty() {
            enabled.push("mock");
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
            enable_claude: effective.claude_enabled,
            enable_anthropic: effective.anthropic_enabled,
            enable_codex: effective.codex_enabled,
            enable_copilot: effective.copilot_enabled,
            anthropic_api_key,
            claude_system_prompt,
            store: rpc.store().clone(),
            mcp_binary_path: resolve_mcp_binary(),
        });
        Some(
            spawn_job_driver_loop(rpc.clone(), factory, worktrees, args.driver_concurrency)
                .await
                .map_err(|e| anyhow!("driver init: {e}"))?,
        )
    };

    // Slack adapter is opt-in via `--enable-slack`. Missing tokens
    // produce a warning rather than a hard failure so the rest of the
    // server still boots — the bot is additive to the runtime and the
    // operator should be able to land Slack later by setting two
    // secrets and restarting. The handle stays alive in this scope so
    // the spawned task is not dropped before `serve_with_shutdown`
    // returns; process exit is the teardown path.
    let _slack = if effective.slack_enabled {
        match codeless_slack::SlackConfig::from_secrets(&store) {
            Ok(cfg) => {
                eprintln!(
                    "codeless-server: slack adapter enabled (channel={})",
                    cfg.channel_id.as_deref().unwrap_or("unset"),
                );
                // The dispatcher reaches the in-process runtime via
                // the same `RpcServer` handle the HTTP transport
                // serves; commands typed in Slack hit the exact same
                // code path as the web UI. Cloning is cheap (the
                // handle is an `Arc<dyn RpcServer>`).
                Some(codeless_slack::SlackBot::spawn(cfg, state.rpc.clone()))
            }
            Err(err) => {
                eprintln!("codeless-server: --enable-slack ignored: {err}");
                None
            }
        }
    } else {
        None
    };

    // Telegram adapter is opt-in via `--enable-telegram`. Mirrors
    // the Slack arm above: a missing token logs a warning and the
    // rest of the server still boots, and the handle stays in scope
    // so the spawned long-poll task is not dropped before
    // `serve_with_shutdown` returns.
    let _telegram = if effective.telegram_enabled {
        match codeless_telegram::TelegramConfig::from_secrets(&store) {
            Ok(cfg) => {
                eprintln!(
                    "codeless-server: telegram adapter enabled (chat={})",
                    cfg.chat_id.as_deref().unwrap_or("unset"),
                );
                match codeless_telegram::TelegramBot::spawn(cfg, state.rpc.clone()) {
                    Ok(bot) => Some(bot),
                    Err(err) => {
                        eprintln!(
                            "codeless-server: --enable-telegram ignored: api init failed: {err}"
                        );
                        None
                    }
                }
            }
            Err(err) => {
                eprintln!("codeless-server: --enable-telegram ignored: {err}");
                None
            }
        }
    } else {
        None
    };

    let port_file = args.port_file.clone();
    serve_with_shutdown(args.bind, state, |addr| {
        eprintln!("codeless-server listening on http://{addr}");
        if let Some(ref path) = port_file {
            if let Err(e) = std::fs::write(path, addr.to_string()) {
                eprintln!("warning: failed to write port file {}: {e}", path.display());
            }
        }
    })
    .await
    .map_err(|e| anyhow!("serve: {e}"))?;

    if let Some(ref path) = port_file {
        let _ = std::fs::remove_file(path);
    }

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
    effective: &codeless_runtime::adapter_registry::EffectiveAdapterRegistry,
    worktree_root: Option<PathBuf>,
    claude: Option<ClaudeStatus>,
    available_cli_runners: Vec<String>,
) -> ServerInfo {
    let mut runners = Vec::new();
    let real_runner_enabled = effective.claude_enabled
        || effective.anthropic_enabled
        || effective.codex_enabled
        || effective.copilot_enabled;
    // `mock` is only published when no real runner is enabled. When
    // the operator passes `--enable-claude` or `--enable-anthropic`
    // they have signalled that real coding work is what they want; a
    // mock entry in the dropdown is just noise that lets the user
    // accidentally submit a no-op job. Tests don't go through this
    // factory — they construct `MockRunner` directly — so the test
    // suite is unaffected.
    if !real_runner_enabled {
        runners.push(RunnerInfo {
            id: "mock".to_owned(),
            default: true,
        });
    }
    if effective.claude_enabled {
        runners.push(RunnerInfo {
            id: "claude".to_owned(),
            default: true,
        });
    }
    if effective.anthropic_enabled {
        runners.push(RunnerInfo {
            id: "anthropic".to_owned(),
            // `claude` wins the default when both are enabled; the
            // anthropic REST runner is the secondary path.
            default: !effective.claude_enabled,
        });
    }
    if effective.codex_enabled {
        runners.push(RunnerInfo {
            id: "codex".to_owned(),
            default: false,
        });
    }
    if effective.copilot_enabled {
        runners.push(RunnerInfo {
            id: "copilot".to_owned(),
            default: false,
        });
    }
    ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        runners,
        fs_root: args.fs_root.as_ref().map(|p| p.display().to_string()),
        worktree_root: worktree_root.as_ref().map(|p| p.display().to_string()),
        claude,
        available_cli_runners,
        // Capability flags default to `false`. Step 2 of the
        // scope-mutable-ui ramp lands the handover-schema fix that
        // round-trips `<!-- SCOPE-PATCH-* -->` markers, and flips
        // `scope_patch_handover_round_trip` here at the same time —
        // until then the UI's REVIEW-gate panel must omit the
        // patch-counter row rather than display a number that the
        // runtime cannot back.
        feature_flags: ServerFeatureFlags::default(),
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

/// Resolve the `codeless-mcp` binary path. Checks:
/// 1. `CODELESS_MCP_BINARY` env var (explicit override).
/// 2. Sibling of the current executable (same directory as the
///    `codeless` server binary — the standard `cargo build` layout).
/// 3. `codeless-mcp` on `PATH`.
///
/// Returns `None` with a tracing warning if not found.
fn resolve_mcp_binary() -> Option<String> {
    use std::path::PathBuf;

    if let Ok(explicit) = std::env::var("CODELESS_MCP_BINARY") {
        if PathBuf::from(&explicit).is_file() {
            tracing::info!(path = %explicit, "codeless-mcp binary (env override)");
            return Some(explicit);
        }
    }

    // Sibling of this process's binary (target/debug/codeless-mcp).
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("codeless-mcp");
        if sibling.is_file() {
            let path = sibling.to_string_lossy().into_owned();
            tracing::info!(path = %path, "codeless-mcp binary (sibling)");
            return Some(path);
        }
    }

    // Fall back to PATH.
    if let Ok(output) = std::process::Command::new("which")
        .arg("codeless-mcp")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                tracing::info!(path = %path, "codeless-mcp binary (PATH)");
                return Some(path);
            }
        }
    }

    tracing::warn!("codeless-mcp binary not found; jobs will not have access to codeless tools");
    None
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

/// Drop any environment variables that would make a child `claude`
/// process attempt to attach to the *caller's* Claude Code session.
/// When `codeless serve --enable-claude` is launched from inside an
/// interactive Claude Code terminal — the most common dev-loop —
/// every spawned runner inherits the parent shell's env. A fresh
/// `claude` invocation that sees `CLAUDECODE=1` and a matching
/// `CLAUDE_CODE_SESSION_ID` will hand its events back into the
/// originating session instead of running independently.
///
/// We scrub the parent process here (after the runtime is built so
/// startup logging still has the originals) so every child the
/// runner factory spawns inherits a clean slate. The set is
/// conservative: only the vars Claude Code itself uses to thread a
/// session identity, plus the SDK-checkpoint flag that has the same
/// "attach to the caller" failure mode.
fn scrub_caller_session_env() {
    for key in [
        "CLAUDECODE",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING",
        "AI_AGENT",
    ] {
        // SAFETY: process-wide env mutation is safe pre-runtime
        // because the only other live thread is the tokio worker
        // pool, which is not yet reading env vars at this point in
        // boot. The runner factories that *do* read env (claude
        // binary discovery via CLAUDE_BINARY, anthropic_api_key from
        // the secrets file) are constructed later, after this scrub
        // completes — so they see the post-scrub view.
        std::env::remove_var(key);
    }
    tracing::info!("scrubbed caller Claude Code session env so spawned runners run clean");
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

#[cfg(test)]
mod tests {
    use super::compose_system_prompt;

    #[test]
    fn compose_keeps_server_baseline_first() {
        let out = compose_system_prompt(Some("base rules"), Some("persona body"));
        assert_eq!(out.as_deref(), Some("base rules\n\npersona body"));
    }

    #[test]
    fn compose_returns_either_alone_when_only_one_set() {
        assert_eq!(
            compose_system_prompt(Some("base"), None).as_deref(),
            Some("base"),
        );
        assert_eq!(
            compose_system_prompt(None, Some("persona")).as_deref(),
            Some("persona"),
        );
    }

    #[test]
    fn compose_trims_whitespace_only_inputs_to_none() {
        assert_eq!(compose_system_prompt(Some("   "), Some("")), None);
        assert_eq!(compose_system_prompt(None, None), None);
    }
}
