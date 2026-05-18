//! Gmail REST `users.messages.send` backend.
//!
//! Auth is a caller-supplied OAuth2 access token. Token acquisition
//! (interactive consent, refresh) sits outside this crate — host
//! wiring picks the strategy (Tauri keychain, env var, GSA) and
//! hands a bearer token to `GmailMailer::new`.

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::json;

use super::mailer::{Mailer, MailerError, SendOutcome};
use super::message::Message;

const ENDPOINT_TEMPLATE: &str =
    "https://gmail.googleapis.com/gmail/v1/users/{user}/messages/send";

pub struct GmailMailer {
    access_token: String,
    user_id: String,
    client: reqwest::Client,
}

impl GmailMailer {
    /// `user_id` is the Gmail account whose mailbox is being sent
    /// from. The literal string `"me"` resolves to the authenticated
    /// user and is the right default for personal tokens.
    pub fn new(access_token: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            user_id: user_id.into(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_client(
        access_token: impl Into<String>,
        user_id: impl Into<String>,
        client: reqwest::Client,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            user_id: user_id.into(),
            client,
        }
    }
}

#[async_trait]
impl Mailer for GmailMailer {
    async fn send(&self, message: &Message) -> Result<SendOutcome, MailerError> {
        let raw = message.to_rfc5322()?;
        let raw_b64 = URL_SAFE_NO_PAD.encode(raw);

        let url = ENDPOINT_TEMPLATE.replace("{user}", &self.user_id);
        let body = json!({ "raw": raw_b64 }).to_string();

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| MailerError::Transport(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            let body = resp.text().await.unwrap_or_default();
            return Err(MailerError::Auth(body));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(MailerError::Rejected {
                status: status.as_u16(),
                body,
            });
        }

        let text = resp
            .text()
            .await
            .map_err(|e| MailerError::Transport(format!("decode response: {e}")))?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| MailerError::Transport(format!("decode response: {e}")))?;
        let id = json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        Ok(SendOutcome {
            message_id: id,
            backend: "gmail",
        })
    }
}
