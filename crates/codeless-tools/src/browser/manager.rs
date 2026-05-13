// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/browser/manager.rs
//
// Lazy supervised lifecycle wrapper around `SidecarProcess`. One
// manager per host process (codeless is single-tenant — R5);
// concurrent first-callers coalesce on the spawn under a single
// mutex.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use super::bootstrap::{ensure_installed, BootstrapPaths};
use super::config::BrowserManagerConfig;
use super::sidecar::SidecarProcess;
use crate::error::ToolError;

/// How the manager learns its spawn paths.
///
/// `Resolved`: the caller already knows the Node binary, sidecar
/// script, and browsers dir. Used by tests with fake sidecars and by
/// production deploys that pin a system Node.
///
/// `Bootstrap`: lazy-install on first request. Downloads Node +
/// Playwright + Chromium under the supplied paths, caches via the
/// install marker, returns a resolved config for spawning. The first
/// request after a cold install takes minutes; every later request
/// short-circuits on the marker.
enum SpawnSource {
    Resolved(BrowserManagerConfig),
    Bootstrap {
        paths: BootstrapPaths,
        /// Timeouts and node args to use after bootstrap resolves.
        /// `node_bin`, `sidecar_script`, and `browsers_dir` are
        /// filled in from the bootstrap output.
        template: BrowserManagerConfig,
    },
}

pub struct BrowserManager {
    source: Mutex<SpawnSource>,
    state: Mutex<State>,
    /// Cached after the first resolved spawn so subsequent requests
    /// don't re-check the marker.
    resolved_config: Mutex<Option<BrowserManagerConfig>>,
    /// Idle-timeout duration extracted up-front so the watcher
    /// doesn't have to lock `source` on every tick.
    idle_timeout: Duration,
    /// Same rationale for max_timeout: hot-path clamp without lock.
    max_timeout: Duration,
    default_timeout: Duration,
}

struct State {
    sidecar: Option<Arc<SidecarProcess>>,
    last_active: Instant,
}

impl BrowserManager {
    /// Construct from a fully-resolved config — caller supplies
    /// `node_bin`, `sidecar_script`, and `browsers_dir`. No
    /// bootstrap will happen.
    pub fn new(config: BrowserManagerConfig) -> Arc<Self> {
        let idle = config.idle_timeout;
        let max = config.max_timeout;
        let default = config.default_timeout;
        let mgr = Arc::new(Self {
            source: Mutex::new(SpawnSource::Resolved(config.clone())),
            state: Mutex::new(State {
                sidecar: None,
                last_active: Instant::now(),
            }),
            resolved_config: Mutex::new(Some(config)),
            idle_timeout: idle,
            max_timeout: max,
            default_timeout: default,
        });
        Self::spawn_idle_watcher(mgr.clone());
        mgr
    }

    /// Construct in lazy-bootstrap mode. On the first `request`, the
    /// manager runs the full install pipeline (Node + Playwright +
    /// Chromium) under `paths`, caches the resolved spawn config,
    /// then spawns the sidecar. `template` supplies node_args,
    /// timeouts, and any other knobs the bootstrap doesn't determine.
    pub fn with_bootstrap(paths: BootstrapPaths, template: BrowserManagerConfig) -> Arc<Self> {
        let idle = template.idle_timeout;
        let max = template.max_timeout;
        let default = template.default_timeout;
        let mgr = Arc::new(Self {
            source: Mutex::new(SpawnSource::Bootstrap { paths, template }),
            state: Mutex::new(State {
                sidecar: None,
                last_active: Instant::now(),
            }),
            resolved_config: Mutex::new(None),
            idle_timeout: idle,
            max_timeout: max,
            default_timeout: default,
        });
        Self::spawn_idle_watcher(mgr.clone());
        mgr
    }

    /// Returns the resolved spawn config if one is cached. In
    /// bootstrap mode this is `None` until the first request lands.
    pub async fn resolved_config(&self) -> Option<BrowserManagerConfig> {
        self.resolved_config.lock().await.clone()
    }

