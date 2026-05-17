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
    ) -> Result<(), SlackPostError> {
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
        Ok(())
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
    async fn happy_path_returns_ok_on_slack_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .and(header("Authorization", "Bearer xoxb-test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "channel": "C1"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let poster = poster_against(&server);
        poster
            .post("C1", "hello", None)
            .await
            .expect("happy path should succeed");
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
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let poster = poster_against(&server);
        poster
            .post("C1", "hello", Some("1700.0001"))
            .await
            .expect("thread_ts path should succeed");
    }

    #[tokio::test]
    async fn slack_ok_false_surfaces_as_slack_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"ok": false, "error": "not_in_channel"}),
            ))
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
