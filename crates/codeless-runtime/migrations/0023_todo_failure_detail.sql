-- Human-readable failure reason for a todo row that ended `Failed`.
-- Populated by the trio emitters (handover write error, verify step
-- exit code, git commit error) and surfaced to the stage's
-- `failure_detail` via the closing-trio gate so the UI shows *which*
-- rail failed and *why* instead of leaving the stage in a silent
-- `Failed` state. Nullable: only meaningful when `status = 'failed'`,
-- always null for `done` / `skipped` / non-terminal rows.

ALTER TABLE todos ADD COLUMN failure_detail TEXT;
