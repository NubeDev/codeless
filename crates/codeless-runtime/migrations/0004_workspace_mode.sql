-- R0: workspace_mode column. Default 'in-repo' matches the new
-- default for fresh jobs. Existing rows (all worktree-created)
-- are backfilled to 'worktree' so their semantics are preserved.
ALTER TABLE jobs ADD COLUMN workspace_mode TEXT NOT NULL DEFAULT 'in-repo';

UPDATE jobs SET workspace_mode = 'worktree' WHERE worktree_path IS NOT NULL;
