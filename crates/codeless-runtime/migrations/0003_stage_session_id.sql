-- Per-stage runner session id, captured the first time a task on the
-- stage reports a non-empty `RunResult.session_id`. NULL until set;
-- never cleared. Observability only — codeless does not reuse this to
-- resume a runner (SCOPE.md hard rule #1).

ALTER TABLE stages ADD COLUMN session_id TEXT;
