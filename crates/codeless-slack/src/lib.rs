//! Slack control-plane adapter for the Codeless server. Stage 4
//! wires the inbound surface end-to-end:
//!
//!   - [`SlackConfig::from_secrets`] reads the bot/app tokens and the
//!     configured channel out of the shared `SecretStore`.
//!   - [`SlackBot::spawn`] establishes a Slack Socket Mode WebSocket
//!     and keeps it open with reconnect/backoff. Inbound envelopes are
//!     acked and then handed to a [`Dispatcher`] that parses the
//!     message body, calls the matching `RpcServer` method, and posts
//!     a synchronous reply via `chat.postMessage`.
//!   - [`command::parse`] turns a raw Slack message body plus
//!     thread-context into a typed [`Command`] for Surface 1.
//!   - [`reply`] holds the renderers each command's reply goes
//!     through; staying pure means the format can be unit-tested
//!     without the network plumbing.
//!   - [`thread_map::ThreadMap`] caches the `(channel, thread_ts)
//!     -> JobId` mappings stage 6 will populate when posting
//!     outbound failure notifications. Stage 4 only reads it; an
//!     empty map means in-thread replies degrade to the cold grammar
//!     (`MissingJobId`), never to a wrong-job dispatch.
//!
//! Host-only per R1 (the WebSocket sits next to `codeless-server`).
//! The crate is excluded from the mobile-safe column of the crate
//! table because mobile shells reach the same RPC surface through the
//! HTTP/SSE transport — they do not need the Slack bridge.

pub mod command;
pub mod config;
pub mod dispatcher;
pub mod reply;
pub mod socket_mode;
pub mod thread_map;
pub mod web_api;

use std::sync::Arc;

use codeless_rpc::RpcServer;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use command::{parse as parse_command, Command, ParseError, ThreadContext};
pub use config::{
    SlackConfig, SlackConfigError, SLACK_APP_TOKEN_KEY, SLACK_BOT_TOKEN_KEY, SLACK_CHANNEL_KEY,
};
pub use dispatcher::{CommandBackend, Dispatcher, InboundMessage, RpcServerBackend};
pub use socket_mode::{SocketModeError, SocketModeSession};
pub use thread_map::ThreadMap;
pub use web_api::{ChatPoster, SlackPostError};

/// Handle to a running Slack bot. Dropping it leaves the spawned task
/// running until the server exits; calling [`SlackBot::shutdown`]
/// signals the task to break out of its reconnect loop and waits for
/// the task to join. The bot dispatches inbound commands through the
/// supplied [`RpcServer`] handle and posts replies back via the bot
/// token. The outbound notification publisher (stage 6) hangs off
/// the same handle once it lands.
pub struct SlackBot {
    join: JoinHandle<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    threads: ThreadMap,
}

impl SlackBot {
    /// Boot a Slack adapter against the supplied config. The returned
    /// handle keeps the connection alive in the background; the caller
    /// is expected to hold it for the server's lifetime. A
    /// `reqwest::Client` is built internally — call [`Self::spawn_with`]
    /// when the caller wants to share a client with the rest of the
    /// process (the Slack adapter does not benefit much from
    /// connection-pool sharing since `apps.connections.open` runs once
    /// per reconnect, but `chat.postMessage` does).
    pub fn spawn(config: SlackConfig, rpc: Arc<dyn RpcServer>) -> Self {
        Self::spawn_with(config, Arc::new(reqwest::Client::new()), rpc)
    }

    /// Like [`Self::spawn`] but accepts a caller-built
    /// `reqwest::Client`. Used by tests that wire a stub against a
    /// local wiremock and by future callers that want to share one
    /// HTTP client across host adapters.
    pub fn spawn_with(
        config: SlackConfig,
        http: Arc<reqwest::Client>,
        rpc: Arc<dyn RpcServer>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let backend: Arc<dyn CommandBackend> = Arc::new(RpcServerBackend::new(rpc));
        let poster = ChatPoster::new(http.clone(), config.bot_token.clone());
        let threads = ThreadMap::new();
        let dispatcher = Dispatcher::new(backend, poster, threads.clone());
        let join = tokio::spawn(async move {
            let session = SocketModeSession::new(config, http).with_dispatcher(dispatcher);
            session.run_until_shutdown(shutdown_rx).await;
        });
        Self {
            join,
            shutdown_tx: Some(shutdown_tx),
            threads,
        }
    }

    /// Snapshot of the outbound thread map. Stage 6 (failure
    /// notifications) calls `record` on this handle when posting a
    /// new notification so the dispatcher can resolve subsequent
    /// thread replies to the job id without the operator retyping it.
    pub fn thread_map(&self) -> ThreadMap {
        self.threads.clone()
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
