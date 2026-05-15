-- Persistence for the in-app assistant surface (ASSISTANT-SCOPE.md).
-- The assistant is a conversational front-end over `codeless-runtime`;
-- threads outlive any single job or worktree, so they live in their
-- own tables alongside `jobs` rather than hanging off the job graph.
--
-- ID columns are ULID-as-TEXT to match the existing convention
-- (`jobs.id`, `stages.id` ...). Timestamps are INTEGER Unix-millis UTC.
-- No FK from `assistant_threads` to `repos` or `jobs` on purpose —
-- the assistant is allowed to discuss work across repos, and a deleted
-- repo should not orphan a conversation the user is still reading.

CREATE TABLE assistant_threads (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX assistant_threads_updated_idx
    ON assistant_threads(updated_at DESC);

-- One row per turn or system note in a thread. `role` carries the
-- speaker label: `user` for the operator, `assistant` for the model
-- reply, `system` for runtime-injected context (e.g. "thread renamed",
-- "attachment added"), and `tool` for tool-call summaries the action
-- cards land. Kept as TEXT rather than a CHECK-constrained enum so a
-- later role variant (e.g. `tool_result`) does not require a migration
-- and is gated by serde on the application side instead.
--
-- `content` is the raw markdown body the UI renders. Tool-call
-- structure / attachment refs land in a separate `meta_json` blob
-- shaped like the `chat-message` event payload so the CommonChat
-- renderer can use one code path for the assistant transcript and the
-- live job chat. NULL meta is the bare-text case.
CREATE TABLE assistant_messages (
    id          TEXT PRIMARY KEY,
    thread_id   TEXT NOT NULL REFERENCES assistant_threads(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    meta_json   TEXT,
    created_at  INTEGER NOT NULL
);
CREATE INDEX assistant_messages_thread_idx
    ON assistant_messages(thread_id, created_at);

-- Attachments dropped onto a thread. The blob itself lives under
-- `<codeless-data>/threads/<thread_id>/attachments/<stored_filename>`
-- per SCOPE.md "Decisions / Attachments directory"; this row is the
-- index the UI lists and the cascade target when a thread is deleted.
-- `original_name` survives the storage rename so the UI can render the
-- human-friendly filename; `stored_filename` is the on-disk basename
-- (prefixed with the attachment id to avoid collisions).
CREATE TABLE assistant_attachments (
    id              TEXT PRIMARY KEY,
    thread_id       TEXT NOT NULL REFERENCES assistant_threads(id) ON DELETE CASCADE,
    original_name   TEXT NOT NULL,
    stored_filename TEXT NOT NULL,
    mime_type       TEXT,
    size_bytes      INTEGER NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX assistant_attachments_thread_idx
    ON assistant_attachments(thread_id, created_at);
