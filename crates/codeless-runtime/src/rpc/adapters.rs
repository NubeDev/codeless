//! `list_chat_adapters` / `set_chat_adapter_enabled` /
//! `validate_chat_adapter_secrets` / `list_runners` /
//! `set_runner_enabled` RPCs. The `chat_adapters` and `runner_config`
//! SQLite tables are the source of truth (R4); methods here read /
//! write rows and gate the enable bit on a process-lifetime in-memory
//! "validated" cache so the UI cannot arm an adapter that would crash
//! on the next boot.
//!
//! Validate-before-enable is the load-bearing contract: the validate
//! cache is in-memory and clears on restart, which means after every
//! restart the operator must re-validate before re-enabling. That is
//! the deliberate UX consequence of "the cache lives for the lifetime
//! of the process" — recorded as the stage-1 decision for OQ#3 in
//! `.codeless/jobs/adapter-registry/SCOPE.md`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use codeless_rpc::{
    AdapterError, ChatAdapterKind, ChatAdapterRow, ChatAdapterSecretProblem,
    ListChatAdaptersResult, ListRunnersResult, RpcError, RpcResult, RunnerRow,
    SetChatAdapterEnabledArgs, SetRunnerEnabledArgs, ValidateChatAdapterSecretsArgs,
    ValidateChatAdapterSecretsResult,
};
use codeless_types::UnixMillis;
use parking_lot::Mutex;

use super::InProcessRpc;
use crate::adapter_registry::{self, DEFAULT_INSTANCE_ID};

/// 5/s per-`(kind, instance_id)` rate-limit window. The doc calls for
/// 5 calls per second per bucket; we keep a sliding-window deque of
/// the last 5 timestamps and refuse when the oldest is younger than a
/// second. Per-`(kind, instance_id)` because a slow Slack probe must
/// not block a concurrent Telegram one (deliberate divergence from
/// `validate_workspace_path`'s per-connection limit).
const RATE_WINDOW: Duration = Duration::from_secs(1);
const RATE_LIMIT: usize = 5;

/// Hard timeout on the upstream identity call. Telegram `getMe` over
/// a bad network has been observed to hang ~30s; capping at 5s keeps
/// the picker responsive and matches the doc.
pub const VALIDATE_TIMEOUT: Duration = Duration::from_secs(5);

/// Snapshot key used for the in-memory validate cache and the rate-
/// limit map. Kept private — the wire-level args carry the same pair.
type AdapterKey = (ChatAdapterKind, String);

/// Process-lifetime "this `(kind, instance_id)` passed validation"
/// set. Cleared on restart. `set_chat_adapter_enabled(true)` refuses
/// unless the key is in here.
#[derive(Default)]
pub(crate) struct ValidationState {
    validated: Mutex<HashSet<AdapterKey>>,
    rate: Mutex<HashMap<AdapterKey, VecDeque<Instant>>>,
}

impl ValidationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn is_validated(&self, key: &AdapterKey) -> bool {
        self.validated.lock().contains(key)
    }

    fn mark_validated(&self, key: AdapterKey) {
        self.validated.lock().insert(key);
    }

    fn forget(&self, key: &AdapterKey) {
        self.validated.lock().remove(key);
    }

    /// Returns `Ok(())` when the bucket has capacity, `Err(())` when
    /// the most recent five attempts all fall inside the 1s window.
    /// The check appends *before* the upstream call, so a refused
    /// call still costs a slot — that matches token-bucket semantics
    /// and stops a tight loop from probing past the limit by racing
    /// the cleanup.
    fn try_acquire(&self, key: &AdapterKey, now: Instant) -> Result<(), Duration> {
        let mut map = self.rate.lock();
        let entry = map.entry(key.clone()).or_default();
        while let Some(front) = entry.front() {
            if now.duration_since(*front) >= RATE_WINDOW {
                entry.pop_front();
            } else {
                break;
            }
        }
        if entry.len() >= RATE_LIMIT {
            let retry_in = RATE_WINDOW - now.duration_since(*entry.front().expect("non-empty"));
            return Err(retry_in);
        }
        entry.push_back(now);
        Ok(())
    }
}

/// Pluggable upstream validator. The default in-tree impl
/// (`HttpValidationProbe`) is the real thing — Slack `auth.test`,
/// Telegram `getMe`. Tests inject a `MockValidationProbe` so the
/// round-trip cargo test does not touch the network. Kept as a trait
/// object on `InProcessRpc` so the runtime stays unaware of which
/// transport is talking to the upstream.
#[async_trait]
pub trait ValidationProbe: Send + Sync {
    /// Validate one `(kind, instance_id)`. Returns the structured
    /// `ValidateChatAdapterSecretsResult` the RPC surfaces verbatim;
    /// the caller wraps the call in `tokio::time::timeout` and the
    /// rate-limit bucket so probe impls do not have to.
    async fn validate(
        &self,
        kind: ChatAdapterKind,
        instance_id: &str,
    ) -> ValidateChatAdapterSecretsResult;
}

/// Canonical secret-key names per chat-adapter kind. Sourced from
/// `codeless-slack/src/config.rs` (`SLACK_APP_TOKEN_KEY`,
/// `SLACK_BOT_TOKEN_KEY`) and `codeless-telegram/src/config.rs`
/// (`TELEGRAM_BOT_TOKEN_KEY`); inlined here so this crate does not
/// need to depend on either adapter crate just to compute a hint.
/// When a new variant lands (Gmail), add its keys alongside the
/// variant — the match is `match`-exhaustive so a missed entry
/// breaks the build, not a runtime branch.
pub fn required_secret_keys(kind: ChatAdapterKind) -> &'static [&'static str] {
    match kind {
        ChatAdapterKind::Slack => &["slack_app_token", "slack_bot_token"],
        ChatAdapterKind::Telegram => &["telegram_bot_token"],
    }
}

