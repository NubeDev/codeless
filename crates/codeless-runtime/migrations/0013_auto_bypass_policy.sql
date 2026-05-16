-- Surface F dependency #7a: per-job auto-bypass policy column on
-- `jobs`. Set at submit time; nullable so existing rows keep the
-- halt-on-failure default. The column is JSON-encoded TEXT because
-- the `Custom { comment }` variant carries operator-supplied
-- free-text — a single TEXT keeps the column count flat and matches
-- the serde-tagged wire form (see codeless-types::auto_bypass).
--
-- Per `DOCS/AUTO-BYPASS-DECISIONS.md` Q6, this migration also lifts
-- the Surface E bypass columns onto `stages` if they have not landed
-- yet. Surface E (in flight on a sibling branch) and Surface F both
-- need a stage row to record a `bypassed_at` timestamp and a
-- `bypassed_reason` string; the two surfaces share the columns and
-- only differ on the event emitted (`StageBypassed` vs.
-- `StageAutoBypassed`). Lifting them out into a third PR would block
-- both surfaces on coordination, so the decision is to add them in
-- whichever migration lands first and have the trailing branch drop
-- its column-creation lines on rebase. SQLite has no
-- `IF NOT EXISTS` on `ADD COLUMN`; the per-connection runtime checks
-- `PRAGMA table_info` before applying. sqlx-migrate does not honour
-- conditional SQL, so the migration is written so the first apply
-- against any schema with Surface E already landed will error and
-- the Surface E branch will rebase its file out at that point.

ALTER TABLE jobs   ADD COLUMN auto_bypass_policy TEXT;
ALTER TABLE stages ADD COLUMN bypassed_at        INTEGER;
ALTER TABLE stages ADD COLUMN bypassed_reason    TEXT;
