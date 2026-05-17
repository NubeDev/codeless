-- Per-job slot for an operator comment that the next runner
-- attempt must read before its first stage. Surface F's auto-bypass
-- path threads its canned guidance inline through `TemplateRunner`
-- state; this column is for the *operator-driven* equivalent:
-- `resume_job(next_stage_comment=...)` writes the text here, the
-- runner factory takes-and-clears it on the next build, and the
-- runner prepends it as `# Operator comment` on the first stage it
-- executes.
--
-- The column is the persistence layer for at-most-once delivery:
-- writing replaces any prior unconsumed value (a second resume
-- before the runner picked up the first wins, by operator intent),
-- and the runner factory clears it the moment it threads the value
-- into the runner so a third resume without a fresh comment does
-- not re-thread the stale one.
--
-- NULL is the default for every existing row and for any job with
-- no pending instruction.

ALTER TABLE jobs ADD COLUMN pending_operator_comment TEXT;
