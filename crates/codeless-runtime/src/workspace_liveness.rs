//! Background sweep that watches every attached workspace's canonical
//! root and surfaces a `WorkspaceUnhealthy` / `WorkspaceRecovered` event
//! pair when its state transitions. Without this the UI would only learn
//! a workspace went missing on the next user-initiated `fs.*` call, by
//! which point the sidebar badge is stale.
//!
//! The sweep walks `HostFs::roots()` rather than `attached_workspaces`
//! directly: the adapter is the runtime trust gate (see stage 6), so a
//! root that is registered in the DB but not yet rehydrated into the
//! adapter is genuinely not serving traffic and reporting it as healthy
//! would be misleading. Each root is matched back to its `repo_id`
//! via `attached_workspaces.fs_root_canonical` so the wire event keeps
//! the same identity the rest of the system uses.
//!
//! Transitions are tracked in an in-process `HashMap<canonical, bool>`
//! kept inside the spawned task: the persisted rows remain the source
//! of truth for *which* workspaces are attached (R4), but tracking
//! healthy/unhealthy edges across ticks does not need a column —
//! whichever process happens to be running the sweep owns the edge
//! detection for its uptime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use codeless_adapters_host::HostFs;
use codeless_types::{Event, RepoId};
use sqlx::SqlitePool;
use tokio::task::JoinHandle;

use crate::event_bus::EventBus;
use crate::time::now_ms;

/// Cadence WORKSPACE-ATTACH.md specifies for the liveness sweep. Slow
/// enough that an attached USB drive briefly hiccuping does not
/// thrash badges; fast enough that the operator notices a missing
/// volume within a single coffee sip.
pub const WORKSPACE_LIVENESS_PERIOD: Duration = Duration::from_secs(30);

/// Spawn the periodic sweep. The handle keeps the task alive; dropping
/// it cancels the loop. The bus and adapter are shared with the rest of
/// the runtime so events emitted here ride the same broadcast tail and
/// catch-up replay as everything else under `RpcServer::subscribe`.
pub fn spawn_workspace_liveness_sweep(
    fs: Arc<HostFs>,
    bus: Arc<EventBus>,
    pool: SqlitePool,
    period: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut state: HashMap<PathBuf, bool> = HashMap::new();
        loop {
            tokio::time::sleep(period).await;
            sweep_once(&fs, &bus, &pool, &mut state).await;
        }
    })
}

/// One pass of the sweep. Public so tests can drive a deterministic
/// tick rather than wait for the 30 s timer. `state` is the per-task
/// edge-detection map; the function mutates it in place.
pub async fn sweep_once(
    fs: &HostFs,
    bus: &EventBus,
    pool: &SqlitePool,
    state: &mut HashMap<PathBuf, bool>,
) {
    let roots = fs.roots();
    // Drop entries the adapter no longer owns: detach removes the root,
    // and we should not keep a stale "previously healthy" flag for it.
    state.retain(|p, _| roots.iter().any(|r| r == p));

    for root in roots {
        let probe = probe_root(&root);
        let was_healthy = state.get(&root).copied();
        let now_healthy = probe.is_ok();

        match (was_healthy, now_healthy) {
            // First sight of a healthy root — record without emitting.
            // We only want events on transitions, not a startup flood.
            (None, true) => {
                state.insert(root, true);
            }
            // First sight of an unhealthy root — emit `WorkspaceUnhealthy`
            // so the UI flips the badge on the first sweep too.
            (None, false) => {
                if let Some(repo_id) = repo_id_for_root(pool, &root).await {
                    emit_unhealthy(bus, repo_id, &root, probe.unwrap_err()).await;
                }
                state.insert(root, false);
            }
            (Some(true), false) => {
                if let Some(repo_id) = repo_id_for_root(pool, &root).await {
                    emit_unhealthy(bus, repo_id, &root, probe.unwrap_err()).await;
                }
                state.insert(root, false);
            }
            (Some(false), true) => {
                if let Some(repo_id) = repo_id_for_root(pool, &root).await {
                    emit_recovered(bus, repo_id, &root).await;
                }
                state.insert(root, true);
            }
            // No edge — keep the recorded state, do not republish.
            (Some(true), true) | (Some(false), false) => {}
        }
    }
}

