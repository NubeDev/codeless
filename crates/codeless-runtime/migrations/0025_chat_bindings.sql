-- Adapter-side lookup table (JOB-CHAT.md §Data model). The Telegram
-- and Slack adapters call `bind_chat_thread` on `/codeless bind
-- <job_id>` to record that messages arriving on
-- `(transport, channel_id, thread_id)` should be appended to that
-- Job's chat. Web UI never needs a binding — it knows the job_id
-- from the URL.
--
-- thread_id is NOT NULL with an empty-string sentinel for the
-- "no-thread on this transport" case, deliberately rejecting NULL.
-- SQLite treats every NULL as distinct in primary-key columns, so
-- making thread_id nullable would let the same (transport,
-- channel_id) bind to two different jobs once via `(…, NULL, A)`
-- and again via `(…, NULL, B)` — the primary key would not catch
-- it. The empty string is unambiguous on both Telegram (real
-- message_thread_id values are positive integers serialised as
-- non-empty strings) and Slack (real thread_ts values are non-empty
-- floating-point timestamps), so '' as the sentinel cannot collide
-- with a real thread.
CREATE TABLE chat_bindings (
    transport       TEXT NOT NULL,
    channel_id      TEXT NOT NULL,
    thread_id       TEXT NOT NULL DEFAULT '',
    job_id          TEXT NOT NULL REFERENCES jobs(id),
    bound_at        INTEGER NOT NULL,
    bound_by        TEXT NOT NULL,
    PRIMARY KEY (transport, channel_id, thread_id)
);
