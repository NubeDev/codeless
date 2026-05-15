use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use codeless_types::{FsEntry, FsEntryKind, UnixMillis};
use thiserror::Error;

/// Host-side filesystem adapter. The adapter owns a set of canonical
/// allowed roots; any path the `fs.*` RPC surface receives must
/// canonicalise to a descendant of *some* root in that set or the
/// call is rejected with `PermissionDenied` before touching disk.
/// The set is mutable through `add_root` / `remove_root` so the
/// `attach_workspace` / `detach_workspace` RPCs can keep the adapter
/// in sync with the `attached_workspaces` table without rebuilding
/// the runtime — the host adapter is the single trust gate, and
/// attach/detach is the verb that toggles a root in or out of it.
///
/// Every stored entry is canonical (symlinks resolved, no trailing
/// slash, no `.` components) so containment checks compare canonical
/// bytes rather than user-supplied prefixes. The order is preserved:
/// `roots[0]` is whichever path was attached first and is the value
/// `fs_cwd` returns to the UI, so an explorer opened against a freshly
/// booted server lands at the bootstrap workspace.
#[derive(Debug)]
pub struct HostFs {
    roots: RwLock<Vec<PathBuf>>,
}

#[derive(Debug, Error)]
pub enum FsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Lexical `..` in a relative path. Caught before touching the
    /// disk because the segment itself is the refusal signal — no
    /// allowed-roots check would let `..` through.
    #[error("path escapes root: {0}")]
    Escape(String),
    /// The path canonicalised to somewhere outside every configured
    /// allowed root. Distinct from `Escape` because the input was
    /// well-formed; the adapter refused it on policy, not syntax.
    /// `WORKSPACE-ATTACH.md` is explicit that detached workspaces
    /// surface as `PermissionDenied`, not `Internal`.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("not a utf-8 text file: {0}")]
    NotUtf8(String),
    #[error("root does not exist or is not a directory: {0}")]
    BadRoot(PathBuf),
}

impl HostFs {
    /// Construct an adapter rooted at `root`. The path must exist and
    /// be a directory; the canonicalised form becomes the first entry
    /// in the allowed-roots list (and the value `fs_cwd` returns).
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, FsError> {
        let canonical = canonicalise_root(root.into())?;
        Ok(Self {
            roots: RwLock::new(vec![canonical]),
        })
    }

    /// Construct an empty adapter — `fs.*` calls return
    /// `PermissionDenied` for every path until a root is added. This
    /// is the shape the runtime uses when boot finds no `--fs-root`
    /// and no rows in `attached_workspaces`: the adapter exists but
    /// has nothing to serve.
    pub fn empty() -> Self {
        Self {
            roots: RwLock::new(Vec::new()),
        }
    }

    /// Builder-style extra root. Kept as a convenience for callers
    /// that compose the adapter at boot (`HostFs::new(repo)
    /// .with_extra_root(worktree_root)`); equivalent to constructing
    /// then calling `add_root` once.
    pub fn with_extra_root(self, root: impl Into<PathBuf>) -> Result<Self, FsError> {
        self.add_root(root)?;
        Ok(self)
    }

    /// Register an allowed root. The path is canonicalised first so
    /// `/a/b`, `/a/b/`, and a symlink resolving to `/a/b` all collapse
    /// to the same entry. Already-present canonical paths are a
    /// no-op; the function returns the canonical form either way so
    /// callers (the attach handler) can log what actually went in.
    pub fn add_root(&self, root: impl Into<PathBuf>) -> Result<PathBuf, FsError> {
        let canonical = canonicalise_root(root.into())?;
        let mut roots = self.roots.write().expect("HostFs roots poisoned");
        if !roots.iter().any(|r| r == &canonical) {
            roots.push(canonical.clone());
        }
        Ok(canonical)
    }

