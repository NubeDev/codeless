//! Resolved Telegram configuration loaded from the shared
//! `SecretStore`. Mirrors `codeless_slack::config::SlackConfig`.

use codeless_adapters_host::secrets::SecretStore;

/// Required secret keys:
///   - `telegram_bot_token` — token from @BotFather (required).
///   - `telegram_chat_id`   — destination chat id for outbound
///                            notifications. When absent, the
///                            outbound publisher is not spawned and
///                            the bot operates inbound-only.
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("telegram_bot_token missing from secret store")]
    MissingBotToken,
    #[error("secret store error: {0}")]
    Store(String),
}

impl TelegramConfig {
    /// Read the bot token (required) and chat id (optional) out of
    /// the secret store. Returns `Ok(None)` from the caller's
    /// perspective if the bot is intentionally disabled — but that
    /// gating happens at the CLI flag, not here.
    pub fn from_secrets(_store: &SecretStore) -> Result<Self, ConfigError> {
        todo!("read telegram_bot_token (required) + telegram_chat_id (optional)")
    }
}