/// Classify a root by stat. Returns `Err(reason)` with a short
/// machine-readable tag the wire event surfaces verbatim. The tags are
/// part of the contract — see `Event::WorkspaceUnhealthy.reason`.
fn probe_root(root: &Path) -> Result<(), &'static str> {
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err("not-a-directory"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("missing"),
        Err(_) => Err("io-error"),
    }
}

async fn repo_id_for_root(pool: &SqlitePool, root: &Path) -> Option<RepoId> {
    let canonical = root.to_string_lossy();
    let lookup = sqlx::query_scalar::<_, String>(
        "SELECT repo_id FROM attached_workspaces WHERE fs_root_canonical = ?",
    )
    .bind(canonical.as_ref())
    .fetch_optional(pool)
    .await;
    let stored = match lookup {
        Ok(Some(s)) => s,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, path = %canonical, "liveness: repo_id lookup failed");
            return None;
        }
    };
    match stored.parse::<RepoId>() {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(error = %e, stored = %stored, "liveness: stored repo_id parse failed");
            None
        }
    }
}

async fn emit_unhealthy(bus: &EventBus, repo_id: RepoId, root: &Path, reason: &str) {
    let event = Event::WorkspaceUnhealthy {
        repo_id,
        fs_root: root.to_string_lossy().into_owned(),
        reason: reason.to_owned(),
    };
    if let Err(e) = bus.publish(None, None, None, event, now_ms()).await {
        tracing::warn!(error = %e, "liveness: publish workspace-unhealthy failed");
    }
}

