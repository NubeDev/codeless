-- Adds the bypass-on-stage columns. Set by resume_job's
-- bypass_failing_stage path; read by TemplateRunner's
-- skip-passed-or-bypassed branch so a bypassed stage is treated as
-- terminal-but-advance rather than retry. The stage's `status`
-- column stays `Failed` so the audit trail is honest; bypass is the
-- forward-advance signal, not a status rewrite.
--
-- Nullable, no default. A NULL bypassed_at means the stage was
-- never bypassed (the common case).

ALTER TABLE stages ADD COLUMN bypassed_at      INTEGER;
ALTER TABLE stages ADD COLUMN bypassed_reason  TEXT;
