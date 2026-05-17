//! Slack control-plane adapter for the Codeless server. The crate
//! sits on two ends:
//!
//!   - **Inbound** (stages 2-4): [`SlackBot::spawn`] establishes a
//!     Slack Socket Mode WebSocket and keeps it open with
//!     reconnect/backoff. Inbound envelopes are acked, projected onto
//!     [`codeless_bot_core::InboundMessage`] by [`envelope::extract_inbound`],
//!     and handed to a [`Dispatcher`] from `codeless-bot-core` that
//!     parses the message body, calls the matching `RpcServer`
//!     method, and posts a synchronous reply via `chat.postMessage`.
//!   - **Outbound** (stage 6): the bot-core [`OutboundPublisher`]
//!     subscribes to the event bus and posts a single Slack message
//!     per terminal transition (`JobFailed` / `JobStopped`), debounced
//!     per-job. The post's `ts` is registered in the [`ThreadMap`] so
//!     a bare-verb reply (`resume bypass`, `stop`) inside the thread
//!     resolves to the right job id without the operator retyping it.
//!
//! All transport-agnostic pieces — parser, reply renderers, failure
//! renderers, dispatcher, publisher, debouncer, REVIEW-gate cache —
//! live in [`codeless_bot_core`]; this crate only owns the
//! Slack-specific bits: the `chat.postMessage` HTTP wrapper, the
//! Socket Mode WebSocket transport, the `events_api` envelope shape,
//! and the `SlackConfig` reader.
//!
//! [`SlackConfig::from_secrets`] reads the bot/app tokens and the
//! configured channel out of the shared `SecretStore`; the outbound
//! publisher is only spawned when `channel_id` is set, so a deployment
//! that wants commands-only behaviour can omit the channel key and
//! the inbound side keeps working.
//!
//! Host-only per R1 (the WebSocket sits next to `codeless-server`).
//! The crate is excluded from the mobile-safe column of the crate
//! table because mobile shells reach the same RPC surface through the
//! HTTP/SSE transport — they do not need the Slack bridge.

pub mod config;
pub mod envelope;
pub mod socket_mode;
pub mod web_api;

use std::sync::Arc;

use codeless_bot_core::transport::BotTransport;
use codeless_bot_core::{
    CommandBackend, Dispatcher, EventSource, OutboundConfig, OutboundPublisher, RpcServerBackend,
    RpcServerEventSource, ThreadMap,
};
use codeless_rpc::RpcServer;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use codeless_bot_core::{
    parse_command, Command, InboundMessage, ParseError, ReviewContext, ThreadContext,
    DEBOUNCE_WINDOW, REVIEW_CACHE_CAPACITY,
};
pub use config::{
    SlackConfig, SlackConfigError, SLACK_APP_TOKEN_KEY, SLACK_BOT_TOKEN_KEY, SLACK_CHANNEL_KEY,
};
pub use envelope::{decode_envelope, extract_inbound, EnvelopePayload};
pub use socket_mode::{SocketModeError, SocketModeSession};
pub use web_api::{ChatPoster, PostedMessage, SlackPostError};

/// Handle to a running Slack bot. Dropping it leaves the spawned task
/// running until the server exits; calling [`SlackBot::shutdown`]
/// signals the task to break out of its reconnect loop and waits for
/// the task to join. The bot dispatches inbound commands through the
/// supplied [`RpcServer`] handle and posts replies back via the bot
/// token. When the config carries a `channel_id` an
/// [`OutboundPublisher`] is also spawned; its task subscribes to the
/// event bus and posts failure notifications into the channel,
/// registering the resulting `ts` in the shared [`ThreadMap`] so the
/// dispatcher can resolve in-thread replies.
pub struct SlackBot {
    join: JoinHandle<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    threads: ThreadMap,
    outbound: Option<OutboundPublisher>,
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
        let backend: Arc<dyn CommandBackend> = Arc::new(RpcServerBackend::new(rpc.clone()));
        let poster = ChatPoster::new(http.clone(), config.bot_token.clone());
        let transport: Arc<dyn BotTransport> = Arc::new(poster);
        let threads = ThreadMap::new();
        let dispatcher = Dispatcher::new(backend.clone(), transport.clone(), threads.clone());

        // The outbound publisher only spawns when a channel is
        // configured — a deployment that wants commands-only behaviour
        // (the bot answers `@codeless status` but never posts on its
        // own) leaves `slack_channel_id` unset and the publisher is
        // simply absent. A WARN-level log surfaces the inert state on
        // startup so a half-configured deployment is visible.
        let outbound = match config.channel_id.clone() {
            Some(channel_id) => {
                let events: Arc<dyn EventSource> = Arc::new(RpcServerEventSource::new(rpc));
                Some(OutboundPublisher::spawn(
                    OutboundConfig::new(channel_id),
                    events,
                    backend,
                    transport,
                    threads.clone(),
                ))
            }
            None => {
                tracing::warn!(
                    "slack: no slack_channel_id configured; outbound failure notifications disabled"
                );
                None
            }
        };

        let socket_config = config;
        let join = tokio::spawn(async move {
            let session = SocketModeSession::new(socket_config, http).with_dispatcher(dispatcher);
            session.run_until_shutdown(shutdown_rx).await;
        });
        Self {
            join,
            shutdown_tx: Some(shutdown_tx),
            threads,
            outbound,
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
    /// already consumed. The outbound publisher (when present) is
    /// shut down in the same call so a clean server shutdown drains
    /// both halves before returning.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
        if let Some(outbound) = self.outbound.take() {
            outbound.shutdown().await;
        }
    }
}
