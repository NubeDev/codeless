-- Plugin #0 (`notes`) — initial schema.
--
-- Substrate doc OQ-PS-4: every plugin-owned table is named
-- `<plugin_id>_<table>`. `codeless_tools::plugin::check_migration_sql`
-- statically rejects any CREATE/ALTER/DROP outside that namespace at
-- load time, so a stray reference to e.g. `personas` here would fail
-- the plugin load rather than scribble on a codeless-owned table.

CREATE TABLE IF NOT EXISTS notes_entries (
    id         TEXT PRIMARY KEY,
    thread_id  TEXT NOT NULL,
    body       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS notes_entries_thread_idx
    ON notes_entries(thread_id, created_at);