pub(super) async fn list_chat_adapters(rpc: &InProcessRpc) -> RpcResult<ListChatAdaptersResult> {
    let rows = adapter_registry::list_chat_adapters(rpc.pool())
        .await
        .map_err(super::db_err)?;
    let mut adapters = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = parse_kind(&row.kind)?;
        adapters.push(ChatAdapterRow {
            kind,
            instance_id: row.instance_id,
            enabled: row.enabled,
            configured_at: UnixMillis(row.configured_at_ms),
        });
    }
    Ok(ListChatAdaptersResult { adapters })
}

pub(super) async fn set_chat_adapter_enabled(
    rpc: &InProcessRpc,
    args: SetChatAdapterEnabledArgs,
) -> RpcResult<()> {
    let key: AdapterKey = (args.kind, args.instance_id.clone());

    // Enabling is the gated path. Disabling is always allowed — the
    // user must be able to switch off a broken adapter regardless of
    // validate state, and `enabled = false` does not arm anything
    // dangerous on the next boot.
    if args.enabled && !rpc.validation.is_validated(&key) {
        return Err(RpcError::Adapter(AdapterError::MissingSecrets {
            keys: required_secret_keys(args.kind)
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }));
    }

    let kind_str = kind_wire(args.kind);
    adapter_registry::upsert_chat_adapter(rpc.pool(), kind_str, &args.instance_id, args.enabled)
        .await
        .map_err(super::db_err)?;

    // Flipping a row off invalidates the cached validate for it —
    // re-enabling later must re-prove the credentials. Keeps the
    // refusal logic above honest after a toggle.
    if !args.enabled {
        rpc.validation.forget(&key);
    }
    Ok(())
}

pub(super) async fn validate_chat_adapter_secrets(
    rpc: &InProcessRpc,
    args: ValidateChatAdapterSecretsArgs,
) -> RpcResult<ValidateChatAdapterSecretsResult> {
    let key: AdapterKey = (args.kind, args.instance_id.clone());

    if let Err(retry_in) = rpc.validation.try_acquire(&key, Instant::now()) {
        return Err(RpcError::Conflict(format!(
            "validate rate limit: retry in {}ms",
            retry_in.as_millis()
        )));
    }

    let probe = match rpc.validation_probe.as_ref() {
        Some(p) => Arc::clone(p),
        None => {
            return Err(RpcError::Internal(
                "no validation probe configured on this runtime".into(),
            ))
        }
    };

    let result = match tokio::time::timeout(
        VALIDATE_TIMEOUT,
        probe.validate(args.kind, &args.instance_id),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => ValidateChatAdapterSecretsResult {
            ok: false,
            problems: vec![ChatAdapterSecretProblem::Timeout],
        },
    };

    if result.ok {
        rpc.validation.mark_validated(key);
    } else {
        rpc.validation.forget(&key);
    }
    Ok(result)
}

pub(super) async fn list_runners(rpc: &InProcessRpc) -> RpcResult<ListRunnersResult> {
    let rows = adapter_registry::list_runners(rpc.pool())
        .await
        .map_err(super::db_err)?;
    let runners = rows
        .into_iter()
        .map(|r| RunnerRow {
            runner_id: r.runner_id,
            enabled: r.enabled,
        })
        .collect();
    Ok(ListRunnersResult { runners })
}

pub(super) async fn set_runner_enabled(
    rpc: &InProcessRpc,
    args: SetRunnerEnabledArgs,
) -> RpcResult<()> {
    if args.runner_id.trim().is_empty() {
        return Err(RpcError::InvalidArgument("runner_id is empty".into()));
    }
    adapter_registry::upsert_runner(rpc.pool(), &args.runner_id, args.enabled)
        .await
        .map_err(super::db_err)?;
    Ok(())
}

fn kind_wire(kind: ChatAdapterKind) -> &'static str {
    match kind {
        ChatAdapterKind::Slack => "slack",
        ChatAdapterKind::Telegram => "telegram",
    }
}

fn parse_kind(raw: &str) -> RpcResult<ChatAdapterKind> {
    match raw {
        "slack" => Ok(ChatAdapterKind::Slack),
        "telegram" => Ok(ChatAdapterKind::Telegram),
        other => Err(RpcError::Internal(format!(
            "unknown chat-adapter kind in db: {other}"
        ))),
    }
}

/// Test-only probe that returns a caller-supplied result. Used by
/// `tests/adapter_registry_rpc.rs` to drive the validate path without
/// touching the network. Kept public so integration tests can build it.
#[doc(hidden)]
pub struct StaticValidationProbe {
    pub result: ValidateChatAdapterSecretsResult,
}

#[async_trait]
impl ValidationProbe for StaticValidationProbe {
    async fn validate(
        &self,
        _kind: ChatAdapterKind,
        _instance_id: &str,
    ) -> ValidateChatAdapterSecretsResult {
        self.result.clone()
    }
}

// `DEFAULT_INSTANCE_ID` is re-exported so call sites that need to
// match the boot upsert's default string without depending on the
// underlying constant location pick it up here.
#[allow(dead_code)]
pub(crate) const fn default_instance_id() -> &'static str {
    DEFAULT_INSTANCE_ID
}
