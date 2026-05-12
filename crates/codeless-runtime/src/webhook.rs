//! Generic outbound webhook backend for the `Notifier` trait. POSTs
//! the `NotificationPayload` as JSON, signs the request body with
//! HMAC-SHA256, and surfaces the signature on the
//! `X-Codeless-Signature` header as lowercase hex.
//!
//! HMAC is over the raw body bytes only — the receiving side then
//! only needs the shared secret and the body to verify, with no
//! header-order or canonicalisation surprises.
//!
//! The config struct is shape-compatible with a TOML section in the
//! secrets file (SCOPE.md "single-tenant, secrets-file backend"):
//!
//! ```toml
//! [notifier.webhook]
//! url = "https://hooks.example.com/codeless"
//! hmac_key_hex = "deadbeef..."
//! ```

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::notifier::{NotificationPayload, Notifier, NotifierError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    /// Hex-encoded HMAC-SHA256 key. Hex (not base64) so the secret
    /// survives TOML round-trips without escape headaches. Decoded
    /// once at `WebhookNotifier::from_config`.
    pub hmac_key_hex: String,
}

pub struct WebhookNotifier {
    url: String,
    hmac_key: Vec<u8>,
    client: reqwest::Client,
}

impl WebhookNotifier {
    pub fn from_config(cfg: WebhookConfig) -> Result<Self, WebhookSetupError> {
        let key = hex::decode(cfg.hmac_key_hex.trim()).map_err(WebhookSetupError::Hex)?;
        if key.is_empty() {
            return Err(WebhookSetupError::EmptyKey);
        }
        Ok(Self {
            url: cfg.url,
            hmac_key: key,
            client: reqwest::Client::new(),
        })
    }

    /// Constructor for tests and embedders that already hold the
    /// decoded key bytes.
    pub fn new(url: impl Into<String>, hmac_key: Vec<u8>) -> Self {
        Self {
            url: url.into(),
            hmac_key,
            client: reqwest::Client::new(),
        }
    }

    /// Header the signature lands on. Exposed so receivers can read
    /// the canonical name from the source.
    pub const SIGNATURE_HEADER: &'static str = "x-codeless-signature";
}

#[derive(Debug, thiserror::Error)]
pub enum WebhookSetupError {
    #[error("hmac_key_hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("hmac_key_hex decodes to an empty key; refusing to send unauthenticated requests")]
    EmptyKey,
}

#[async_trait]
impl Notifier for WebhookNotifier {
    async fn notify(&self, payload: NotificationPayload) -> Result<(), NotifierError> {
        let body =
            serde_json::to_vec(&payload).map_err(|e| NotifierError::Transport(format!("{e}")))?;
        let signature = hex::encode(sign(&self.hmac_key, &body));
        let response = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            .header(Self::SIGNATURE_HEADER, &signature)
            .body(body)
            .send()
            .await
            .map_err(|e| NotifierError::Transport(format!("{e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(NotifierError::Status {
                status: status.as_u16(),
            });
        }
        Ok(())
    }
}

fn sign(key: &[u8], body: &[u8]) -> Vec<u8> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length, including 0");
    mac.update(body);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_known_hmac_vector() {
        let sig = sign(b"key", b"The quick brown fox jumps over the lazy dog");
        assert_eq!(
            hex::encode(sig),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }
}
