//! Thin wrapper around the Bot API HTTP endpoints used by the
//! Telegram surface. Counterpart of `codeless_slack::web_api`.
//!
//! Base URL shape: `https://api.telegram.org/bot<token>/<method>`.
//! Every response is wrapped in `{ok, result?, description?,
//! error_code?}`; we surface `ok=false` as [`WebApiError::Api`] so
//! the dispatcher (and the bot-core publisher via
//! [`BotTransport`]) gets a structured failure rather than a 200 it
//! has to disambiguate.
//!
//! The four methods wired here are the ones the inbound dispatcher
//! and outbound publisher actually call:
//!
//! - `getMe` — startup only; surfaces the bot's own username so the
//!   dispatcher can strip a leading `@username` mention from inbound
//!   text without a per-message round-trip.
//! - `sendMessage` — every command reply and every outbound
//!   notification.
//! - `editMessageText` — kept for the outbound publisher's
//!   not-yet-wired follow-up edits (REVIEW verdict landing after the
//!   initial failure card).
//! - `getUpdates` — drives the long-poll loop; constants for the
//!   long-poll timeout live alongside the method.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use codeless_bot_core::transport::{
    BotPostError, BotTransport, PostedMessage as CorePostedMessage,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default Bot API host. Tests override via
/// [`TelegramApi::with_base_url`] so the dispatcher never has to know
/// about the override.
const DEFAULT_BASE_URL: &str = "https://api.telegram.org";

/// Long-poll timeout sent to `getUpdates`. Telegram caps the upper
/// bound at 50s; 30s matches the value the SCOPE doc commits to and
/// keeps the worst-case shutdown latency bounded.
pub const LONG_POLL_TIMEOUT_SECS: u64 = 30;

/// Per-request HTTP client timeout. Must exceed
/// [`LONG_POLL_TIMEOUT_SECS`] by a margin large enough to cover the
/// last-chunk flush from Telegram; 45s is the value the slack
/// adapter uses for the equivalent Socket Mode ack window.
const HTTP_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Error)]
pub enum WebApiError {
    #[error("telegram transport error: {0}")]
    Http(#[from] reqwest::Error),
    /// Telegram returned a 200 wrapping `ok=false`. The description
    /// field carries the human-readable label (`Bad Request:
    /// message thread not found`, `Forbidden: bot was kicked`, …).
    /// Mirrors `SlackPostError::SlackApi` so the cross-platform
    /// [`From`] impl onto [`BotPostError::Api`] stays mechanical.
    #[error("telegram api: {description}")]
    Api { description: String },
    #[error("telegram api returned HTTP {status}")]
    HttpStatus { status: u16 },
}

/// Subset of the `User` object `getMe` returns. The dispatcher only
/// needs the username (for the mention-strip path); the numeric id
/// is kept around for log lines that operators reach for when a
/// token suddenly stops working ("which bot is this token?").
#[derive(Debug, Clone, Deserialize)]
pub struct BotUser {
    pub id: i64,
    pub username: Option<String>,
}

/// Argument struct for `sendMessage`. The four optional fields cover
/// every payload shape this crate sends; a future richer
/// notification (buttons, photo) would add a new method rather than
/// growing this struct.
#[derive(Debug, Serialize)]
pub struct SendMessageArgs<'a> {
    pub chat_id: &'a str,
    pub text: &'a str,
    /// `Some("MarkdownV2")` for outbound failure cards;
    /// `None` for plain command replies. MarkdownV2 escaping is the
    /// caller's responsibility — the dispatcher renders plain text
    /// and skips the field, the outbound publisher wraps its body
    /// in a triple-backtick block before passing it here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<&'a str>,
    /// When the inbound message arrived as a reply (not a forum
    /// topic), echo the same `message_id` here so the reply stays
    /// threaded under the parent. Bot API rejects this field for a
    /// post into a non-existent message with a `Bad Request`; the
    /// dispatcher omits it for cold threads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
    /// For forum topics (`is_forum=true` supergroups) the platform
    /// expects `message_thread_id` instead of `reply_to_message_id`
    /// to keep a post inside the same topic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_thread_id: Option<i64>,
}

