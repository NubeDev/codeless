-- Per-job runner overrides. Adapters silently ignore knobs they don't
-- support (Copilot has no permission_mode / effort); the columns exist
-- on every row so the wire shape and the schema agree, with NULL
-- meaning "use the adapter's default."

ALTER TABLE jobs ADD COLUMN model            TEXT;
ALTER TABLE jobs ADD COLUMN permission_mode  TEXT;
ALTER TABLE jobs ADD COLUMN effort           TEXT;
