//! Transport-agnostic send trait.

use async_trait::async_trait;

use super::message::Message;

#[derive(Debug, Clone)]
pub struct SendOutcome {
    /// Backend-specific message identifier (Gmail returns its id;
    /// SMTP returns the queued message-id we generated).
    pub message_id: String,
    /// Backend tag for audit logs.
    pub backend: &'static str,
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, message: &Message) -> Result<SendOutcome, MailerError>;
}

#[derive(Debug, thiserror::Error)]
pub enum MailerError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("invalid message: {0}")]
    Message(#[from] super::message::MessageError),
    #[error("transport failed: {0}")]
    Transport(String),
    #[error("backend rejected message: {status}: {body}")]
    Rejected { status: u16, body: String },
    #[error("cancelled")]
    Cancelled,
}