    /// Drop an allowed root. Idempotent: removing a path that's not
    /// in the set is fine, which lines up with the detach RPC's
    /// "delete the row, then mirror into the adapter" sequencing —
    /// double-detach must not error out the second time. Returns
    /// whether the set actually changed.
    pub fn remove_root(&self, root: impl AsRef<Path>) -> bool {
        let target = std::fs::canonicalize(root.as_ref()).ok();
        let mut roots = self.roots.write().expect("HostFs roots poisoned");
        let before = roots.len();
        // The stored set is canonical, so the match needs the
        // canonical form of the input. When the path no longer
        // resolves on disk (workspace folder vanished), fall back to
        // a lexical compare so a `remove_root` against a stale path
        // can still clean the entry up.
        roots.retain(|r| {
            if let Some(canon) = target.as_deref() {
                r != canon
            } else {
                r.as_path() != root.as_ref()
            }
        });
        roots.len() != before
    }

    /// Snapshot of the current canonical allowed roots in registration
    /// order. The liveness sweep (stage 7) and tests use this to walk
    /// what the adapter currently serves.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.read().expect("HostFs roots poisoned").clone()
    }

    /// The first-registered root; what `fs_cwd` returns. `None` when
    /// no workspace is attached — the runtime maps that to a typed
    /// "no workspace" error so the UI can render the empty state.
    pub fn root(&self) -> Option<PathBuf> {
        self.roots
            .read()
            .expect("HostFs roots poisoned")
            .first()
            .cloned()
    }

    fn allowed_under_any_root(&self, path: &Path) -> bool {
        self.roots
            .read()
            .expect("HostFs roots poisoned")
            .iter()
            .any(|r| path.starts_with(r))
    }

    /// Public sandbox check for absolute paths handed in by callers
    /// outside the read/write/move API (e.g. `agent_chat`'s per-call
    /// cwd override). Canonicalises the input first so symlinks can't
    /// escape; returns false when the path does not exist or sits
    /// outside every configured root.
    pub fn is_path_allowed(&self, abs: &Path) -> bool {
        match std::fs::canonicalize(abs) {
            Ok(canon) => self.allowed_under_any_root(&canon),
            Err(_) => false,
        }
    }

    /// Resolve `path` against the allowed roots, refusing anything
    /// that would escape. Two input shapes are accepted:
    ///
    /// - Relative paths (`"."`, `"src/lib.rs"`): joined onto the
    ///   first root. `ParentDir` segments are rejected up front so an
    ///   obvious traversal never touches disk.
    /// - Absolute paths (`"/home/user/proj/src/lib.rs"`): used
    ///   directly. The UI's explorer treats the result of `fs_cwd`
    ///   as the display root and ships absolute paths back over the
    ///   wire; accepting them is what makes the explorer round-trip.
    ///
    /// Both shapes finish at the same `canonicalize + starts_with(any
    /// root)` check, so symlinks pointing out and absolute paths
    /// outside the allowed set are caught identically. Missing tail
    /// segments (a path to a file the caller is about to create)
    /// resolve via the parent so writes to new files work.
    fn resolve(&self, path: &str) -> Result<PathBuf, FsError> {
        let raw = Path::new(path);
        for c in raw.components() {
            match c {
                Component::Normal(_) | Component::CurDir | Component::RootDir => {}
                Component::Prefix(_) => {}
                Component::ParentDir => return Err(FsError::Escape(path.to_owned())),
            }
        }
        let joined = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            // Relative paths need *some* base; lean on the
            // first-registered root so the bootstrap workspace stays
            // the implicit cwd. With no roots, every relative path
            // is denied — there's no plausible base.
            let base = self.root().ok_or_else(|| {
                FsError::PermissionDenied(format!("no workspaces attached: {path}"))
            })?;
            base.join(raw)
        };
        match std::fs::canonicalize(&joined) {
            Ok(canon) => {
                if self.allowed_under_any_root(&canon) {
                    Ok(canon)
                } else {
                    Err(FsError::PermissionDenied(path.to_owned()))
                }
            }
            Err(_) => {
                let parent = joined
                    .parent()
                    .ok_or_else(|| FsError::PermissionDenied(path.to_owned()))?;
                let parent_canon = std::fs::canonicalize(parent)
                    .map_err(|_| FsError::PermissionDenied(path.to_owned()))?;
                if !self.allowed_under_any_root(&parent_canon) {
                    return Err(FsError::PermissionDenied(path.to_owned()));
                }
                let tail = joined
                    .file_name()
                    .ok_or_else(|| FsError::PermissionDenied(path.to_owned()))?;
                Ok(parent_canon.join(tail))
            }
        }
    }

    pub async fn read_dir(&self, rel: &str) -> Result<Vec<FsEntry>, FsError> {
        let abs = self.resolve(rel)?;
        let mut iter = tokio::fs::read_dir(&abs).await?;
        let mut out = Vec::new();
        while let Some(entry) = iter.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata().await?;
            let kind = if meta.is_dir() {
                FsEntryKind::Dir
            } else if meta.file_type().is_symlink() {
                FsEntryKind::Symlink
            } else {
                FsEntryKind::File
            };
            let size = if meta.is_file() {
                Some(meta.len() as i64)
            } else {
                None
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| UnixMillis(d.as_millis() as i64));
            out.push(FsEntry {
                name,
                kind,
                size,
                mtime,
            });
        }
        // Deterministic order makes the explorer UI stable across
        // platform-specific readdir ordering (ext4 vs APFS vs NTFS).
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub async fn read_file(&self, rel: &str) -> Result<String, FsError> {
        let abs = self.resolve(rel)?;
        let bytes = tokio::fs::read(&abs).await?;
        String::from_utf8(bytes).map_err(|_| FsError::NotUtf8(rel.to_owned()))
    }

    pub async fn write_file(&self, rel: &str, content: &str) -> Result<(), FsError> {
        let abs = self.resolve(rel)?;
        tokio::fs::write(&abs, content.as_bytes()).await?;
        Ok(())
    }

    /// Stat one path. Returns `None` if the path does not exist;
    /// `Some` if it does (or its parent is reachable and the tail is
    /// a dangling symlink, which `symlink_metadata` will surface).
    pub async fn stat(
        &self,
        rel: &str,
    ) -> Result<Option<(FsEntryKind, Option<i64>, Option<UnixMillis>)>, FsError> {
        let abs = self.resolve(rel)?;
        let meta = match tokio::fs::symlink_metadata(&abs).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let kind = if meta.file_type().is_symlink() {
            FsEntryKind::Symlink
        } else if meta.is_dir() {
            FsEntryKind::Dir
        } else {
            FsEntryKind::File
        };
        let size = if meta.is_file() {
            Some(meta.len() as i64)
        } else {
            None
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| UnixMillis(d.as_millis() as i64));
        Ok(Some((kind, size, mtime)))
    }

    /// Create a file. When `content` is `None` the file is empty.
    /// When `overwrite` is false and the path already exists, returns
    /// `Io(AlreadyExists)`.
    pub async fn create_file(
        &self,
        rel: &str,
        content: Option<&str>,
        overwrite: bool,
    ) -> Result<(), FsError> {
        let abs = self.resolve(rel)?;
        if !overwrite && abs.exists() {
            return Err(FsError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("file already exists: {}", abs.display()),
            )));
        }
        let bytes = content.unwrap_or("").as_bytes();
        tokio::fs::write(&abs, bytes).await?;
        Ok(())
    }

    /// Create a directory. When `recursive` is true, missing ancestors
    /// are created; when false, the parent must already exist.
    pub async fn create_dir(&self, rel: &str, recursive: bool) -> Result<(), FsError> {
        let abs = self.resolve(rel)?;
        if recursive {
            tokio::fs::create_dir_all(&abs).await?;
        } else {
            tokio::fs::create_dir(&abs).await?;
        }
        Ok(())
    }

    /// Move (rename) a path. Both `from` and `to` must resolve within
    /// the sandbox. When `overwrite` is false and `to` already exists,
    /// returns `AlreadyExists`.
    pub async fn rename(&self, from: &str, to: &str, overwrite: bool) -> Result<(), FsError> {
        let abs_from = self.resolve(from)?;
        let abs_to = self.resolve(to)?;
        if !overwrite && abs_to.exists() {
            return Err(FsError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("target already exists: {}", abs_to.display()),
            )));
        }
        tokio::fs::rename(&abs_from, &abs_to).await?;
        Ok(())
    }

    /// Delete a path. When `recursive` is true, directories are
    /// removed with all contents; when false, only empty directories
    /// and files are removed.
    pub async fn delete(&self, rel: &str, recursive: bool) -> Result<(), FsError> {
        let abs = self.resolve(rel)?;
        let meta = tokio::fs::symlink_metadata(&abs).await?;
        if meta.is_dir() {
            if recursive {
                tokio::fs::remove_dir_all(&abs).await?;
            } else {
                tokio::fs::remove_dir(&abs).await?;
            }
        } else {
            tokio::fs::remove_file(&abs).await?;
        }
        Ok(())
    }
}

