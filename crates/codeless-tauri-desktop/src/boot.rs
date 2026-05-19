use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use codeless_adapters_host::{HostFs, SecretStore, WorktreeManager};
use codeless_rpc::{RunnerInfo, ServerFeatureFlags, ServerInfo};
use codeless_runtime::{attached_workspaces, DefaultRunnerFactory, InProcessRpc};

/// Errors that prevent the desktop shell from booting.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("could not determine home directory")]
    NoHomeDir,
    #[error("database: {0}")]
    Db(String),
    #[error("filesystem root: {0}")]
    FsRoot(String),
}

/// Paths the desktop shell writes to, derived from the home dir and
/// the launch-time workspace. Secrets are user-global so the API key
/// and Slack token survive across workspaces; everything else is keyed
/// by the workspace slug so two desktop launches on different folders
/// own disjoint state — disjoint SQLite file, disjoint worktrees,
/// disjoint job queue, disjoint event bus.
pub struct DataPaths {
    pub workspace_root: PathBuf,
    pub secrets_path: PathBuf,
    pub db_path: PathBuf,
    pub worktree_base: PathBuf,
    pub assistant_root: PathBuf,
}

/// Resolve the on-disk layout under `~/.codeless/`:
///
/// ```text
/// ~/.codeless/
///     secrets.toml                  shared across workspaces
///     workspaces/<slug>/
///         codeless.sqlite
///         worktrees/
///         assistant/
/// ```
///
/// `<slug>` is `<last-path-segment>-<8-hex>` where the hex is a hash
/// of the canonical workspace path. Two different folders that share
/// a last segment (`~/code/foo` vs `~/work/foo`) get distinct slugs;
/// the same folder accessed through a symlink resolves to one slug.
pub fn resolve_data_paths(workspace: &Path) -> Result<DataPaths, BootError> {
    let home = dirs_home()?;
    let codeless_dir = home.join(".codeless");
    let secrets_path = codeless_dir.join("secrets.toml");

    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let slug = workspace_slug(&canonical);
    let ws_dir = codeless_dir.join("workspaces").join(&slug);

    Ok(DataPaths {
        workspace_root: canonical,
        secrets_path,
        db_path: ws_dir.join("codeless.sqlite"),
        worktree_base: ws_dir.join("worktrees"),
        assistant_root: ws_dir.join("assistant"),
    })
}

/// Stable per-workspace directory name. The leading segment is the
/// last path component so the directory is human-recognisable when a
/// user opens `~/.codeless/workspaces/`; the trailing 8-hex hash of
/// the canonical path disambiguates two folders that share a name.
fn workspace_slug(canonical: &Path) -> String {
    let leaf = canonical
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "root".to_owned());
    let leaf_sanitised: String = leaf
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{leaf_sanitised}-{hash:08x}")
}

fn dirs_home() -> Result<PathBuf, BootError> {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or(BootError::NoHomeDir)
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

/// Boot the in-process runtime for the desktop shell against a single
/// workspace. Each launch of the desktop binary owns one workspace —
/// the SQLite file, worktrees, and assistant attachments live under
/// `~/.codeless/workspaces/<slug>/`. Two launches on different folders
/// never see each other's jobs or events; two launches on the same
/// folder open the same SQLite file (single-instance handling lives
/// in `main.rs`, not here). Secrets are shared across workspaces.
///
/// The `HostFs` jail is seeded with the workspace root and rehydrated
/// from this workspace's own `attached_workspaces` table — that table
/// is still used to track extra roots within this single-workspace
/// process (e.g. a sibling repo a job needs read access to).
pub async fn boot(workspace_root: PathBuf) -> Result<BootResult, BootError> {
    let paths = resolve_data_paths(&workspace_root)?;

    if let Some(parent) = paths.secrets_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all(&paths.worktree_base).ok();
    std::fs::create_dir_all(&paths.assistant_root).ok();

    let workspace_root = paths.workspace_root.clone();
    let db_path = paths.db_path;
    let worktree_base = paths.worktree_base;
    let assistant_root = paths.assistant_root;
    let secrets_path = paths.secrets_path;

    let runtime = InProcessRpc::with_file(&db_path)
        .await
        .map_err(|e| BootError::Db(e.to_string()))?;

    // The workspace root seeds the allowed-roots list; any extra roots
    // tracked in this workspace's own `attached_workspaces` table layer
    // on top. Per-workspace state means a fresh `~/.codeless/workspaces/<slug>/`
    // SQLite starts with an empty extras list — the only guaranteed
    // root is the one this binary was launched against.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_deterministic_for_same_path() {
        let p = PathBuf::from("/home/user/code/foo");
        assert_eq!(workspace_slug(&p), workspace_slug(&p));
    }

    #[test]
    fn slug_disambiguates_same_leaf_in_different_parents() {
        let a = workspace_slug(&PathBuf::from("/home/user/code/foo"));
        let b = workspace_slug(&PathBuf::from("/home/user/work/foo"));
        assert!(a.starts_with("foo-"), "expected leaf prefix, got {a}");
        assert!(b.starts_with("foo-"), "expected leaf prefix, got {b}");
        assert_ne!(a, b, "same leaf in different parents must not collide");
    }

    #[test]
    fn slug_sanitises_non_alphanumeric_leaf_chars() {
        let s = workspace_slug(&PathBuf::from("/tmp/my project (v2)"));
        assert!(s.starts_with("my-project--v2--"), "got {s}");
    }
}
