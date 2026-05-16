//! Slack control-plane adapter for the Codeless server. The full
//! surface — command parser, RPC dispatch, outbound failure
//! notifications — lands in later stages of the slack-integration
//! job; this crate currently exposes only the transport seam:
//!
//!   - [`SlackConfig::from_secrets`] reads the bot/app tokens and the
//!     configured channel out of the shared `SecretStore`.
//!   - [`SlackBot::spawn`] establishes a Slack Socket Mode WebSocket
//!     and keeps it open with reconnect/backoff. Inbound envelopes are
//!     logged and acked; nothing is dispatched to the runtime yet.
//!
//! Host-only per R1 (the WebSocket sits next to `codeless-server`).
//! The crate is excluded from the mobile-safe column of the crate
//! table because mobile shells reach the same RPC surface through the
//! HTTP/SSE transport — they do not need the Slack bridge.

pub mod config;
pub mod socket_mode;

use std::sync::Arc;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use config::{
    SlackConfig, SlackConfigError, SLACK_APP_TOKEN_KEY, SLACK_BOT_TOKEN_KEY, SLACK_CHANNEL_KEY,
};
pub use socket_mode::{SocketModeError, SocketModeSession};

/// Handle to a running Slack bot. Dropping it leaves the spawned task
/// running until the server exits; calling [`SlackBot::shutdown`]
/// signals the task to break out of its reconnect loop and waits for
/// the task to join. Stage 2 keeps the task body deliberately bare —
/// it owns the Socket Mode session and discards inbound envelopes
/// after acking them. Stages 3+ replace the discard with a real
/// dispatch into `RpcServer` and an outbound publisher on the event
/// bus.
pub struct SlackBot {
    join: JoinHandle<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl SlackBot {
    /// Boot a Slack adapter against the supplied config. The returned
    /// handle keeps the connection alive in the background; the caller
    /// is expected to hold it for the server's lifetime. A
    /// `reqwest::Client` is built internally — call [`Self::spawn_with`]
    /// when the caller wants to share a client with the rest of the
    /// process (the Slack adapter does not benefit much from
    /// connection-pool sharing since `apps.connections.open` runs once
    /// per reconnect).
    pub fn spawn(config: SlackConfig) -> Self {
        Self::spawn_with(config, Arc::new(reqwest::Client::new()))
    }

    /// Like [`Self::spawn`] but accepts a caller-built
    /// `reqwest::Client`. Used by tests that wire a stub against a
    /// local wiremock and by future callers that want to share one
    /// HTTP client across host adapters.
    pub fn spawn_with(config: SlackConfig, http: Arc<reqwest::Client>) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let session = SocketModeSession::new(config, http);
            session.run_until_shutdown(shutdown_rx).await;
        });
        Self {
            join,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    /// Signal the connect loop to exit, then await the task. Idempotent:
    /// a second call returns immediately because the signal channel is
    /// already consumed.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
    }
}
