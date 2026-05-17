use std::path::PathBuf;
use std::sync::Arc;

use codeless_adapters_host::{HostFs, WorktreeManager};
use codeless_rpc::{RunnerInfo, ServerFeatureFlags, ServerInfo};
use codeless_runtime::InProcessRpc;

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

/// Boot the in-process runtime for the desktop shell.
///
/// The desktop shell is single-user, same-host — it uses OS-standard
/// data directories for the SQLite file, worktrees, and assistant
/// attachments, and the process cwd as the initial workspace root.
pub async fn boot() -> Result<(Arc<InProcessRpc>, Arc<ServerInfo>), BootError> {
    let dirs = directories::ProjectDirs::from("dev", "codeless", "Codeless")
        .ok_or(BootError::NoDataDir)?;
    let data_dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&data_dir).ok();

    let db_path = data_dir.join("codeless.sqlite");
    let worktree_base = data_dir.join("worktrees");
    let assistant_root = data_dir.join("assistant");
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    std::fs::create_dir_all(&worktree_base).ok();
    std::fs::create_dir_all(&assistant_root).ok();

    let host_fs = HostFs::new(&workspace_root)
        .map_err(|e| BootError::FsRoot(format!("{}: {e}", workspace_root.display())))?;

    let runtime = InProcessRpc::with_file(&db_path)
        .await
        .map_err(|e| BootError::Db(e.to_string()))?
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

    let server_info = Arc::new(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        runners: vec![RunnerInfo {
            id: "mock".to_owned(),
            default: true,
        }],
        fs_root: Some(workspace_root.display().to_string()),
        worktree_root: Some(worktree_base.display().to_string()),
        claude: claude_status,
        available_cli_runners,
        feature_flags: ServerFeatureFlags::default(),
    });

    Ok((Arc::new(runtime), server_info))
}