fn canonicalise_root(raw: PathBuf) -> Result<PathBuf, FsError> {
    let canonical = std::fs::canonicalize(&raw).map_err(|_| FsError::BadRoot(raw.clone()))?;
    if !canonical.is_dir() {
        return Err(FsError::BadRoot(canonical));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, HostFs) {
        let tmp = tempdir().unwrap();
        let fs_adapter = HostFs::new(tmp.path()).unwrap();
        (tmp, fs_adapter)
    }

    #[tokio::test]
    async fn write_then_read_round_trips() {
        let (_tmp, fs_adapter) = setup();
        fs_adapter.write_file("hello.txt", "world").await.unwrap();
        let got = fs_adapter.read_file("hello.txt").await.unwrap();
        assert_eq!(got, "world");
    }

    #[tokio::test]
    async fn read_dir_lists_entries_sorted_with_kinds() {
        let (tmp, fs_adapter) = setup();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("a.txt"), "x").unwrap();
        fs::write(tmp.path().join("b.txt"), "yy").unwrap();
        let entries = fs_adapter.read_dir(".").await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "sub"]);
        assert_eq!(entries[0].kind, FsEntryKind::File);
        assert_eq!(entries[0].size, Some(1));
        assert_eq!(entries[2].kind, FsEntryKind::Dir);
        assert_eq!(entries[2].size, None);
    }

    #[tokio::test]
    async fn stat_missing_returns_none() {
        let (_tmp, fs_adapter) = setup();
        assert!(fs_adapter.stat("no-such.txt").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn parent_traversal_is_rejected_as_escape() {
        let (_tmp, fs_adapter) = setup();
        let err = fs_adapter.read_dir("../etc").await.unwrap_err();
        assert!(matches!(err, FsError::Escape(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn absolute_path_outside_roots_is_permission_denied() {
        let (_tmp, fs_adapter) = setup();
        let err = fs_adapter.read_file("/etc/passwd").await.unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn absolute_path_inside_root_is_allowed() {
        let (tmp, fs_adapter) = setup();
        fs::write(tmp.path().join("note.md"), "ok").unwrap();
        let canonical_root = std::fs::canonicalize(tmp.path()).unwrap();
        let abs = canonical_root.join("note.md");
        let got = fs_adapter
            .read_file(abs.to_str().unwrap())
            .await
            .expect("absolute-in-root should resolve");
        assert_eq!(got, "ok");

        let entries = fs_adapter
            .read_dir(canonical_root.to_str().unwrap())
            .await
            .expect("absolute root dir should list");
        assert!(entries.iter().any(|e| e.name == "note.md"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_pointing_outside_is_permission_denied() {
        let (tmp, fs_adapter) = setup();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret"), "shhh").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), tmp.path().join("leak")).unwrap();
        let err = fs_adapter.read_file("leak").await.unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn non_utf8_read_returns_typed_error() {
        let (tmp, fs_adapter) = setup();
        fs::write(tmp.path().join("bin"), [0xff, 0xfe, 0x00]).unwrap();
        let err = fs_adapter.read_file("bin").await.unwrap_err();
        assert!(matches!(err, FsError::NotUtf8(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn bad_root_is_caught_in_constructor() {
        let err = HostFs::new("/nonexistent/path/should/not/exist").unwrap_err();
        assert!(matches!(err, FsError::BadRoot(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn write_to_new_file_in_existing_dir_works() {
        let (_tmp, fs_adapter) = setup();
        fs_adapter.write_file("new.txt", "content").await.unwrap();
        let stat = fs_adapter.stat("new.txt").await.unwrap();
        let (kind, size, _) = stat.unwrap();
        assert_eq!(kind, FsEntryKind::File);
        assert_eq!(size, Some(7));
    }

    #[tokio::test]
    async fn add_root_makes_outside_path_resolvable() {
        let (_tmp, fs_adapter) = setup();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("note.md"), "extra").unwrap();
        let extra_abs = std::fs::canonicalize(outside.path())
            .unwrap()
            .join("note.md");
        let err = fs_adapter
            .read_file(extra_abs.to_str().unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied(_)));

        fs_adapter.add_root(outside.path()).unwrap();
        let got = fs_adapter
            .read_file(extra_abs.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(got, "extra");
    }

    #[tokio::test]
    async fn remove_root_revokes_access_to_that_subtree() {
        let (tmp, fs_adapter) = setup();
        let extra = tempdir().unwrap();
        fs::write(extra.path().join("a.txt"), "1").unwrap();
        fs_adapter.add_root(extra.path()).unwrap();
        let extra_abs = std::fs::canonicalize(extra.path()).unwrap().join("a.txt");
        fs_adapter
            .read_file(extra_abs.to_str().unwrap())
            .await
            .unwrap();

        let changed = fs_adapter.remove_root(extra.path());
        assert!(changed);
        let err = fs_adapter
            .read_file(extra_abs.to_str().unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied(_)), "got {err:?}");

        // The original root keeps working — remove only revoked the
        // one entry it matched.
        fs_adapter.write_file("still.txt", "ok").await.unwrap();
        let _ = tmp;
    }

    #[tokio::test]
    async fn remove_root_is_idempotent() {
        let (_tmp, fs_adapter) = setup();
        let extra = tempdir().unwrap();
        fs_adapter.add_root(extra.path()).unwrap();
        assert!(fs_adapter.remove_root(extra.path()));
        assert!(!fs_adapter.remove_root(extra.path()));
    }

    #[tokio::test]
    async fn add_root_canonicalises_so_duplicates_collapse() {
        let (_tmp, fs_adapter) = setup();
        let extra = tempdir().unwrap();
        let canon = std::fs::canonicalize(extra.path()).unwrap();
        let with_trailing = {
            let mut s = canon.to_string_lossy().into_owned();
            s.push('/');
            PathBuf::from(s)
        };
        fs_adapter.add_root(extra.path()).unwrap();
        fs_adapter.add_root(&with_trailing).unwrap();
        fs_adapter.add_root(extra.path().join(".")).unwrap();
        // One bootstrap root + one extra (collapsed across forms).
        assert_eq!(fs_adapter.roots().len(), 2);
    }

    #[tokio::test]
    async fn empty_adapter_denies_everything() {
        let fs_adapter = HostFs::empty();
        let err = fs_adapter.read_file("/etc/passwd").await.unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied(_)), "got {err:?}");
        let err = fs_adapter.read_dir(".").await.unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied(_)), "got {err:?}");
        assert!(fs_adapter.root().is_none());
    }
}
