//! Transport-agnostic core for the Codeless chat bots.
//!
//! Two adapters sit on top: [`codeless-slack`](../codeless_slack/)
//! and [`codeless-telegram`](../codeless_telegram/). Both speak to
//! Codeless through the same in-process [`codeless_rpc::RpcServer`]
//! handle, parse the same five-verb grammar, post the same plain-text
//! replies, and emit the same failure notifications. The bits that
//! actually differ between the two — how a message is sent over the
//! wire, how a thread is identified, how the inbound transport is
//! pumped — live in the per-transport crate. Everything else is
//! shared so a bug fix in the parser or a tweak to a notification's
//! shape lands once.
//!
//! The split is:
//!
//!   - [`command`] — the message body parser (`/status`, `/stop …`,
//!     `/resume … bypass "<comment>"`), the [`command::Command`]
//!     enum, and [`command::ThreadContext`].
//!   - [`reply`] — synchronous reply renderers for each command.
//!   - [`notify`] — outbound failure-notification renderers.
//!   - [`thread_map`] — `(chat, in-reply-to id) -> JobId` cache the
//!     outbound publisher writes and the dispatcher reads.
//!   - [`transport`] — the [`transport::BotTransport`] trait every
//!     adapter implements plus the small `PostedMessage` /
//!     `BotPostError` types the trait returns.
//!   - [`dispatcher`] — generic dispatcher that takes a
//!     [`transport::BotTransport`] and a [`dispatcher::CommandBackend`]
//!     (the RPC seam) and turns one inbound message into one reply
//!     post.
//!   - [`outbound`] — generic event-bus subscriber that posts a
//!     debounced notification per terminal `JobFailed` / `JobStopped`
//!     and registers each post's id in the [`thread_map::ThreadMap`].
//!
//! Host-only per the workspace's R1 rule — the crate carries no
//! `std::process` / `tokio::process` calls and is excluded from the
//! mobile-safe column of the crate table.

pub mod alias_map;
pub mod command;
pub mod dispatcher;
pub mod notify;
pub mod outbound;
pub mod reply;
pub mod thread_map;
pub mod transport;

pub use alias_map::AliasMap;
pub use command::{parse as parse_command, Command, ParseError, ThreadContext};
pub use dispatcher::{
    CommandBackend, Dispatcher, DispatcherConfig, InboundMessage, RpcServerBackend,
};
pub use notify::ReviewContext;
pub use outbound::{
    EventSource, OutboundConfig, OutboundPublisher, RpcServerEventSource, DEBOUNCE_WINDOW,
    REVIEW_CACHE_CAPACITY,
};
pub use thread_map::ThreadMap;
pub use transport::{BotPostError, BotTransport, PostedMessage};
