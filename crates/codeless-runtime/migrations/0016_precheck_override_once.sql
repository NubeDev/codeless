-- One-shot opt-in for the REVIEW stage's diff-verify pre-check.
-- Written by `override_pre_check_and_resume`, consumed atomically by
-- the runner just before the pre-check runs. Distinct from
-- `pending_operator_comment` (which threads forward as prompt text on
-- every resume) because override-once is a structural guard bypass
-- and must not survive past the single re-attempt it was authorised
-- for. Stored as INTEGER 0/1 — SQLite has no native boolean, and the
-- atomic-take pattern (`UPDATE ... WHERE val = 1 RETURNING val`)
-- reads cleaner against an integer column.
ALTER TABLE jobs
    ADD COLUMN precheck_override_once INTEGER NOT NULL DEFAULT 0;
