-- Per-stage runner session id, captured the first time a task on the
-- stage reports a non-empty `RunResult.session_id`. NULL until set;
-- never cleared, and never reused by a later stage (SCOPE.md hard
-- rule #1: the stage is the session boundary). Subsequent tasks
-- within the same stage resume this session via `--continue` — the
-- handle that makes mid-stage pause / resume continuous.

ALTER TABLE stages ADD COLUMN session_id TEXT;
