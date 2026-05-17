//! Resolved Telegram configuration loaded from the shared
//! [`SecretStore`]. Mirrors `codeless_slack::config::SlackConfig`,
//! with the Slack-specific app/bot token split collapsed into one
//! Bot API token (BotFather issues a single bearer credential per
//! bot).
//!
//! Required key:
//!
//! - `telegram_bot_token` — token from `@BotFather`, format
//!   `\d+:[A-Za-z0-9_-]+` (a numeric bot id, a colon, and a
//!   URL-safe secret). The prefix check is a sanity guard against
//!   a swapped Slack token landing in the wrong slot, not a full
//!   format validator — Telegram itself rejects malformed tokens
//!   with a 401 from `getMe` on first call.
//!
//! Optional key:
//!
//! - `telegram_chat_id` — destination chat the outbound publisher
//!   posts failure cards into. Stored as a string so a group chat's
//!   negative id (`-100…`) and a 1:1 dm's positive id both round-trip
//!   without a numeric-overflow surprise. Absent → the outbound
//!   publisher is not spawned and the bot operates inbound-only.

use codeless_adapters_host::SecretStore;
use thiserror::Error;

/// Bot API bearer token. Required.
pub const TELEGRAM_BOT_TOKEN_KEY: &str = "telegram_bot_token";

/// Destination chat for outbound failure notifications. Optional.
/// When absent the outbound publisher is not spawned; the inbound
/// command surface still works.
pub const TELEGRAM_CHAT_ID_KEY: &str = "telegram_chat_id";

/// Parsed Telegram configuration. Constructed via
/// [`Self::from_secrets`]; fields are intentionally `pub` so the
/// wiring at [`crate::TelegramBot::spawn`] can read them directly
/// without going through accessors.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot API bearer credential.
    pub bot_token: String,
    /// Destination chat for outbound notifications. `None` → no
    /// outbound publisher.
    pub chat_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum TelegramConfigError {
    /// `telegram_bot_token` is absent. The bot cannot call any Bot
    /// API method without it; a hard error here surfaces the missing
    /// secret at boot rather than burning a long-poll loop on 401s.
    #[error(
        "missing `telegram_bot_token` in secrets store; run \
         `codeless secrets set telegram_bot_token <token>` and restart"
    )]
    MissingBotToken,
    /// Token does not look like a Telegram Bot API token. Telegram
    /// tokens take the shape `<bot_id>:<secret>` — a digit-run, a
    /// colon, and a URL-safe secret. Catching the obvious swap
    /// (Slack token in the telegram slot) at boot is worth the
    /// trivial check.
    #[error(
        "`telegram_bot_token` does not match the `<id>:<secret>` shape \
         (a numeric id followed by `:` and a URL-safe secret)"
    )]
    BadTokenShape,
}

impl TelegramConfig {
    /// Pull the Telegram keys out of `store`. Returns `Err` when the
    /// required token is missing or has an obviously wrong shape.
    pub fn from_secrets(store: &SecretStore) -> Result<Self, TelegramConfigError> {
        let bot_token = store
            .get(TELEGRAM_BOT_TOKEN_KEY)
            .map(str::to_owned)
            .ok_or(TelegramConfigError::MissingBotToken)?;
        if !looks_like_bot_token(&bot_token) {
            return Err(TelegramConfigError::BadTokenShape);
        }
        let chat_id = store.get(TELEGRAM_CHAT_ID_KEY).map(str::to_owned);
        Ok(Self { bot_token, chat_id })
    }
}

/// Cheap structural check: at least one digit, a colon, at least one
/// URL-safe secret character. Tighter validation is left to `getMe`
/// at first call — Bot API tokens can carry test/production suffix
/// variations that a regex here would have to keep up with.
fn looks_like_bot_token(token: &str) -> bool {
    let mut parts = token.splitn(2, ':');
    let Some(id) = parts.next() else {
        return false;
    };
    let Some(secret) = parts.next() else {
        return false;
    };
    !id.is_empty()
        && id.chars().all(|c| c.is_ascii_digit())
        && !secret.is_empty()
        && secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store_with(entries: &[(&str, &str)]) -> SecretStore {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("secrets.toml");
        let mut store = SecretStore::open(&path).expect("open empty store");
        for (k, v) in entries {
            store.set(*k, *v).expect("set");
        }
        store
    }

    #[test]
    fn missing_bot_token_reports_specific_error() {
        let store = store_with(&[]);
        let err = TelegramConfig::from_secrets(&store).unwrap_err();
        assert!(matches!(err, TelegramConfigError::MissingBotToken));
    }

    #[test]
    fn slack_token_in_telegram_slot_is_rejected_by_shape_check() {
        let store = store_with(&[(TELEGRAM_BOT_TOKEN_KEY, "xoxb-bot-1-abc")]);
        let err = TelegramConfig::from_secrets(&store).unwrap_err();
        assert!(matches!(err, TelegramConfigError::BadTokenShape));
    }

    #[test]
    fn empty_secret_is_rejected_by_shape_check() {
        let store = store_with(&[(TELEGRAM_BOT_TOKEN_KEY, "123:")]);
        let err = TelegramConfig::from_secrets(&store).unwrap_err();
        assert!(matches!(err, TelegramConfigError::BadTokenShape));
    }

    #[test]
    fn happy_path_with_chat_id() {
        let store = store_with(&[
            (
                TELEGRAM_BOT_TOKEN_KEY,
                "8882032367:ABCdefGHIjklMNOpqrSTUvwxYZ-1234567",
            ),
            (TELEGRAM_CHAT_ID_KEY, "987654321"),
        ]);
        let cfg = TelegramConfig::from_secrets(&store).expect("parsed");
        assert!(cfg.bot_token.starts_with("8882032367:"));
        assert_eq!(cfg.chat_id.as_deref(), Some("987654321"));
    }

    #[test]
    fn happy_path_with_negative_group_chat_id() {
        let store = store_with(&[
            (TELEGRAM_BOT_TOKEN_KEY, "12345:secret_token-1"),
            (TELEGRAM_CHAT_ID_KEY, "-1001234567890"),
        ]);
        let cfg = TelegramConfig::from_secrets(&store).expect("parsed");
        assert_eq!(cfg.chat_id.as_deref(), Some("-1001234567890"));
    }

    #[test]
    fn happy_path_without_chat_id_leaves_outbound_disabled() {
        let store = store_with(&[(TELEGRAM_BOT_TOKEN_KEY, "12345:secret_token-1")]);
        let cfg = TelegramConfig::from_secrets(&store).expect("parsed");
        assert!(cfg.chat_id.is_none());
    }
}
