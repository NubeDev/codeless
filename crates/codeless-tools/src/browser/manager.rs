// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/browser/manager.rs
//
// Lazy supervised lifecycle wrapper around `SidecarProcess`. One
// manager per host process (codeless is single-tenant — R5);
// concurrent first-callers coalesce on the spawn under a single
// mutex.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use super::config::BrowserManagerConfig;
use super::sidecar::SidecarProcess;
use crate::error::ToolError;

pub struct BrowserManager {
    config: BrowserManagerConfig,
    state: Mutex<State>,
}

struct State {
    sidecar: Option<Arc<SidecarProcess>>,
    last_active: Instant,
}

impl BrowserManager {
    pub fn new(config: BrowserManagerConfig) -> Arc<Self> {
        let mgr = Arc::new(Self {
            config,
            state: Mutex::new(State {
                sidecar: None,
                last_active: Instant::now(),
            }),
        });
        Self::spawn_idle_watcher(mgr.clone());
        mgr
    }

    pub fn config(&self) -> &BrowserManagerConfig {
        &self.config
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
        let want = requested.unwrap_or(self.config.default_timeout);
        want.clamp(Duration::from_secs(1), self.config.max_timeout)
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

        // Slow path: spawn under the lock so concurrent first callers
        // coalesce on a single spawn.
        let mut st = self.state.lock().await;
        if let Some(sc) = &st.sidecar {
            if sc.is_alive() {
                return Ok(sc.clone());
            }
        }
        let sidecar = Arc::new(SidecarProcess::spawn(&self.config).await?);
        st.sidecar = Some(sidecar.clone());
        st.last_active = Instant::now();
        Ok(sidecar)
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
                let idle_timeout = mgr.config.idle_timeout;
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