async fn emit_recovered(bus: &EventBus, repo_id: RepoId, root: &Path) {
    let event = Event::WorkspaceRecovered {
        repo_id,
        fs_root: root.to_string_lossy().into_owned(),
    };
    if let Err(e) = bus.publish(None, None, None, event, now_ms()).await {
        tracing::warn!(error = %e, "liveness: publish workspace-recovered failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::InProcessRpc;
    use codeless_types::{GitAuth, UnixMillis};
    use tempfile::tempdir;

    async fn seed_attached(pool: &SqlitePool, repo_id: RepoId, canonical: &str) {
        let git_auth = serde_json::to_string(&GitAuth::Token {
            env_var: String::new(),
        })
        .unwrap();
        sqlx::query(
            "INSERT INTO repos (id, name, clone_url, default_branch, local_path, git_auth, \
             concurrency_cap, default_runner, created_at, updated_at) \
             VALUES (?, ?, '', 'main', ?, ?, NULL, NULL, 0, 0)",
        )
        .bind(repo_id.to_string())
        .bind(format!("repo-{}", repo_id))
        .bind(canonical)
        .bind(&git_auth)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO attached_workspaces \
             (repo_id, fs_root_canonical, fs_root_display, attached_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(repo_id.to_string())
        .bind(canonical)
        .bind(canonical)
        .bind(UnixMillis(0).0)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn collect_workspace_events(pool: &SqlitePool) -> Vec<(String, String, Option<String>)> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT type, payload FROM events \
             WHERE type IN ('workspace-unhealthy', 'workspace-recovered') \
             ORDER BY cursor",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        rows.into_iter()
            .map(|(ty, payload)| {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                let fs_root = v
                    .get("fs_root")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_owned();
                let reason = v
                    .get("reason")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_owned());
                (ty, fs_root, reason)
            })
            .collect()
    }

    #[tokio::test]
    async fn first_sight_of_healthy_root_emits_no_event() {
        let rpc = InProcessRpc::new().await.unwrap();
        let tmp = tempdir().unwrap();
        let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
        let canonical = fs.roots()[0].to_string_lossy().into_owned();
        seed_attached(rpc.pool(), RepoId(ulid::Ulid::new()), &canonical).await;

        let mut state = HashMap::new();
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;

        assert_eq!(state.len(), 1);
        assert_eq!(state.values().copied().collect::<Vec<_>>(), vec![true]);
        assert!(collect_workspace_events(rpc.pool()).await.is_empty());
    }

    #[tokio::test]
    async fn healthy_to_unhealthy_emits_unhealthy_with_reason() {
        let rpc = InProcessRpc::new().await.unwrap();
        let tmp = tempdir().unwrap();
        let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
        let canonical = fs.roots()[0].to_string_lossy().into_owned();
        seed_attached(rpc.pool(), RepoId(ulid::Ulid::new()), &canonical).await;

        let mut state = HashMap::new();
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;
        // Drop the temp directory so the next sweep sees `missing`.
        drop(tmp);
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;

        let events = collect_workspace_events(rpc.pool()).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "workspace-unhealthy");
        assert_eq!(events[0].1, canonical);
        assert_eq!(events[0].2.as_deref(), Some("missing"));

        // A third sweep with the path still missing must not republish —
        // the event fires on the edge, not every tick.
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;
        assert_eq!(collect_workspace_events(rpc.pool()).await.len(), 1);
    }

    #[tokio::test]
    async fn unhealthy_to_healthy_emits_recovered() {
        let rpc = InProcessRpc::new().await.unwrap();
        let tmp = tempdir().unwrap();
        let canonical_path = std::fs::canonicalize(tmp.path()).unwrap();
        let canonical = canonical_path.to_string_lossy().into_owned();
        let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
        seed_attached(rpc.pool(), RepoId(ulid::Ulid::new()), &canonical).await;

        // Seed the in-memory state as "previously unhealthy" so the sweep
        // sees the recovered edge without us first having to remove the
        // dir. This isolates the recovery branch from the
        // healthy→unhealthy branch already covered above.
        let mut state = HashMap::new();
        state.insert(canonical_path.clone(), false);

        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;
        let events = collect_workspace_events(rpc.pool()).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "workspace-recovered");
        assert_eq!(events[0].1, canonical);
        assert!(events[0].2.is_none());
    }

    #[tokio::test]
    async fn first_sight_of_unhealthy_emits_immediately() {
        let rpc = InProcessRpc::new().await.unwrap();
        // Construct on a real dir so `HostFs::new` accepts it, then drop
        // the directory before the sweep so the very first tick observes
        // a missing path. This is the realistic "boot rehydration found a
        // workspace whose folder was deleted while the server was off"
        // path; the operator must see the badge without a prior
        // healthy-to-unhealthy edge.
        let tmp = tempdir().unwrap();
        let canonical_path = std::fs::canonicalize(tmp.path()).unwrap();
        let canonical = canonical_path.to_string_lossy().into_owned();
        let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
        seed_attached(rpc.pool(), RepoId(ulid::Ulid::new()), &canonical).await;
        drop(tmp);

        let mut state = HashMap::new();
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;

        let events = collect_workspace_events(rpc.pool()).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "workspace-unhealthy");
        assert_eq!(events[0].2.as_deref(), Some("missing"));
    }

    /// Regression test for the wedge post-mortem: an earlier theory of the
    /// stuck-in-Queued bug was that the liveness sweep had been writing to
    /// the `jobs` table (e.g. flipping Stopped to Queued when a workspace
    /// transiently disappeared). The audit recorded in
    /// `DOCS/RUNTIME-DRIVER-RECOVERY-DECISIONS.md` confirms it does not.
    /// This test pins that invariant: every job row's `status`,
    /// `stop_reason`, `worktree_path`, `started_at` and `ended_at` are
    /// byte-identical before and after a sweep that publishes both an
    /// unhealthy and a recovered transition.
    #[tokio::test]
    async fn sweep_never_writes_to_jobs_table() {
        let rpc = InProcessRpc::new().await.unwrap();
        let tmp = tempdir().unwrap();
        let canonical_path = std::fs::canonicalize(tmp.path()).unwrap();
        let canonical = canonical_path.to_string_lossy().into_owned();
        let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
        let repo_id = RepoId(ulid::Ulid::new());
        seed_attached(rpc.pool(), repo_id, &canonical).await;

        // Seed one job per status the post-mortem flagged as a candidate
        // target: Queued (theorised re-queue victim), Stopped (theorised
        // source state), Running (would be catastrophic to touch), Failed
        // (terminal-but-recoverable). If the sweep mutated any of them the
        // diff below would fire regardless of which it picked.
        for status in ["draft", "queued", "running", "stopped", "failed"] {
            sqlx::query(
                "INSERT INTO jobs \
                 (id, repo_id, status, stop_reason, template_yaml, prompt, runner, branch, \
                  workspace_mode, worktree_path, cost_cap_cents, wall_clock_cap_ms, cost_cents, \
                  model, permission_mode, effort, started_at, ended_at, created_at) \
                 VALUES (?, ?, ?, NULL, NULL, NULL, 'mock', 'main', \
                  'worktree', ?, 0, 0, 0, NULL, NULL, NULL, NULL, NULL, 0)",
            )
            .bind(ulid::Ulid::new().to_string())
            .bind(repo_id.to_string())
            .bind(status)
            .bind(&canonical)
            .execute(rpc.pool())
            .await
            .unwrap();
        }

        // (id, status, stop_reason, worktree_path, started_at, ended_at).
        // Every column the sweep could plausibly touch; aliased to keep
        // clippy's type-complexity gate happy.
        type JobAuditRow = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
        );
        let snapshot = || async {
            sqlx::query_as::<_, JobAuditRow>(
                "SELECT id, status, stop_reason, worktree_path, started_at, ended_at \
                 FROM jobs ORDER BY id",
            )
            .fetch_all(rpc.pool())
            .await
            .unwrap()
        };
        let before = snapshot().await;
        assert_eq!(before.len(), 5);

        // Drive both edges in a single test: healthy on tick one (no
        // workspace event yet), unhealthy on tick two after the dir
        // disappears, recovered on tick three after we recreate it. Any
        // job-table write would surface on the diff regardless of which
        // tick triggered it.
        let mut state = HashMap::new();
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;
        drop(tmp);
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;
        std::fs::create_dir_all(&canonical_path).unwrap();
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;

        let after = snapshot().await;

        assert_eq!(before, after, "liveness sweep mutated the jobs table");

        // Sanity check: the sweep did emit the workspace events it owns,
        // so a vacuous "wrote nothing because it did nothing" is ruled out.
        let events = collect_workspace_events(rpc.pool()).await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "workspace-unhealthy");
        assert_eq!(events[1].0, "workspace-recovered");
    }

    #[tokio::test]
    async fn removed_root_drops_state_entry() {
        let rpc = InProcessRpc::new().await.unwrap();
        let tmp = tempdir().unwrap();
        let extra = tempdir().unwrap();
        let fs = Arc::new(HostFs::new(tmp.path()).unwrap());
        fs.add_root(extra.path()).unwrap();
        let extra_canon = std::fs::canonicalize(extra.path()).unwrap();
        seed_attached(
            rpc.pool(),
            RepoId(ulid::Ulid::new()),
            &fs.roots()[0].to_string_lossy(),
        )
        .await;
        seed_attached(
            rpc.pool(),
            RepoId(ulid::Ulid::new()),
            &extra_canon.to_string_lossy(),
        )
        .await;

        let mut state = HashMap::new();
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;
        assert_eq!(state.len(), 2);

        fs.remove_root(extra.path());
        sweep_once(&fs, rpc.bus(), rpc.pool(), &mut state).await;
        assert_eq!(state.len(), 1);
        assert!(!state.contains_key(&extra_canon));
    }
}
