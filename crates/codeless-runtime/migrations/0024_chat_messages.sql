-- Per-Job chat substrate (JOB-CHAT.md §Data model). One row per
-- message regardless of which transport produced it: the web chat
-- panel, the Telegram bot, the Slack bot, the CLI, and the
-- supervisor agent all write through here. The supervisor reads
-- per-Job (by job_id) and writes back with transport='supervisor';
-- it is a consumer, not a peer surface.
--
-- run_id is nullable on purpose. JOB-WORKFLOW (B) — the Job/Run
-- split — has not landed yet, so pre-(B) rows leave it NULL. Once
-- (B) lands, fresh inserts populate it for UI filtering and
-- analytics only; the supervisor's reading view stays per-Job. See
-- OQ-CHAT-4 in JOB-CHAT.md for the resolution.
--
-- external_id is the transport-native message id (Telegram chat+id,
-- Slack ts, etc.) and is required for transports that have one
-- (telegram, slack) so we can recognise echoes and forward-delivery
-- receipts without double-ingest. Web / CLI / supervisor messages
-- leave it NULL and are uniquified by the ULID primary key alone.
CREATE TABLE chat_messages (
    id              TEXT PRIMARY KEY,
    job_id          TEXT NOT NULL REFERENCES jobs(id),
    run_id          TEXT,
    transport       TEXT NOT NULL,
    external_id     TEXT,
    thread_key      TEXT,
    author          TEXT NOT NULL,
    role            TEXT NOT NULL,
    body            TEXT NOT NULL,
    metadata_json   TEXT,
    created_at      INTEGER NOT NULL
);

-- The supervisor and every transport reads history per Job, ordered
-- chronologically; list_job_messages paginates by created_at with
-- the id as tiebreaker. Index covers both the equality filter and
-- the sort key.
CREATE INDEX chat_messages_job_idx ON chat_messages (job_id, created_at);

-- Partial unique on (transport, external_id) — narrowed to rows
-- whose external_id is non-NULL. SQLite treats every NULL as
-- distinct under a UNIQUE constraint, so a naive
-- `UNIQUE (transport, external_id)` would silently allow duplicate
-- ingest for any transport that ever sends a NULL external_id (web,
-- cli, supervisor — all three). The partial form keeps the
-- duplicate-ingest defence for the transports whose invariant is
-- that external_id must exist (telegram, slack) and lets the other
-- three rely on their ULID primary key for identity.
CREATE UNIQUE INDEX chat_messages_external_idx
    ON chat_messages (transport, external_id)
    WHERE external_id IS NOT NULL;
