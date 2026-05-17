//! Telegram control-plane adapter. Sits next to `codeless-server`
//! and exposes the same operator surface as `codeless-slack`, but
//! over the Telegram Bot API.
//!
//! Transport differences vs Slack (full design:
//! `DOCS/SCOPE-TELEGRAM-INTEGRATION.md`):
//!
//!   - Inbound: long-polling via `getUpdates` (no public HTTPS
//!     callback). Webhook is intentionally out of scope.
//!   - Outbound: `sendMessage` with `parse_mode = MarkdownV2`. Job
//!     ids and other operator-supplied strings must be wrapped in a
//!     preformatted block to bypass MarkdownV2 escaping.
//!   - Thread context: forum topics use `message_thread_id`; plain
//!     chats fall back to `reply_to_message.message_id`.
//!
//! The parser, `BotTransport` trait, renderers, `ThreadMap`, and
//! `ReviewCache` live in `codeless-bot-core` and are shared with
//! `codeless-slack`. This crate only owns the Telegram-specific
//! transport layer (HTTP client + long-poll loop + sendMessage
//! wrapper) and the `TelegramBot::spawn` entry point that the CLI
//! calls when `--enable-telegram` is set.
//!
//! Host-only per R1.

pub mod config;
pub mod dispatcher;
pub mod long_poll;
pub mod outbound;
pub mod web_api;

pub use config::TelegramConfig;

/// Entry point invoked from `codeless-cli` when `--enable-telegram`
/// is passed. Holds the long-poll task handle and the outbound
/// publisher handle so the caller can keep them alive for the
/// process lifetime.
pub struct TelegramBot {
    _long_poll: tokio::task::JoinHandle<()>,
    _outbound: Option<tokio::task::JoinHandle<()>>,
}

impl TelegramBot {
    /// Wire the long-poll loop and (if a chat id is configured) the
    /// outbound publisher. Mirrors `codeless_slack::SlackBot::spawn`.
    pub fn spawn(
        _cfg: TelegramConfig,
        _rpc: std::sync::Arc<dyn codeless_rpc::RpcServer>,
    ) -> Self {
        todo!("wire long_poll::run + outbound::run; see codeless-slack::SlackBot::spawn")
    }
}
