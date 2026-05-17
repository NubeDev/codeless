//! Thin wrapper around the Telegram Bot API HTTP endpoints used by
//! the operator surface. Counterpart of
//! `codeless_slack::web_api::SlackWebApi`.
//!
//! Base URL: `https://api.telegram.org/bot<token>/<method>`.

use serde::{Deserialize, Serialize};

pub struct TelegramApi {
    _http: reqwest::Client,
    _base_url: String,
}

#[derive(Debug, Serialize)]
pub struct SendMessageArgs<'a> {
    pub chat_id: &'a str,
    pub text: &'a str,
    pub parse_mode: Option<&'a str>,
    pub reply_to_message_id: Option<i64>,
    pub message_thread_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SentMessage {
    pub message_id: i64,
    pub chat: Chat,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct BotUser {
    pub id: i64,
    pub username: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WebApiError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("telegram api: {description}")]
    Api { description: String },
}

impl TelegramApi {
    pub fn new(_bot_token: &str) -> Self {
        todo!("build reqwest client with sensible timeouts; set _base_url")
    }

    /// `getMe` — used at startup to verify the token and learn the
    /// bot's own username so the dispatcher can strip a leading
    /// `@bot_username` mention from inbound text.
    pub async fn get_me(&self) -> Result<BotUser, WebApiError> {
        todo!()
    }

    pub async fn send_message(&self, _args: SendMessageArgs<'_>) -> Result<SentMessage, WebApiError> {
        todo!()
    }

    /// `editMessageText` — used by the outbound publisher when an
    /// already-posted notification needs an updated body (e.g. a
    /// review verdict arriving after the initial failure card).
    pub async fn edit_message_text(
        &self,
        _chat_id: &str,
        _message_id: i64,
        _text: &str,
        _parse_mode: Option<&str>,
    ) -> Result<(), WebApiError> {
        todo!()
    }
}
