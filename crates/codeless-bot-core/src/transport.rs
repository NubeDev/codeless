//! Trait every chat transport implements so the [`crate::dispatcher`]
//! and [`crate::outbound`] modules can stay transport-agnostic.
//!
//! Slack and Telegram both speak a "post one message into a chat,
//! optionally as a reply to a prior message, optionally edit a
//! prior post" shape — the wire formats differ but the semantics
//! collapse to one trait. `chat` is the channel id (Slack `C…`) /
//! chat id (Telegram integer rendered as a string). `reply_to` is
//! the Slack `thread_ts` / Telegram `reply_to_message_id` /
//! `message_thread_id`, also rendered as a string so the trait can
//! stay one-shape across both.
//!
//! `post` returns a [`PostedMessage`] carrying the canonical chat id
//! the server resolved (Slack rewrites `#name` → `C…`; Telegram
//! returns the same integer back unchanged) plus the new message's
//! id (Slack `ts`, Telegram `message_id`). The outbound publisher
//! registers that pair in [`crate::ThreadMap`] so a subsequent
//! reply-to / in-thread message resolves back to the failing
//! job id without the operator retyping it.

use async_trait::async_trait;
use thiserror::Error;

#[async_trait]
pub trait BotTransport: Send + Sync + 'static {
    /// Post `text` into `chat`. When `reply_to` is `Some`, the
    /// adapter threads the new message under the referenced parent
    /// (Slack `thread_ts`, Telegram `reply_to_message_id`). The
    /// returned id is the new message's own identifier — for a
    /// top-level post it doubles as the parent id of future replies,
    /// which is the value the outbound publisher records in the
    /// thread map.
    async fn post(
        &self,
        chat: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<PostedMessage, BotPostError>;
}

/// Outcome of a successful `post`. The `chat` field carries whatever
/// the server echoed back (Slack canonicalises channel names; Telegram
/// returns the integer chat id unchanged); the `id` is the new
/// message's identifier. Both are kept as strings so the trait does
/// not have to grow per-transport id types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedMessage {
    pub chat: String,
    pub id: String,
}

/// Transport error surface. Adapters map their own HTTP / API
/// failures into one of these so the publisher and the dispatcher do
/// not have to match on a transport-specific variant.
#[derive(Debug, Error)]
pub enum BotPostError {
    /// Network-level / I/O error talking to the chat platform. The
    /// payload is a stringified form of the underlying error so the
    /// trait does not have to depend on `reqwest` (or any specific
    /// HTTP client) — the dispatcher logs it and moves on.
    #[error("transport error: {0}")]
    Transport(String),
    /// Platform returned a non-2xx HTTP status. Adapters use this
    /// when there is no structured error body to surface.
    #[error("HTTP {status}")]
    HttpStatus { status: u16 },
    /// Platform accepted the request but signalled failure in the
    /// response body (Slack `ok: false` with an `error` label,
    /// Telegram `ok: false` with `description`). The `Option<String>`
    /// carries the platform-supplied label when present so an
    /// operator-visible log line can echo it.
    #[error("API returned error: {0:?}")]
    Api(Option<String>),
}
