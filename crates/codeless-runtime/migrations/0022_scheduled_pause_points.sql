-- Operator-declared breakpoints lifted out of `template.yaml`. One
-- row per entry in the YAML `pause_points:` list; the parser resolves
-- symbolic stage names and trio kinds at submit time
-- (`JobTemplate::resolve_pause_points`) and the persistence layer
-- writes the resolved tuple here so the runtime hook never has to
-- re-walk the source.
--
-- Primary key is `(job_id, ordinal)` per SCOPED-PAUSE-POINTS §4: the
-- 1-based YAML index is the only stable handle that survives a
-- `resync_template_from_disk` reshuffle of stage ordinals. Resolved
-- stage_ordinal / todo_ordinal columns are *not* part of identity —
-- they sit inside `target_json` and may be different after a resync
-- without orphaning the row. ON DELETE CASCADE on `job_id` matches
-- the cascade pattern the rest of the job-scoped tables use.
--
-- `target_json` carries a `PausePointTarget` serialised with the
-- wire-shape serde derives from `codeless_types::pause_point`, so a
-- column dump round-trips through the same parser the wire uses. The
-- alternative (a wide column-per-field shape) would force every
-- selector variant onto the same row layout and break the type-level
-- "selector absent ⇒ Stage variant" guarantee the wire type encodes.
--
-- `position` is stored as the kebab-case wire label (`before` /
-- `after`) by explicit pattern match in the store layer — the labels
-- are wire-stable, so drift here is a wire-format break.
--
-- `reason` is the operator's optional free-text justification, capped
-- at 512 bytes by the parser (SCOPED-PAUSE-POINTS §1.4). The cap is
-- a parse-time rule, not a column constraint, so a future relaxation
-- is a parser edit alone.
--
-- `created_at` is the row insert time, useful for the eventual
-- `pause_points_updated` resync event payload but otherwise informational.

CREATE TABLE scheduled_pause_points (
    job_id      TEXT    NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    ordinal     INTEGER NOT NULL,
    point_id    TEXT    NOT NULL,
    target_json TEXT    NOT NULL,
    position    TEXT    NOT NULL,
    reason      TEXT,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (job_id, ordinal)
);

CREATE INDEX scheduled_pause_points_job_idx
    ON scheduled_pause_points(job_id, ordinal);
