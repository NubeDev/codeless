-- Persist the persona the user picked at submit time alongside the
-- already-resolved `system_prompt`. The prompt is the executed text;
-- `persona_id` is the lookup key that produced it. A rerun is supposed
-- to reproduce the exact agent posture, so the id rides the row even
-- though the runtime still composes the prompt from `system_prompt`.
--
-- Personas currently live in the UI KV store; the column is a free
-- TEXT, NOT a foreign key. The personas-to-SQLite migration lands in
-- a later stage and only then does this column gain a `REFERENCES`.
-- Existing rows get NULL — they pre-date the persona picker and the
-- server default applies as before.

ALTER TABLE jobs ADD COLUMN persona_id TEXT;
