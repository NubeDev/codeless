use std::path::PathBuf;
use std::sync::Arc;

use codeless_adapters_host::{HostFs, SecretStore, WorktreeManager};
use codeless_rpc::{RunnerInfo, ServerFeatureFlags, ServerInfo};
use codeless_runtime::{attached_workspaces, DefaultRunnerFactory, InProcessRpc};

/// Errors that prevent the desktop shell from booting.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("could not determine OS data directory")]
    NoDataDir,
    #[error("database: {0}")]
    Db(String),
    #[error("filesystem root: {0}")]
    FsRoot(String),
}

/// Result of `boot()`: the wired runtime plus the assembled
/// [`ServerInfo`] and a `DefaultRunnerFactory` ready to hand to the
/// driver loop. The desktop shell never runs the demo `mock` runner —
/// it always enables `claude` and `anthropic` so a fresh install talks
/// to real coding tools out of the box.
pub struct BootResult {
    pub rpc: Arc<InProcessRpc>,
    pub server_info: Arc<ServerInfo>,
    pub runner_factory: Arc<DefaultRunnerFactory>,
}

/// Boot the in-process runtime for the desktop shell.
///
/// The desktop shell is single-user, same-host — it uses OS-standard
/// data directories for the SQLite file, worktrees, secrets, and
/// assistant attachments. The `HostFs` jail is seeded from the
/// `attached_workspaces` table so every previously-attached workspace
/// is reachable immediately after launch; without this rehydration the
/// in-memory allowed-roots list would only contain the process cwd and
/// every prior attach would `PermissionDenied` until the user re-ran
/// the attach flow.
pub async fn boot() -> Result<BootResult, BootError> {
    let dirs = directories::ProjectDirs::from("dev", "codeless", "Codeless")
        .ok_or(BootError::NoDataDir)?;
    let data_dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&data_dir).ok();

    let db_path = data_dir.join("codeless.sqlite");
    let worktree_base = data_dir.join("worktrees");
    let assistant_root = data_dir.join("assistant");
    let secrets_path = data_dir.join("secrets.toml");
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    std::fs::create_dir_all(&worktree_base).ok();
    std::fs::create_dir_all(&assistant_root).ok();

    let runtime = InProcessRpc::with_file(&db_path)
        .await
        .map_err(|e| BootError::Db(e.to_string()))?;

    // Rehydrate the host adapter's allowed-roots list from the
    // `attached_workspaces` table before publishing the runtime. The
    // process cwd seeds an initial root so a first-run install (no
    // attached rows yet) still has somewhere to read from; every
    // canonical row found in the DB is added on top.
    let host_fs = HostFs::new(&workspace_root)
        .map_err(|e| BootError::FsRoot(format!("{}: {e}", workspace_root.display())))?;
    match attached_workspaces::list_canonical_roots(runtime.pool()).await {
        Ok(roots) => {
            for canonical in roots {
                if let Err(e) = host_fs.add_root(PathBuf::from(&canonical)) {
                    tracing::warn!(error = %e, path = %canonical, "boot: rehydrate add_root failed");
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "boot: could not list attached_workspaces");
        }
    }

    let runtime = runtime
        .with_fs(Arc::new(host_fs))
        .with_worktrees(Arc::new(WorktreeManager::new(&worktree_base)))
        .with_agent_chat(
            Arc::new(ai_runner::Registry::with_defaults()),
            workspace_root.clone(),
        )
        .with_assistant_data_dir(assistant_root);

    let agent_chat_registry = Arc::new(ai_runner::Registry::with_defaults());
    let available_cli_runners =
        codeless_adapters_host::probe_available_cli_runners(&agent_chat_registry).await;
    let claude_status = codeless_adapters_host::probe_claude().await;

    // Secrets file is best-effort: a missing or unreadable file just
    // means the anthropic API key is unset (the adapter surfaces the
    // auth failure at run time as `RunnerOutcome::Failed`). The desktop
    // shell never refuses to boot over a missing secret.
    let secrets = SecretStore::open(&secrets_path).ok();
    let anthropic_api_key = secrets
        .as_ref()
        .and_then(|s| s.get("anthropic_api_key").map(str::to_owned));
    let claude_system_prompt = secrets
        .as_ref()
        .and_then(|s| s.get("claude_system_prompt").map(str::to_owned));

    let runtime = Arc::new(runtime);

    let runner_factory = Arc::new(DefaultRunnerFactory {
        enable_claude: true,
        enable_anthropic: true,
        enable_codex: false,
        enable_copilot: false,
        anthropic_api_key,
        claude_system_prompt,
        store: runtime.store().clone(),
        mcp_binary_path: resolve_mcp_binary(),
    });

    // The runner list mirrors the factory's enabled set so the UI
    // dropdown reflects exactly what the driver will accept. `claude`
    // is the default — the higher-fidelity path; `anthropic` is the
    // secondary REST fallback.
    let runners = vec![
        RunnerInfo {
            id: "claude".to_owned(),
            default: true,
        },
        RunnerInfo {
            id: "anthropic".to_owned(),
            default: false,
        },
    ];

    let server_info = Arc::new(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        runners,
        fs_root: Some(workspace_root.display().to_string()),
        worktree_root: Some(worktree_base.display().to_string()),
        claude: claude_status,
        available_cli_runners,
        feature_flags: ServerFeatureFlags::default(),
        rest_url: None,
    });

    Ok(BootResult {
        rpc: runtime,
        server_info,
        runner_factory,
    })
}

/// Resolve the `codeless-mcp` binary. Checks `CODELESS_MCP_BINARY` env,
/// then the sibling of the current executable. Returns `None` when not
/// found — Claude runners then run without the codeless-tools MCP
/// surface but everything else still works.
fn resolve_mcp_binary() -> Option<String> {
    if let Ok(explicit) = std::env::var("CODELESS_MCP_BINARY") {
        if PathBuf::from(&explicit).is_file() {
            return Some(explicit);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("codeless-mcp");
        if sibling.is_file() {
            return Some(sibling.to_string_lossy().into_owned());
        }
    }
    None
}
