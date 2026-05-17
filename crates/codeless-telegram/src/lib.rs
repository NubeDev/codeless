//! Telegram control-plane adapter for the Codeless server. Mirrors
//! `codeless_slack`:
//!
//!   - **Inbound** (Surface 1): [`TelegramBot::spawn`] runs a
//!     `getUpdates` long-poll loop. Each inbound `message` is
//!     projected onto [`codeless_bot_core::InboundMessage`] by
//!     [`dispatcher::project_update`] and handed to a
//!     [`codeless_bot_core::Dispatcher`] whose backend is the
//!     supplied [`RpcServer`].
//!   - **Outbound** (Surface 2): when [`config::TelegramConfig::chat_id`]
//!     is set, a [`codeless_bot_core::OutboundPublisher`] is also
//!     spawned. It subscribes to the runtime event bus and posts a
//!     MarkdownV2 failure card via [`outbound::MarkdownV2Transport`]
//!     on `JobFailed` / `JobStopped`, debounced per job. The
//!     returned `message_id` is registered in the bot-core
//!     [`codeless_bot_core::ThreadMap`] so a bare-verb reply in the
//!     thread (`resume bypass`, `stop`) resolves to the right job
//!     id without the operator retyping it.
//!
//! All transport-agnostic pieces (parser, reply renderers, failure
//! renderers, dispatcher, publisher, debouncer, REVIEW cache) live
//! in [`codeless_bot_core`]; this crate only owns Telegram-specific
//! bits: the Bot API HTTP wrapper, the long-poll loop, the
//! MarkdownV2 framing, and the secrets reader.
//!
//! Host-only per R1 (the long-poll loop calls reqwest). The crate is
//! excluded from the mobile-safe column of the crate table because
//! mobile shells reach the same RPC surface through the HTTP/SSE
//! transport — they do not need the Telegram bridge.

pub mod config;
pub mod dispatcher;
pub mod long_poll;
pub mod outbound;
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

pub use config::{
    TelegramConfig, TelegramConfigError, TELEGRAM_BOT_TOKEN_KEY, TELEGRAM_CHAT_ID_KEY,
};
pub use web_api::{TelegramApi, WebApiError};

/// Handle to a running Telegram bot. Dropping it leaves the spawned
/// tasks running; [`TelegramBot::shutdown`] signals both the inbound
/// long-poll loop and the outbound publisher to exit at their next
/// event boundary and waits for the joins.
pub struct TelegramBot {
    join: JoinHandle<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    threads: ThreadMap,
    outbound: Option<OutboundPublisher>,
}

impl TelegramBot {
    /// Boot a Telegram adapter against the supplied config. The
    /// returned handle keeps both halves alive in the background;
    /// the caller is expected to hold it for the server's lifetime.
    /// A fresh `reqwest::Client` is constructed internally so the
    /// adapter never trips over a different host crate's pool
    /// configuration (the slack adapter exposes a `_with` variant for
    /// the same reason; Telegram has fewer reasons to share a client
    /// because every call is a single round-trip).
    pub fn spawn(config: TelegramConfig, rpc: Arc<dyn RpcServer>) -> Result<Self, WebApiError> {
        let api = TelegramApi::new(&config.bot_token)?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let backend: Arc<dyn CommandBackend> = Arc::new(RpcServerBackend::new(rpc.clone()));
        let transport: Arc<dyn BotTransport> = Arc::new(api.clone());
        let threads = ThreadMap::new();
        let dispatcher = Dispatcher::new(backend.clone(), transport.clone(), threads.clone());

        // Outbound publisher only spawns when a chat is configured —
        // a deployment that wants commands-only behaviour leaves
        // `telegram_chat_id` unset and the publisher is simply
        // absent. A WARN log makes the inert state visible on boot.
        let outbound = match config.chat_id.clone() {
            Some(chat_id) => {
                let events: Arc<dyn EventSource> = Arc::new(RpcServerEventSource::new(rpc));
                let pre_transport: Arc<dyn BotTransport> =
                    Arc::new(outbound::MarkdownV2Transport::new(api.clone()));
                Some(OutboundPublisher::spawn(
                    OutboundConfig::new(chat_id),
                    events,
                    backend,
                    pre_transport,
                    threads.clone(),
                ))
            }
            None => {
                tracing::warn!(
                    "telegram: no telegram_chat_id configured; outbound failure notifications disabled"
                );
                None
            }
        };

        let join = tokio::spawn(async move {
            long_poll::run(api, dispatcher, shutdown_rx).await;
        });
        Ok(Self {
            join,
            shutdown_tx: Some(shutdown_tx),
            threads,
            outbound,
        })
    }

    /// Snapshot of the outbound thread map. Symmetrical with
    /// [`codeless_slack::SlackBot::thread_map`] — exposed so a future
    /// test harness can inspect the registered notification threads.
    pub fn thread_map(&self) -> ThreadMap {
        self.threads.clone()
    }

    /// Signal both loops to exit, then await the tasks. Idempotent:
    /// a second call returns immediately because the signal channel
    /// is already consumed. The outbound publisher (when present) is
    /// drained in the same call so a clean server shutdown joins
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
