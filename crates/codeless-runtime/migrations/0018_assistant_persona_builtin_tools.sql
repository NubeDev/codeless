-- PS8 (DOCS/PLUGIN-SUBSTRATE.md item 8): the Assistant agent loop drives
-- tool calls from the existing `ai-runner` using the persona's
-- `allowed_tools` list as the cap. The substrate-doc PS8 acceptance
-- ("an Assistant thread with the `general` persona can call one
-- read-only tool (e.g. list_jobs) end-to-end") presupposes the
-- assistant's built-in action catalogue is reachable from the
-- general persona; the PS5 seed left `allowed_tools = []` because at
-- the time only MCP/plugin tools were planned to live behind the cap.
--
-- This migration grants `assistant.*` on the two seeded built-in
-- personas so the planner's catalogue (`list_jobs`, `get_job`, …,
-- exposed under the `assistant.<verb>` namespace by
-- `assistant_planner::assistant_tool_id`) is reachable by default.
-- The coding persona keeps its existing `fs.*` / `shell.*` /
-- `attachments.read` grants and gains `assistant.*` so it can also
-- view + drive jobs from a coding thread; the general persona starts
-- with just `assistant.*`. Plugin-supplied personas continue to
-- declare their own `allowed_tools` via `plugin.toml`; this migration
-- does not touch any non-built-in row.

UPDATE personas
   SET allowed_tools = '["assistant.*"]',
       updated_at = strftime('%s', 'now') * 1000
 WHERE id = 'builtin:general';

UPDATE personas
   SET allowed_tools = '["fs.*","shell.*","attachments.read","assistant.*"]',
       updated_at = strftime('%s', 'now') * 1000
 WHERE id = 'builtin:coding';
