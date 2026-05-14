-- Idle-timeout bookkeeping on the per-stage warm session. `session_id`
-- (column added by 0003) is the resume handle; these two columns
-- bound how long that handle is held open after the last touch.
--
-- `last_activity_at` is bumped every time the runtime sees activity on
-- the stage's session (turn start, turn end, user message, sweep tick).
-- It is NULL on rows that have never carried a session. A NULL value
-- never triggers archive: only sessions with a captured id participate.
--
-- `archived` is the terminal state. Once 1, the runtime does not pass
-- `--continue <session_id>` for that stage again; the next user message
-- against the stage opens a fresh session preceded by a handover
-- document. The flag is one-way (no un-archive) so the lifecycle event
-- the UI sees fires exactly once per session.
ALTER TABLE stages ADD COLUMN last_activity_at INTEGER;
ALTER TABLE stages ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
CREATE INDEX stages_idle_idx
    ON stages(last_activity_at)
    WHERE session_id IS NOT NULL AND archived = 0;
