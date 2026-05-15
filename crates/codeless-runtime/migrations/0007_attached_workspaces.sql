-- WORKSPACE-ATTACH milestone 2: the set of filesystem roots the server
-- will serve `fs.*` calls under at runtime. Replaces the single
-- `--fs-root` flag as the source of truth; the flag becomes a
-- bootstrap convenience that upserts one row at boot.
--
-- Two path columns by design:
--   `fs_root_canonical` — symlinks resolved, trailing slashes and `.`
--     segments stripped. The unique index sits here so `/a/b`, `/a/b/`,
--     and `symlink-to-/a/b` all collapse onto one row regardless of
--     how the operator typed them.
--   `fs_root_display`   — the user-supplied string, kept verbatim for
--     the workspaces sidebar so the UI can render the path the user
--     recognises, not the resolved one (which may differ on macOS
--     `/var` ↔ `/private/var` or under bind-mounts).
--
-- `repo_id` is the foreign key into `repos`; one repo, one attachment
-- at a time (PRIMARY KEY). `ON DELETE CASCADE` because the
-- attachment is meaningless without the repo row it points to —
-- destructive removal is `remove_repo`, reversible removal is
-- `detach_workspace` which drops only this row.
CREATE TABLE attached_workspaces (
    repo_id           TEXT PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    fs_root_canonical TEXT NOT NULL,
    fs_root_display   TEXT NOT NULL,
    attached_at       INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_attached_workspaces_canonical
    ON attached_workspaces(fs_root_canonical);
