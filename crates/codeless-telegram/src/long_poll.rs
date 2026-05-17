//! Long-poll inbound loop for the Telegram surface. Counterpart of
//! `codeless_slack::socket_mode::SocketModeSession`.
//!
//! Telegram does not offer a persistent websocket; the official
//! polling transport is `getUpdates` with a long timeout. The loop
//! shape is:
//!
//!   1. Call `getMe` once at startup to learn the bot's own
//!      username (used by the projector to strip the leading
//!      `@username` mention). A failure here surfaces a clear "bad
//!      token" error rather than letting the loop burn on `getUpdates`
//!      401s.
//!   2. Loop: call `getUpdates(offset, timeout=30)`. On success,
//!      project each update onto a `codeless_bot_core::InboundMessage`
//!      and hand it to [`codeless_bot_core::Dispatcher::dispatch_message`],
//!      then bump `offset = max(update_id) + 1`.
//!   3. On network or API failure, sleep with exponential backoff
//!      (1s → 60s cap, doubled per failure, reset on the next
//!      success).
//!   4. Honour the shutdown signal at every await point so a clean
//!      server shutdown drains within one in-flight request.

use std::time::Duration;

use codeless_bot_core::Dispatcher;
use tokio::sync::oneshot;
use tokio::time::sleep;

use crate::dispatcher::project_update;
use crate::web_api::{TelegramApi, WebApiError, LONG_POLL_TIMEOUT_SECS};

/// Initial backoff after a failure. Doubles per consecutive failure
/// up to [`BACKOFF_MAX`]; resets after the next successful poll.
const BACKOFF_INITIAL: Duration = Duration::from_secs(1);

/// Cap on the backoff sleep. 60s matches the slack reconnect cap;
/// long-poll failures are usually transient (DNS, 502 from a
/// platform incident) and an aggressive ceiling here just delays
/// observability of a real outage.
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Run the long-poll loop until the shutdown channel fires or the
/// `getMe` precheck fails. The bot username is resolved once at
/// startup; a missing `username` on the bot user (Telegram allows
/// bots without one, though BotFather requires it on creation) is
/// treated as the empty string — the mention-strip path then just
/// passes every body through.
pub async fn run(api: TelegramApi, dispatcher: Dispatcher, mut shutdown: oneshot::Receiver<()>) {
    let bot_username = match api.get_me().await {
        Ok(user) => user.username.unwrap_or_default(),
        Err(err) => {
            // Without `getMe` we cannot strip mentions and we know
            // the token is broken (only auth failures and transport
            // errors can land here at startup). Surface the failure
            // and exit so the operator sees the boot-time error
            // rather than a tight reconnect loop on `getUpdates`.
            tracing::warn!(
                error = %err,
                "telegram: getMe failed at startup; long-poll loop will not start",
            );
            return;
        }
    };

    let mut offset: Option<i64> = None;
    let mut backoff = BACKOFF_INITIAL;
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                tracing::info!("telegram: long-poll loop received shutdown");
                return;
            }
            result = api.get_updates(offset, LONG_POLL_TIMEOUT_SECS) => {
                match result {
                    Ok(updates) => {
                        backoff = BACKOFF_INITIAL;
                        for update in &updates {
                            // Advance the offset before dispatch so a panic
                            // inside the dispatcher would not re-deliver
                            // the same message after restart; the shared
                            // `RpcServer` calls are idempotent at the
                            // command level but a duplicate `start` would
                            // still post a duplicate reply.
                            offset = Some(offset.map_or(update.update_id + 1, |o| o.max(update.update_id + 1)));
                            let Some(msg) = update.message.as_ref() else { continue };
                            let Some(inbound) = project_update(msg, &bot_username) else { continue };
                            dispatcher.dispatch_message(inbound).await;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            backoff_ms = backoff.as_millis() as u64,
                            "telegram: getUpdates failed; sleeping before retry",
                        );
                        if matches!(err, WebApiError::HttpStatus { status: 401 } | WebApiError::HttpStatus { status: 404 }) {
                            // 401 = bad token, 404 = wrong base URL.
                            // Both are non-transient; backing off
                            // forever would just bury the cause.
                            tracing::error!("telegram: token rejected; stopping long-poll loop");
                            return;
                        }
                        tokio::select! {
                            biased;
                            _ = &mut shutdown => return,
                            _ = sleep(backoff) => {
                                backoff = (backoff * 2).min(BACKOFF_MAX);
                            }
                        }
                    }
                }
            }
        }
    }
}
