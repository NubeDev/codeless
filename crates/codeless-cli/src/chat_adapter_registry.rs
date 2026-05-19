//! Boot-time registry that spawns the chat-adapter background tasks
//! (Slack, Telegram) the `codeless serve` process holds open for its
//! lifetime.
//!
//! Before stage 8 the spawn lived inline in [`crate::serve::run_server`]
//! behind two `if effective.<kind>_enabled` blocks; the registry
//! collapses both into a single iteration over `chat_adapters` rows so
//! adding a third adapter is a `match` arm here rather than a new
//! conditional in the serve flow. The current `EffectiveAdapterRegistry`
//! projection still hands us the closed set of bits the loaded rows
//! produce — the `kind` strings are matched against the same closed
//! set, so an unknown row in the table never silently activates an
//! adapter the operator has not opted into.
//!
//! Lives in `codeless-cli` rather than `codeless-runtime` because the
//! Slack and Telegram crates are CLI-side dependencies; the runtime is
//! deliberately ignorant of the bot transports (R1 of CLAUDE.md keeps
//! the host-only stack out of the mobile-safe crates, and the runtime
//! has no need of `codeless-slack`/`codeless-telegram` either way).

use std::sync::Arc;

use codeless_adapters_host::SecretStore;
use codeless_rpc::RpcServer;
use codeless_runtime::adapter_registry::EffectiveAdapterRegistry;

/// Handles to the chat-adapter background tasks. Dropped at process
/// exit; the spawned long-poll / Socket Mode loops run until then.
/// Holding the struct in scope (rather than `_ = ...`-binding the
/// individual handles) gives a single place for future shutdown wiring
/// without re-touching the serve flow. Today the fields are never read
/// — the only contract is "stay alive for the process lifetime" — so
/// the `dead_code` allow stays on each field rather than the whole
/// struct (a future graceful shutdown will read them, and we want any
/// new unused field to still warn).
pub struct ChatAdapterRegistry {
    #[allow(dead_code)]
    pub slack: Option<codeless_slack::SlackBot>,
    #[allow(dead_code)]
    pub telegram: Option<codeless_telegram::TelegramBot>,
}

impl ChatAdapterRegistry {
    /// Walk the effective registry and spawn one background task per
    /// enabled chat adapter. Missing secrets and adapter-side init
    /// failures are logged with the same `--enable-<kind> ignored: …`
    /// shape the inline code used, so existing operators see the same
    /// diagnostics on the same lines they always have. The rest of the
    /// server still boots — chat adapters are additive, not
    /// load-bearing for the runtime.
    pub fn spawn(
        effective: &EffectiveAdapterRegistry,
        store: &SecretStore,
        rpc: Arc<dyn RpcServer>,
    ) -> Self {
        let slack = if effective.slack_enabled {
            spawn_slack(store, rpc.clone())
        } else {
            None
        };
        let telegram = if effective.telegram_enabled {
            spawn_telegram(store, rpc.clone())
        } else {
            None
        };
        Self { slack, telegram }
    }
}

fn spawn_slack(store: &SecretStore, rpc: Arc<dyn RpcServer>) -> Option<codeless_slack::SlackBot> {
    match codeless_slack::SlackConfig::from_secrets(store) {
        Ok(cfg) => {
            eprintln!(
                "codeless-server: slack adapter enabled (channel={})",
                cfg.channel_id.as_deref().unwrap_or("unset"),
            );
            Some(codeless_slack::SlackBot::spawn(cfg, rpc))
        }
        Err(err) => {
            eprintln!("codeless-server: --enable-slack ignored: {err}");
            None
        }
    }
}

fn spawn_telegram(
    store: &SecretStore,
    rpc: Arc<dyn RpcServer>,
) -> Option<codeless_telegram::TelegramBot> {
    match codeless_telegram::TelegramConfig::from_secrets(store) {
        Ok(cfg) => {
            eprintln!(
                "codeless-server: telegram adapter enabled (chat={})",
                cfg.chat_id.as_deref().unwrap_or("unset"),
            );
            match codeless_telegram::TelegramBot::spawn(cfg, rpc) {
                Ok(bot) => Some(bot),
                Err(err) => {
                    eprintln!("codeless-server: --enable-telegram ignored: api init failed: {err}");
                    None
                }
            }
        }
        Err(err) => {
            eprintln!("codeless-server: --enable-telegram ignored: {err}");
            None
        }
    }
}