/// Outcome of a successful `sendMessage`. The `message_id` is the
/// Telegram-side id the [`codeless_bot_core::ThreadMap`] keys on so
/// a subsequent bare-verb reply in the same thread resolves to the
/// notification's job id without the operator retyping it. The chat
/// id is echoed back as a string so the result composes with
/// [`CorePostedMessage`] without an i64 round-trip the publisher
/// would just stringify again.
#[derive(Debug, Clone, Deserialize)]
pub struct SentMessage {
    pub message_id: i64,
    pub chat: Chat,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
}

/// Argument bundle for the `getUpdates` long-poll. `offset` is the
/// `update_id + 1` of the last message we processed; Telegram uses it
/// to ack everything strictly below it. `allowed_updates` narrows
/// the stream to the two kinds the dispatcher actually consumes.
#[derive(Debug, Serialize)]
pub struct GetUpdatesArgs<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    pub timeout: u64,
    pub allowed_updates: &'a [&'a str],
}

/// One element of the `getUpdates` response. Only the two fields the
/// dispatcher uses are surfaced; the rest of the envelope is left
/// off so a new field upstream does not break deserialization.
#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<UpdateMessage>,
}

/// One incoming `message` update. Fields are the subset the
/// dispatcher reads — the long-poll loop projects each one onto
/// [`codeless_bot_core::InboundMessage`] before handing it off.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMessage {
    pub message_id: i64,
    pub chat: UpdateChat,
    pub from: Option<UpdateFrom>,
    pub text: Option<String>,
    pub reply_to_message: Option<ReplyToMessage>,
    /// Set on supergroup forum topics. When present, takes priority
    /// over `reply_to_message.message_id` for thread resolution.
    pub message_thread_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateChat {
    pub id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateFrom {
    pub id: i64,
}

/// Slimmed-down `reply_to_message` carrying just the id the
/// dispatcher needs to resolve thread context against the
/// [`codeless_bot_core::ThreadMap`].
#[derive(Debug, Clone, Deserialize)]
pub struct ReplyToMessage {
    pub message_id: i64,
}

/// Bot API response envelope. `result` is present iff `ok=true`;
/// `description` is present iff `ok=false`. A missing `description`
/// on an `ok=false` response (Telegram is supposed to always send
/// one) is rendered as a generic label by the call sites.
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

/// Handle holding the shared `reqwest::Client` plus the URL-baked
/// bot token. Cloning is cheap — the client is `Arc`-pooled — so the
/// long-poll loop and the outbound publisher can hold independent
/// handles without contention.
#[derive(Debug, Clone)]
pub struct TelegramApi {
    http: Arc<reqwest::Client>,
    /// Pre-built `https://host/bot<token>` prefix; method names are
    /// appended at call time. Keeps the token out of every per-call
    /// `format!` and avoids accidentally logging it.
    method_base: String,
}

impl TelegramApi {
    /// Build a handle with the default Bot API host. The client is
    /// configured with [`HTTP_TIMEOUT`] which is the loose upper
    /// bound for any single call (long-poll included); transient
    /// network errors come back as [`WebApiError::Http`].
    pub fn new(bot_token: impl AsRef<str>) -> Result<Self, WebApiError> {
        let http = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;
        Ok(Self::from_parts(
            Arc::new(http),
            bot_token,
            DEFAULT_BASE_URL,
        ))
    }

    /// Like [`Self::new`] but reuses a caller-built client (test
    /// harnesses, future shared host client). The base URL stays the
    /// public Bot API endpoint; tests override via
    /// [`Self::with_base_url`].
    pub fn new_with_client(http: Arc<reqwest::Client>, bot_token: impl AsRef<str>) -> Self {
        Self::from_parts(http, bot_token, DEFAULT_BASE_URL)
    }

    /// Override the Bot API base URL. Used by the wiremock-backed
    /// tests; production callers should not touch this.
    pub fn with_base_url(self, base_url: impl AsRef<str>) -> Self {
        let tail = self
            .method_base
            .rsplit_once("/bot")
            .map(|(_, token)| token)
            .unwrap_or("");
        Self {
            http: self.http,
            method_base: format!("{}/bot{}", base_url.as_ref().trim_end_matches('/'), tail),
        }
    }

    fn from_parts(http: Arc<reqwest::Client>, bot_token: impl AsRef<str>, base_url: &str) -> Self {
        Self {
            http,
            method_base: format!(
                "{}/bot{}",
                base_url.trim_end_matches('/'),
                bot_token.as_ref()
            ),
        }
    }

    fn method_url(&self, method: &str) -> String {
        format!("{}/{}", self.method_base, method)
    }

    /// `getMe` — startup-only; surfaces the bot's username so the
    /// dispatcher can strip a leading `@username` mention without a
    /// per-message round-trip.
    pub async fn get_me(&self) -> Result<BotUser, WebApiError> {
        let resp = self.http.get(self.method_url("getMe")).send().await?;
        decode::<BotUser>(resp).await
    }

    /// `sendMessage`. Used both by the inbound reply path and by the
    /// outbound publisher (through [`BotTransport::post`]).
    pub async fn send_message(
        &self,
        args: SendMessageArgs<'_>,
    ) -> Result<SentMessage, WebApiError> {
        let resp = self
            .http
            .post(self.method_url("sendMessage"))
            .json(&args)
            .send()
            .await?;
        decode::<SentMessage>(resp).await
    }

    /// `editMessageText`. Kept for the outbound publisher's REVIEW
    /// verdict follow-up; not yet wired but exposed so the publisher
    /// does not grow ad-hoc reqwest call sites later.
    pub async fn edit_message_text(
        &self,
        chat_id: &str,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<(), WebApiError> {
        #[derive(Serialize)]
        struct EditArgs<'a> {
            chat_id: &'a str,
            message_id: i64,
            text: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            parse_mode: Option<&'a str>,
        }
        let resp = self
            .http
            .post(self.method_url("editMessageText"))
            .json(&EditArgs {
                chat_id,
                message_id,
                text,
                parse_mode,
            })
            .send()
            .await?;
        decode::<serde_json::Value>(resp).await?;
        Ok(())
    }

    /// `getUpdates` long-poll. The HTTP timeout is set above the
    /// `timeout` argument by [`HTTP_TIMEOUT`] so the server's own
    /// long-poll always wins over the client's request timer.
    pub async fn get_updates(
        &self,
        offset: Option<i64>,
        timeout: u64,
    ) -> Result<Vec<Update>, WebApiError> {
        let args = GetUpdatesArgs {
            offset,
            timeout,
            allowed_updates: &["message"],
        };
        let resp = self
            .http
            .post(self.method_url("getUpdates"))
            .json(&args)
            .send()
            .await?;
        decode::<Vec<Update>>(resp).await
    }
}

/// Decode a Bot API response into either the typed result or a
/// labelled [`WebApiError`]. Pulled out so every method has the same
/// 200/`ok=false`/HTTP-status handling.
async fn decode<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T, WebApiError> {
    let status = resp.status();
    if !status.is_success() {
        return Err(WebApiError::HttpStatus {
            status: status.as_u16(),
        });
    }
    let body: ApiResponse<T> = resp.json().await?;
    if !body.ok {
        return Err(WebApiError::Api {
            description: body
                .description
                .unwrap_or_else(|| "missing description".to_string()),
        });
    }
    body.result.ok_or(WebApiError::Api {
        description: "missing result on ok=true".to_string(),
    })
}

/// Map the Telegram-flavoured error onto the transport-agnostic
/// [`BotPostError`] surface so the bot-core dispatcher and publisher
/// never branch on Telegram-specific variants. The HTTP and labelled
/// API arms map across directly; the underlying `reqwest::Error` is
/// flattened to a string because the trait does not depend on
/// reqwest.
impl From<WebApiError> for BotPostError {
    fn from(err: WebApiError) -> Self {
        match err {
            WebApiError::Http(e) => BotPostError::Transport(e.to_string()),
            WebApiError::HttpStatus { status } => BotPostError::HttpStatus { status },
            WebApiError::Api { description } => BotPostError::Api(Some(description)),
        }
    }
}

/// Wire [`TelegramApi`] into the transport-agnostic
/// [`BotTransport`] surface. `chat` is the chat id rendered as a
/// string (the bot-core surface is platform-neutral); `reply_to` is
/// the parent message id as a string. The string-to-i64 parse is
/// best-effort — a non-numeric `reply_to` (which would only happen
/// if a future adapter started recording slack-shaped values into
/// the same ThreadMap by mistake) is silently dropped rather than
/// failing the whole post.
#[async_trait]
impl BotTransport for TelegramApi {
    async fn post(
        &self,
        chat: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<CorePostedMessage, BotPostError> {
        let reply_to_message_id = reply_to.and_then(|s| s.parse::<i64>().ok());
        let sent = self
            .send_message(SendMessageArgs {
                chat_id: chat,
                text,
                parse_mode: None,
                reply_to_message_id,
                message_thread_id: None,
            })
            .await?;
        Ok(CorePostedMessage {
            chat: sent.chat.id.to_string(),
            id: sent.message_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn api(server: &MockServer) -> TelegramApi {
        TelegramApi::new_with_client(Arc::new(reqwest::Client::new()), "12345:test-secret")
            .with_base_url(server.uri())
    }

    #[tokio::test]
    async fn get_me_returns_username_on_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot12345:test-secret/getMe"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": { "id": 8882032367i64, "username": "aidan_codeless_bot" }
            })))
            .mount(&server)
            .await;
        let me = api(&server).get_me().await.expect("ok");
        assert_eq!(me.id, 8882032367);
        assert_eq!(me.username.as_deref(), Some("aidan_codeless_bot"));
    }

    #[tokio::test]
    async fn send_message_round_trips_payload_and_returns_message_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot12345:test-secret/sendMessage"))
            .and(body_json(json!({
                "chat_id": "555",
                "text": "hi",
                "reply_to_message_id": 42i64,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": { "message_id": 99i64, "chat": { "id": 555i64 } }
            })))
            .mount(&server)
            .await;
        let sent = api(&server)
            .send_message(SendMessageArgs {
                chat_id: "555",
                text: "hi",
                parse_mode: None,
                reply_to_message_id: Some(42),
                message_thread_id: None,
            })
            .await
            .expect("ok");
        assert_eq!(sent.message_id, 99);
        assert_eq!(sent.chat.id, 555);
    }

    #[tokio::test]
    async fn ok_false_surfaces_description() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot12345:test-secret/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": false,
                "description": "Forbidden: bot was blocked by the user",
                "error_code": 403,
            })))
            .mount(&server)
            .await;
        let err = api(&server)
            .send_message(SendMessageArgs {
                chat_id: "555",
                text: "hi",
                parse_mode: None,
                reply_to_message_id: None,
                message_thread_id: None,
            })
            .await
            .unwrap_err();
        match err {
            WebApiError::Api { description } => assert!(description.contains("Forbidden")),
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_updates_returns_typed_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot12345:test-secret/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": [
                    {
                        "update_id": 11,
                        "message": {
                            "message_id": 7,
                            "chat": { "id": 555 },
                            "from": { "id": 42 },
                            "text": "status",
                            "reply_to_message": null,
                            "message_thread_id": null,
                        }
                    }
                ]
            })))
            .mount(&server)
            .await;
        let updates = api(&server).get_updates(None, 0).await.expect("ok");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 11);
        let msg = updates[0].message.as_ref().expect("message present");
        assert_eq!(msg.chat.id, 555);
        assert_eq!(msg.text.as_deref(), Some("status"));
    }

    #[tokio::test]
    async fn bot_transport_impl_maps_chat_and_reply_to() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot12345:test-secret/sendMessage"))
            .and(body_json(json!({
                "chat_id": "987",
                "text": "[ok] done",
                "reply_to_message_id": 17i64,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": { "message_id": 24i64, "chat": { "id": 987i64 } }
            })))
            .mount(&server)
            .await;
        let posted = BotTransport::post(&api(&server), "987", "[ok] done", Some("17"))
            .await
            .expect("ok");
        assert_eq!(posted.chat, "987");
        assert_eq!(posted.id, "24");
    }

    #[tokio::test]
    async fn bot_transport_impl_drops_non_numeric_reply_to() {
        // A `ThreadMap` populated by a different adapter would never
        // happen in practice, but the impl must not propagate a parse
        // failure as a transport error — the post should still land
        // as a fresh top-level message.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot12345:test-secret/sendMessage"))
            .and(body_json(json!({
                "chat_id": "987",
                "text": "hi",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": { "message_id": 1i64, "chat": { "id": 987i64 } }
            })))
            .mount(&server)
            .await;
        BotTransport::post(&api(&server), "987", "hi", Some("not-a-number"))
            .await
            .expect("ok");
    }
}
