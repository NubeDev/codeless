-- Surface F dependency #7a: per-job auto-bypass policy column on
-- `jobs`. Set at submit time; nullable so existing rows keep the
-- halt-on-failure default. The column is JSON-encoded TEXT because
-- the `Custom { comment }` variant carries operator-supplied
-- free-text — a single TEXT keeps the column count flat and matches
-- the serde-tagged wire form (see codeless-types::auto_bypass).
--
-- The `stages.bypassed_at` and `stages.bypassed_reason` columns
-- Surface F also needs are added by `0013_stage_bypassed.sql`,
-- which landed first as the Surface E foundation. This migration
-- intentionally does NOT re-add them — sqlx-migrate would fail on
-- the duplicate `ADD COLUMN`. Per the decision recorded in
-- `DOCS/AUTO-BYPASS-DECISIONS.md` Q6, the trailing branch drops the
-- overlapping ALTER TABLE lines on rebase; this is that drop.

ALTER TABLE jobs ADD COLUMN auto_bypass_policy TEXT;
