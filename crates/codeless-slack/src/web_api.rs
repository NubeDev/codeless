//! Thin wrapper over the Slack Web API methods the dispatcher needs.
//! Only `chat.postMessage` is wired in stage 4 — outbound failure
//! notifications (stage 6) will share the same endpoint with a
//! richer payload. The wrapper exists so the dispatcher does not
//! grow ad-hoc reqwest call sites and so tests can point a single
//! base URL at a wiremock server without juggling per-method URLs.
//!
//! The endpoint is documented at <https://api.slack.com/methods/chat.postMessage>.
//! Slack accepts JSON or form-encoded bodies; we send JSON because
//! `thread_ts` round-trips cleanly without quoting. Auth is the bot
//! `xoxb-…` token in the `Authorization` header — the app-level
//! `xapp-…` token used to open Socket Mode has no `chat:write`
//! scope, so swapping them produces a 200-with-ok=false response
//! that surfaces here as `SlackApi`.

use async_trait::async_trait;
use codeless_bot_core::transport::{
    BotPostError, BotTransport, PostedMessage as CorePostedMessage,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default Slack Web API base. Tests override via
/// [`ChatPoster::with_base_url`] so the dispatcher does not have to
/// know about the override.
const DEFAULT_BASE_URL: &str = "https://slack.com/api";

#[derive(Debug, Error)]
pub enum SlackPostError {
    #[error("chat.postMessage transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("chat.postMessage returned HTTP {status}")]
    HttpStatus { status: u16 },
    /// Slack's API returns 200 with `ok: false` and an `error` field
    /// for permission / scope problems (`not_in_channel`,
    /// `channel_not_found`, `invalid_auth`). Propagating the raw error
    /// label gives the operator (and the tests) something specific
    /// to grep for; the dispatcher logs it and moves on.
    #[error("chat.postMessage returned ok=false (error={0:?})")]
    SlackApi(Option<String>),
}

/// Handle that holds the bot token plus the shared `reqwest::Client`.
/// One handle is built per `SlackBot` and reused for every reply —
/// reqwest's pool keeps the TLS handshake costs bounded.
#[derive(Debug, Clone)]
pub struct ChatPoster {
    http: std::sync::Arc<reqwest::Client>,
    bot_token: String,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct ChatPostMessageRequest<'a> {
    channel: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct ChatPostMessageResponse {
    ok: bool,
    error: Option<String>,
    /// The Slack-side timestamp of the posted message. For a top-level
    /// post the value doubles as the `thread_ts` of every reply in the
    /// thread — the outbound failure publisher (stage 6) keys its
    /// `ThreadMap` registration off this field so that bare-verb
    /// replies (`resume bypass`, `stop`) inside the notification
    /// thread resolve to the failing job's id. A reply-post returns
    /// its own ts; the publisher discards it because the parent
    /// thread is already mapped.
    ts: Option<String>,
}

/// Outcome of a successful `chat.postMessage` call. The `ts` is the
/// Slack-side message timestamp; for a top-level post it doubles as
/// the `thread_ts` of every subsequent reply, which is why the
/// outbound failure publisher (stage 6) registers it in the
/// [`crate::ThreadMap`] right after posting the notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedMessage {
    /// Channel the post landed in. Echoed back from Slack — Slack
    /// resolves channel names server-side, so a caller that passed a
    /// `#name` form gets the canonical `C…` id here.
    pub channel: String,
    /// Slack-side message timestamp. Used as the `thread_ts` of
    /// in-thread replies.
    pub ts: String,
}

impl ChatPoster {
    pub fn new(http: std::sync::Arc<reqwest::Client>, bot_token: impl Into<String>) -> Self {
        Self {
            http,
            bot_token: bot_token.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Override the Slack base URL. Used by tests to point at a
    /// wiremock server; production callers should not touch this.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
    }

    /// Post a plain-text message into `channel`, optionally as a reply
    /// to a thread. `thread_ts` for a top-level post is `None`; for an
    /// in-thread reply it carries the parent message's `ts`. The
    /// dispatcher hands its own thread context here so a reply to a
    /// notification stays inside the notification's thread (Risk 1's
    /// safety-net echo lands next to the message it answers, not at
    /// the bottom of the channel).
    pub async fn post(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<PostedMessage, SlackPostError> {
        let url = format!("{}/chat.postMessage", self.base_url);
        let body = ChatPostMessageRequest {
            channel,
            text,
            thread_ts,
        };
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SlackPostError::HttpStatus {
                status: status.as_u16(),
            });
        }
        let payload: ChatPostMessageResponse = resp.json().await?;
        if !payload.ok {
            return Err(SlackPostError::SlackApi(payload.error));
        }
        // `ts` should always be present on `ok = true` per Slack's
        // documented response; treat a missing value as a Slack-side
        // protocol break rather than a silent no-op so the outbound
        // publisher's ThreadMap registration never sees a phantom
        // post.
        let ts = payload
            .ts
            .ok_or_else(|| SlackPostError::SlackApi(Some("missing-ts".to_string())))?;
        Ok(PostedMessage {
            channel: channel.to_string(),
            ts,
        })
    }
}

/// Map a [`SlackPostError`] onto the transport-agnostic
/// [`BotPostError`] surface so the [`codeless_bot_core::Dispatcher`]
/// and [`codeless_bot_core::OutboundPublisher`] never have to match on
/// Slack-specific variants. `Transport` carries the underlying
/// `reqwest::Error` as a string (the trait does not depend on
/// reqwest); the labelled API and HTTP-status arms map across
/// directly.
impl From<SlackPostError> for BotPostError {
    fn from(err: SlackPostError) -> Self {
        match err {
            SlackPostError::Transport(e) => BotPostError::Transport(e.to_string()),
            SlackPostError::HttpStatus { status } => BotPostError::HttpStatus { status },
            SlackPostError::SlackApi(label) => BotPostError::Api(label),
        }
    }
}

/// Wire [`ChatPoster`] into the transport-agnostic
/// [`codeless_bot_core::BotTransport`] surface so the shared
/// dispatcher and outbound publisher can drive Slack the same way
/// they drive Telegram. `reply_to` maps to Slack's `thread_ts`.
#[async_trait]
impl BotTransport for ChatPoster {
    async fn post(
        &self,
        chat: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<CorePostedMessage, BotPostError> {
        let posted = ChatPoster::post(self, chat, text, reply_to).await?;
        Ok(CorePostedMessage {
            chat: posted.channel,
            id: posted.ts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn poster_against(server: &MockServer) -> ChatPoster {
        let http = Arc::new(reqwest::Client::new());
        ChatPoster::new(http, "xoxb-test").with_base_url(server.uri() + "/api")
    }

    #[tokio::test]
    async fn happy_path_returns_posted_message_with_ts() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .and(header("Authorization", "Bearer xoxb-test"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"ok": true, "channel": "C1", "ts": "1700.0042"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let poster = poster_against(&server);
        let posted = poster
            .post("C1", "hello", None)
            .await
            .expect("happy path should succeed");
        assert_eq!(posted.channel, "C1");
        assert_eq!(posted.ts, "1700.0042");
    }

    #[tokio::test]
    async fn thread_ts_is_forwarded_when_set() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .and(wiremock::matchers::body_partial_json(
                serde_json::json!({"thread_ts": "1700.0001"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "ts": "1700.0009"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let poster = poster_against(&server);
        let posted = poster
            .post("C1", "hello", Some("1700.0001"))
            .await
            .expect("thread_ts path should succeed");
        // The reply's ts (not the parent's) is what Slack returns here;
        // the outbound publisher discards it because the parent thread
        // is already mapped — only the assertion shape matters.
        assert_eq!(posted.ts, "1700.0009");
    }

    #[tokio::test]
    async fn missing_ts_surfaces_as_slack_api_error() {
        // A `ok: true` response with no `ts` is a Slack-side protocol
        // break (the field is documented as always present on success).
        // Surfacing it as `SlackApi("missing-ts")` keeps the outbound
        // publisher from registering a phantom thread.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;
        let poster = poster_against(&server);
        let err = poster.post("C1", "hello", None).await.unwrap_err();
        match err {
            SlackPostError::SlackApi(Some(label)) => assert_eq!(label, "missing-ts"),
            other => panic!("expected SlackApi(missing-ts), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn slack_ok_false_surfaces_as_slack_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": false, "error": "not_in_channel"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let poster = poster_against(&server);
        let err = poster.post("C1", "hello", None).await.unwrap_err();
        match err {
            SlackPostError::SlackApi(Some(label)) => assert_eq!(label, "not_in_channel"),
            other => panic!("expected SlackApi(not_in_channel), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_2xx_surfaces_as_http_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let poster = poster_against(&server);
        let err = poster.post("C1", "hello", None).await.unwrap_err();
        assert!(matches!(err, SlackPostError::HttpStatus { status: 500 }));
    }
}
