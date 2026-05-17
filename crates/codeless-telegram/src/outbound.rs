//! Outbound transport adapter for Telegram.
//!
//! The transport-agnostic publisher lives in
//! [`codeless_bot_core::OutboundPublisher`] — it owns the event
//! subscription, the debounce, the REVIEW cache, and the
//! [`codeless_bot_core::notify`] renderer. This module exists to
//! wrap a [`crate::web_api::TelegramApi`] with two pieces of
//! Telegram-specific behaviour so the publisher's `BotTransport`
//! contract still works:
//!
//!   1. Outbound notification bodies are wrapped in a
//!      triple-backtick block before being sent. Telegram's
//!      MarkdownV2 parser otherwise treats `_`, `*`, `[`, `]`, `(`,
//!      `)`, `~`, `` ` ``, `>`, `#`, `+`, `-`, `=`, `|`, `{`, `}`,
//!      `.`, `!` as control characters; failing to escape any of
//!      them surfaces as a `Bad Request: can't parse entities`.
//!      Wrapping the whole body in a pre block sidesteps the
//!      escape table entirely while still rendering the failure
//!      card monospaced (which is what an operator wants for a
//!      Reason / Stage column anyway). The pre block only requires
//!      escaping `` ` `` and `\\` inside the content.
//!   2. `parse_mode` is set to `MarkdownV2` so the pre block
//!      actually renders as one.
//!
//! Command replies do not go through this wrapper — they use the
//! plain [`BotTransport`] impl on [`TelegramApi`] directly so they
//! land as ordinary text without the monospaced framing.

use async_trait::async_trait;
use codeless_bot_core::transport::{
    BotPostError, BotTransport, PostedMessage as CorePostedMessage,
};

use crate::web_api::{SendMessageArgs, TelegramApi};

/// `BotTransport` wrapper that posts every body as a MarkdownV2 pre
/// block. Hold one per [`codeless_bot_core::OutboundPublisher`].
#[derive(Debug, Clone)]
pub struct MarkdownV2Transport {
    api: TelegramApi,
}

impl MarkdownV2Transport {
    pub fn new(api: TelegramApi) -> Self {
        Self { api }
    }
}

#[async_trait]
impl BotTransport for MarkdownV2Transport {
    async fn post(
        &self,
        chat: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<CorePostedMessage, BotPostError> {
        let wrapped = wrap_pre(text);
        let reply_to_message_id = reply_to.and_then(|s| s.parse::<i64>().ok());
        let sent = self
            .api
            .send_message(SendMessageArgs {
                chat_id: chat,
                text: &wrapped,
                parse_mode: Some("MarkdownV2"),
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

/// Wrap `body` in a triple-backtick block, escaping the only two
/// characters that have meaning inside a MarkdownV2 pre block.
fn wrap_pre(body: &str) -> String {
    let mut escaped = String::with_capacity(body.len() + 8);
    escaped.push_str("```\n");
    for ch in body.chars() {
        if ch == '\\' || ch == '`' {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    if !escaped.ends_with('\n') {
        escaped.push('\n');
    }
    escaped.push_str("```");
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn api(server: &MockServer) -> TelegramApi {
        TelegramApi::new_with_client(Arc::new(reqwest::Client::new()), "12345:test-secret")
            .with_base_url(server.uri())
    }

    #[test]
    fn wrap_pre_escapes_backticks_and_backslashes() {
        let out = wrap_pre("a`b\\c");
        assert!(out.starts_with("```\n"));
        assert!(out.ends_with("```"));
        assert!(out.contains("a\\`b\\\\c"));
    }

    #[test]
    fn wrap_pre_idempotent_trailing_newline() {
        let with = wrap_pre("x\n");
        let without = wrap_pre("x");
        // Both produce the same shape: pre-opener, body, single
        // trailing newline, pre-closer. The wrapper does not double
        // the trailing newline when the caller already supplied one.
        assert_eq!(with.matches('\n').count(), without.matches('\n').count());
    }

    #[tokio::test]
    async fn post_sends_markdown_v2_pre_block_with_chat_and_reply_to() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot12345:test-secret/sendMessage"))
            .respond_with(|req: &Request| {
                let body: serde_json::Value = req.body_json().expect("json");
                assert_eq!(body["chat_id"], "555");
                assert_eq!(body["parse_mode"], "MarkdownV2");
                assert_eq!(body["reply_to_message_id"], 17);
                let text = body["text"].as_str().expect("text");
                assert!(text.starts_with("```\n"));
                assert!(text.ends_with("```"));
                assert!(text.contains("[fail]"));
                ResponseTemplate::new(200).set_body_json(json!({
                    "ok": true,
                    "result": { "message_id": 88i64, "chat": { "id": 555i64 } }
                }))
            })
            .mount(&server)
            .await;
        let transport = MarkdownV2Transport::new(api(&server));
        let posted = transport
            .post("555", "[fail] something exploded", Some("17"))
            .await
            .expect("ok");
        assert_eq!(posted.chat, "555");
        assert_eq!(posted.id, "88");
    }
}
