-- Authored stage metadata, populated from the per-job template. Both
-- columns are nullable so rows written before the field existed keep
-- working: NULL means "this stage predates the field", an empty JSON
-- array means "the author explicitly listed no criteria yet". The UI
-- overview keeps those two states visually distinct.

ALTER TABLE stages ADD COLUMN goal TEXT;
ALTER TABLE stages ADD COLUMN acceptance TEXT;
