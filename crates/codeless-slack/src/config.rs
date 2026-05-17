//! Configuration plucked out of the shared secrets file. Two tokens
//! drive the Socket Mode surface and the Web API surface respectively;
//! Slack distinguishes them on purpose — the app-level token only
//! grants Socket Mode `connections:write` (no posting), the bot token
//! grants `chat:write` against the workspace but cannot open Socket
//! Mode. Keeping them as two keys mirrors that boundary so a leaked
//! bot token cannot also receive inbound commands and vice versa.

use codeless_adapters_host::SecretStore;
use thiserror::Error;

/// App-level token used to open Socket Mode connections. Must start
/// with `xapp-`.
pub const SLACK_APP_TOKEN_KEY: &str = "slack_app_token";

/// Bot user OAuth token used for outbound Web API calls (post message,
/// reactions, etc.). Must start with `xoxb-`. Not consumed until the
/// command-dispatch + outbound notification stages land — kept on the
/// config struct now so the secrets file is the only place tokens live
/// and missing-token errors surface at boot rather than first send.
pub const SLACK_BOT_TOKEN_KEY: &str = "slack_bot_token";

/// Default channel ID for outbound failure notifications. Optional at
/// the secrets layer — the bot still answers commands without it, but
/// the event-bus publisher in a later stage refuses to fire when no
/// channel is configured.
pub const SLACK_CHANNEL_KEY: &str = "slack_channel_id";

/// Parsed Slack configuration. Constructed via [`Self::from_secrets`];
/// individual fields are intentionally `pub` so the later wiring
/// stages can read tokens out without going through accessors.
#[derive(Debug, Clone)]
pub struct SlackConfig {
    /// App-level `xapp-…` token for Socket Mode.
    pub app_token: String,
    /// Bot user `xoxb-…` token for the Web API.
    pub bot_token: String,
    /// Optional channel ID (`C…`) for outbound notifications.
    pub channel_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum SlackConfigError {
    /// `slack_app_token` is absent. The bot cannot open Socket Mode
    /// without it; this is a hard error so the boot path surfaces a
    /// clear message rather than burning a reconnect loop on 401s.
    #[error(
        "missing `slack_app_token` in secrets store; run \
         `codeless secrets set slack_app_token xapp-...` and restart"
    )]
    MissingAppToken,
    /// `slack_bot_token` is absent. Kept as a hard error so a half-
    /// configured bot does not get a Socket Mode connection it cannot
    /// reply over.
    #[error(
        "missing `slack_bot_token` in secrets store; run \
         `codeless secrets set slack_bot_token xoxb-...` and restart"
    )]
    MissingBotToken,
    /// Token prefix sanity check: catches the common `xoxb` <-> `xapp`
    /// swap that produces a 401 with a hard-to-find error message.
    #[error("`{key}` has unexpected prefix; expected `{expected}`")]
    BadTokenPrefix {
        key: &'static str,
        expected: &'static str,
    },
}

impl SlackConfig {
    /// Pull the Slack keys out of `store`. Returns `Err` when either
    /// required key is missing or has an obviously wrong prefix.
    pub fn from_secrets(store: &SecretStore) -> Result<Self, SlackConfigError> {
        let app_token = store
            .get(SLACK_APP_TOKEN_KEY)
            .map(str::to_owned)
            .ok_or(SlackConfigError::MissingAppToken)?;
        if !app_token.starts_with("xapp-") {
            return Err(SlackConfigError::BadTokenPrefix {
                key: SLACK_APP_TOKEN_KEY,
                expected: "xapp-",
            });
        }

        let bot_token = store
            .get(SLACK_BOT_TOKEN_KEY)
            .map(str::to_owned)
            .ok_or(SlackConfigError::MissingBotToken)?;
        if !bot_token.starts_with("xoxb-") {
            return Err(SlackConfigError::BadTokenPrefix {
                key: SLACK_BOT_TOKEN_KEY,
                expected: "xoxb-",
            });
        }

        let channel_id = store.get(SLACK_CHANNEL_KEY).map(str::to_owned);

        Ok(Self {
            app_token,
            bot_token,
            channel_id,
        })
    }
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
        // The store path keeps the tempdir alive through the SecretStore;
        // returning the store transfers ownership of the loaded entries
        // and the dir is dropped at scope exit which is fine for tests.
        store
    }

    #[test]
    fn missing_app_token_reports_specific_error() {
        let store = store_with(&[]);
        let err = SlackConfig::from_secrets(&store).unwrap_err();
        assert!(matches!(err, SlackConfigError::MissingAppToken));
    }

    #[test]
    fn missing_bot_token_reports_specific_error() {
        let store = store_with(&[(SLACK_APP_TOKEN_KEY, "xapp-1-abc")]);
        let err = SlackConfig::from_secrets(&store).unwrap_err();
        assert!(matches!(err, SlackConfigError::MissingBotToken));
    }

    #[test]
    fn swapped_tokens_are_rejected_by_prefix_check() {
        let store = store_with(&[
            (SLACK_APP_TOKEN_KEY, "xoxb-wrong-here"),
            (SLACK_BOT_TOKEN_KEY, "xapp-also-wrong"),
        ]);
        let err = SlackConfig::from_secrets(&store).unwrap_err();
        match err {
            SlackConfigError::BadTokenPrefix { key, expected } => {
                assert_eq!(key, SLACK_APP_TOKEN_KEY);
                assert_eq!(expected, "xapp-");
            }
            other => panic!("expected BadTokenPrefix, got {other:?}"),
        }
    }

    #[test]
    fn happy_path_populates_optional_channel() {
        let store = store_with(&[
            (SLACK_APP_TOKEN_KEY, "xapp-1-abc"),
            (SLACK_BOT_TOKEN_KEY, "xoxb-bot-1-abc"),
            (SLACK_CHANNEL_KEY, "C12345"),
        ]);
        let cfg = SlackConfig::from_secrets(&store).expect("parsed");
        assert_eq!(cfg.app_token, "xapp-1-abc");
        assert_eq!(cfg.bot_token, "xoxb-bot-1-abc");
        assert_eq!(cfg.channel_id.as_deref(), Some("C12345"));
    }

    #[test]
    fn channel_is_optional() {
        let store = store_with(&[
            (SLACK_APP_TOKEN_KEY, "xapp-1-abc"),
            (SLACK_BOT_TOKEN_KEY, "xoxb-bot"),
        ]);
        let cfg = SlackConfig::from_secrets(&store).expect("parsed");
        assert!(cfg.channel_id.is_none());
    }
}