    /// Ensure the sidecar is running, forward a JSON-RPC request,
    /// and return the decoded result. Updates last-active even on
    /// error — the agent is still using the sidecar, errors don't
    /// mean it can be idle-killed.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Option<Duration>,
    ) -> Result<serde_json::Value, ToolError> {
        let timeout = self.clamp_timeout(timeout);
        let sidecar = self.get_or_spawn().await?;
        let result = sidecar.request(method, params, timeout).await;

        let mut st = self.state.lock().await;
        st.last_active = Instant::now();
        if !sidecar.is_alive() {
            st.sidecar = None;
        }
        result
    }

    /// Graceful shutdown. Idempotent.
    pub async fn shutdown(&self) {
        let taken = {
            let mut st = self.state.lock().await;
            st.sidecar.take()
        };
        if let Some(sc) = taken {
            sc.shutdown().await;
        }
    }

    fn clamp_timeout(&self, requested: Option<Duration>) -> Duration {
        let want = requested.unwrap_or(self.default_timeout);
        want.clamp(Duration::from_secs(1), self.max_timeout)
    }

    async fn get_or_spawn(&self) -> Result<Arc<SidecarProcess>, ToolError> {
        // Fast path: live sidecar already.
        {
            let st = self.state.lock().await;
            if let Some(sc) = &st.sidecar {
                if sc.is_alive() {
                    return Ok(sc.clone());
                }
            }
        }

        // Slow path: resolve config (running bootstrap if needed) and
        // spawn under the lock so concurrent first callers coalesce.
        let mut st = self.state.lock().await;
        if let Some(sc) = &st.sidecar {
            if sc.is_alive() {
                return Ok(sc.clone());
            }
        }
        let config = self.resolve_spawn_config().await?;
        let sidecar = Arc::new(SidecarProcess::spawn(&config).await?);
        st.sidecar = Some(sidecar.clone());
        st.last_active = Instant::now();
        Ok(sidecar)
    }

    /// Return a `BrowserManagerConfig` ready to spawn from. In
    /// bootstrap mode the first call here runs `ensure_installed`
    /// and caches the result.
    async fn resolve_spawn_config(&self) -> Result<BrowserManagerConfig, ToolError> {
        if let Some(cfg) = self.resolved_config.lock().await.as_ref() {
            return Ok(cfg.clone());
        }
        let mut source = self.source.lock().await;
        let cfg = match &*source {
            SpawnSource::Resolved(c) => c.clone(),
            SpawnSource::Bootstrap { paths, template } => {
                let install = ensure_installed(paths).await?;
                BrowserManagerConfig {
                    node_bin: install.node_bin,
                    sidecar_script: install.sidecar_script,
                    browsers_dir: install.browsers_dir,
                    node_args: template.node_args.clone(),
                    default_timeout: template.default_timeout,
                    max_timeout: template.max_timeout,
                    idle_timeout: template.idle_timeout,
                }
            }
        };
        // Flip the source over to Resolved so we never bootstrap
        // twice even if the cache gets cleared.
        *source = SpawnSource::Resolved(cfg.clone());
        *self.resolved_config.lock().await = Some(cfg.clone());
        Ok(cfg)
    }

    /// Background task that idle-kills the sidecar when it hasn't
    /// been used for `config.idle_timeout`. Exits when the manager
    /// is dropped (no other Arc reference remains).
    fn spawn_idle_watcher(mgr: Arc<Self>) {
        tokio::spawn(async move {
            let check_interval = Duration::from_secs(30);
            loop {
                tokio::time::sleep(check_interval).await;
                if Arc::strong_count(&mgr) <= 1 {
                    return;
                }
                let idle_timeout = mgr.idle_timeout;
                let mut st = mgr.state.lock().await;
                let idle_for = st.last_active.elapsed();
                if let Some(sc) = &st.sidecar {
                    if sc.is_alive() && idle_for >= idle_timeout {
                        tracing::info!(
                            idle_secs = idle_for.as_secs(),
                            "idle-killing browser sidecar"
                        );
                        let to_close = st.sidecar.take();
                        drop(st);
                        if let Some(sc) = to_close {
                            sc.shutdown().await;
                        }
                        continue;
                    }
                    if !sc.is_alive() {
                        st.sidecar = None;
                    }
                }
            }
        });
    }
}
