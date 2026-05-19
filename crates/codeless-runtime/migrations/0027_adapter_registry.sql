-- Adapter registry: the persisted "what is enabled" state for chat
-- adapters (Slack, Telegram, Gmail, ...) and AI runners (claude,
-- anthropic, codex, copilot). Replaces the `--enable-*` CLI flags as
-- the source of truth — the flags become bootstrap conveniences that
-- upsert rows, exactly the way `--fs-root` upserts `attached_workspaces`
-- (migration 0007). Every subsequent boot reads from these tables.
--
-- Two tables, not one, because the long-term config shape diverges:
-- runners are essentially `enabled` + an optional binary path the host
-- already discovers via `which`; chat adapters carry per-instance state
-- the registry needs (workspace IDs, channel filters, mailbox). If the
-- chat-adapter config stays small enough to collapse into a single
-- `enabled_components` table when the registry graduates to its own
-- scope doc, the migration that does that will fold both tables in;
-- splitting up front keeps the schema honest about the shape we
-- actually need today.

-- Composite PK on `(kind, instance_id)` so the same chat-adapter
-- backend can run multiple instances side by side (Slack-personal +
-- Slack-work, two Gmail accounts) without a future schema change. The
-- today-case uses `instance_id = 'default'` for every kind; multi-
-- instance UX lands in a later RPC, not here.
--
-- `enabled` is the only mutable bit this table carries. Secrets stay
-- in the `SecretStore` (XDG TOML at `~/.config/codeless/secrets.toml`),
-- not here, so the same key-rotation flow Slack and Telegram already
-- use keeps working without a schema migration each time a backend
-- adds a new credential.
--
-- `configured_at` is the wall-clock millis of the most recent upsert,
-- not the row's creation time. Adapters added by `--enable-*` at boot
-- carry the boot time; rows the future UI flips carry the click time.
-- A single column is enough because the registry only needs to answer
-- "when did the user last touch this" for the Settings page; full
-- change history is the event bus's job.
CREATE TABLE chat_adapters (
    kind          TEXT NOT NULL,
    instance_id   TEXT NOT NULL,
    enabled       INTEGER NOT NULL,
    configured_at INTEGER NOT NULL,
    PRIMARY KEY (kind, instance_id)
);

-- Runner enablement. One row per runner_id (`claude`, `anthropic`,
-- `codex`, `copilot`, future entries). No instance dimension because a
-- runner is a single global piece of host capability — there is one
-- `claude` binary on `PATH`, not two — and the binary path itself is
-- discovered at boot rather than persisted (so a moved install is
-- picked up without a DB edit).
--
-- `mock` deliberately has no row: the in-process mock runner is gated
-- on "no real runner enabled" in `DefaultRunnerFactory`, not on a row
-- here. Persisting `mock` would invite a "mock + claude both enabled"
-- footgun the current factory's invariant explicitly forbids.
CREATE TABLE runner_config (
    runner_id TEXT PRIMARY KEY,
    enabled   INTEGER NOT NULL
);
