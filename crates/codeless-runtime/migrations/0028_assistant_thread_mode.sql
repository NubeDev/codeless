-- Job `assistant-fs-tools` stage 3: per-thread filesystem permission
-- posture for the in-app Assistant. `mode` selects which fs.* tools
-- the planner registers and how `fs.write` / `fs.edit` dispatch
-- (immediate, action-card-gated, or refused). Server-side enforcement
-- — the column is the source of truth (R4), the UI dropdown lands in
-- stage 7 and only mirrors it.
--
-- Three wire values: `read-only`, `approve-edits`, `bypass`. The
-- vocabulary aligns with Claude Code / Copilot CLI / Codex (SCOPE.md
-- Decisions D1) so users carry one mental model across tools.
--
-- No SQLite `CHECK` constraint: the bind-time enum on
-- `AssistantThreadMode::from_wire` rejects unknown strings server-side
-- (D1's typo-reject), and a `CHECK` would force a table rebuild if
-- D1 ever grows a fourth mode. The `NOT NULL DEFAULT 'read-only'`
-- back-fills pre-existing rows to the safest posture, matching the
-- "safe by default" stance.

ALTER TABLE assistant_threads
    ADD COLUMN mode TEXT NOT NULL DEFAULT 'read-only';
