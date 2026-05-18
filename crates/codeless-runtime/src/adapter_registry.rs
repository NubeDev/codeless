//! Boot-time upsert + read for the adapter registry tables
//! (`chat_adapters`, `runner_config`).
//!
//! The shape mirrors `attached_workspaces`: the legacy `--enable-*`
//! CLI flags are bootstrap conveniences that upsert one row each, and
//! every boot reads the table back to compute the effective enabled
//! set. The table is the source of truth — once a row is written, the
//! flag is no longer required on subsequent boots, and the future UI's
//! `set_chat_adapter_enabled` / `set_runner_enabled` RPCs write the
//! same columns.
//!
//! `instance_id` defaults to `"default"` because the today-case is
//! single-instance per kind. The composite PK leaves room for the
//! multi-account flows (Slack-personal + Slack-work, two Gmail
//! mailboxes) the registry will grow into without a schema change.

use sqlx::SqlitePool;

use crate::time::now_ms;

/// The `instance_id` boot upserts use when the CLI flag is single-
/// instance (today: every flag). UI-driven multi-instance flows pass
/// their own value; this constant exists so the boot path and the
/// migration test reference the same string.
pub const DEFAULT_INSTANCE_ID: &str = "default";

/// Upsert a single chat-adapter row. Idempotent: re-running with the
/// same `(kind, instance_id, enabled)` triple touches `configured_at`
/// but otherwise leaves the row alone, so a CLI flag re-passed on a
/// subsequent boot does not lose state the UI has since changed.
///
/// The `enabled` value the caller passes wins over the existing row's
/// — re-passing `--enable-slack` after the UI disabled the row re-
/// enables it, which matches operator intent: a flag on the command
/// line is an explicit ask.
pub async fn upsert_chat_adapter(
    pool: &SqlitePool,
    kind: &str,
    instance_id: &str,
    enabled: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO chat_adapters (kind, instance_id, enabled, configured_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(kind, instance_id) DO UPDATE SET \
             enabled = excluded.enabled, \
             configured_at = excluded.configured_at",
    )
    .bind(kind)
    .bind(instance_id)
    .bind(i64::from(enabled))
    .bind(now_ms().0)
    .execute(pool)
    .await?;
    Ok(())
}

/// Upsert a single runner row. Same idempotency contract as
/// `upsert_chat_adapter`: the caller's `enabled` value wins.
pub async fn upsert_runner(
    pool: &SqlitePool,
    runner_id: &str,
    enabled: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO runner_config (runner_id, enabled) VALUES (?, ?) \
         ON CONFLICT(runner_id) DO UPDATE SET enabled = excluded.enabled",
    )
    .bind(runner_id)
    .bind(i64::from(enabled))
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ChatAdapterRow {
    pub kind: String,
    pub instance_id: String,
    pub enabled: bool,
    pub configured_at_ms: i64,
}

/// Snapshot the chat-adapter table. Returns rows in deterministic
/// `(kind, instance_id)` order so the boot log and the Settings page
/// render the same sequence regardless of insertion order.
pub async fn list_chat_adapters(pool: &SqlitePool) -> sqlx::Result<Vec<ChatAdapterRow>> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT kind, instance_id, enabled, configured_at FROM chat_adapters \
         ORDER BY kind, instance_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(kind, instance_id, enabled, configured_at_ms)| ChatAdapterRow {
            kind,
            instance_id,
            enabled: enabled != 0,
            configured_at_ms,
        })
        .collect())
}

#[derive(Debug, Clone)]
pub struct RunnerConfigRow {
    pub runner_id: String,
    pub enabled: bool,
}

/// Snapshot the runner-config table. Same ordering contract as
/// `list_chat_adapters`.
pub async fn list_runners(pool: &SqlitePool) -> sqlx::Result<Vec<RunnerConfigRow>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT runner_id, enabled FROM runner_config ORDER BY runner_id")
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(runner_id, enabled)| RunnerConfigRow {
            runner_id,
            enabled: enabled != 0,
        })
        .collect())
}

/// Aggregated boot-time snapshot used by `codeless-cli serve` to pick
/// the effective enabled set. The CLI's `--enable-*` flags upsert rows
/// before this read runs, so a boot with `--enable-claude` produces
/// `claude_enabled = true` here on the same tick.
#[derive(Debug, Clone, Default)]
pub struct EffectiveAdapterRegistry {
    pub slack_enabled: bool,
    pub telegram_enabled: bool,
    pub claude_enabled: bool,
    pub anthropic_enabled: bool,
    pub codex_enabled: bool,
    pub copilot_enabled: bool,
}

/// Read both tables and project them into the closed set of flags the
/// `serve` binary still consumes today. New kinds (Gmail, Discord, …)
/// land as additional fields on this struct alongside the migration
/// that introduces their first row; until then the projection stays
/// closed so an unknown `kind` row in `chat_adapters` does not
/// silently change boot behaviour.
pub async fn load_effective(pool: &SqlitePool) -> sqlx::Result<EffectiveAdapterRegistry> {
    let mut out = EffectiveAdapterRegistry::default();
    for row in list_chat_adapters(pool).await? {
        // Multi-instance flows aren't wired up yet — Stage 1 only
        // reads the `default` instance per kind, which matches what
        // `--enable-*` upserts. Non-default rows are ignored here and
        // will start contributing once the registry grows the
        // per-instance dispatch the TODO in WORKSPACE-ATTACH calls
        // out.
        if row.instance_id != DEFAULT_INSTANCE_ID {
            continue;
        }
        match row.kind.as_str() {
            "slack" => out.slack_enabled = row.enabled,
            "telegram" => out.telegram_enabled = row.enabled,
            _ => {}
        }
    }
    for row in list_runners(pool).await? {
        match row.runner_id.as_str() {
            "claude" => out.claude_enabled = row.enabled,
            "anthropic" => out.anthropic_enabled = row.enabled,
            "codex" => out.codex_enabled = row.enabled,
            "copilot" => out.copilot_enabled = row.enabled,
            _ => {}
        }
    }
    Ok(out)
}
