-- Adds machine-readable failure-reason columns to `stages`. Written
-- by the StageRecorder in the same SQL update as `status = 'failed'`
-- from the `Event::StageCompleted` payload, and by
-- `reap_orphan_running_stages` for the crash-recovery path.
--
-- `failure_class` is one of the wire labels in `FailureClass`
-- (`pre-check-failed`, `runner-error`, `review-patch-invalid`,
-- `review-fail`, `review-unparseable`, `orphan-reap`). `failure_detail`
-- is a short human-readable line (~200 chars) — the runner's reason,
-- the REVIEW model's FAIL text, or the validator error.
--
-- Both nullable, no default. NULL means either `status != 'failed'`
-- or the row was written before this column existed; readers treat
-- the latter as a legacy / unclassified failure and fall back to the
-- event stream.

ALTER TABLE stages ADD COLUMN failure_class  TEXT;
ALTER TABLE stages ADD COLUMN failure_detail TEXT;
