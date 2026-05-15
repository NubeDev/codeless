use std::path::{Component, Path, PathBuf};

use codeless_types::{FsEntry, FsEntryKind, UnixMillis};
use thiserror::Error;

/// Host-side filesystem adapter. All paths in the public methods are
/// interpreted relative to the configured `root`; any attempt to
/// resolve a path that escapes the root (via absolute paths, parent
/// segments, or symlinks pointing outside) is rejected with
/// `Escape` *before* touching disk. This is the single trust gate
/// for the `fs.*` RPC surface — every transport ultimately reaches
/// `HostFs` and inherits that guarantee.
///
/// `root` is canonicalised once in the constructor so containment
/// checks compare canonical bytes rather than user-supplied prefixes.
/// A non-existent root is an error: the caller is expected to point
/// the adapter at an existing workspace directory.
#[derive(Debug)]
pub struct HostFs {
    /// Primary root — what `fs_cwd` returns and the default join
    /// target for relative paths.
    root: PathBuf,
    /// Additional roots paths are allowed to resolve under. The
    /// worktree root (`--worktree-root`) lives here so the UI can
    /// read per-job `handover.md` / `runs/*/notes/*.md` through
    /// `fs_read_file` without the host adapter rejecting paths that
    /// live outside the source tree. Each extra root is canonical;
    /// the trust check accepts a path that's a descendant of *any*
    /// listed root, including the primary one.
    extra_roots: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum FsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("path escapes root: {0}")]
    Escape(String),
    #[error("not a utf-8 text file: {0}")]
    NotUtf8(String),
    #[error("root does not exist or is not a directory: {0}")]
    BadRoot(PathBuf),
}

impl HostFs {
    /// Construct an adapter rooted at `root`. The path must exist and
    /// be a directory; otherwise `BadRoot` is returned so the caller
    /// can surface the misconfiguration at startup rather than at
    /// first request.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, FsError> {
        let root = root.into();
        let canonical = std::fs::canonicalize(&root).map_err(|_| FsError::BadRoot(root.clone()))?;
        if !canonical.is_dir() {
            return Err(FsError::BadRoot(canonical));
        }
        Ok(Self {
            root: canonical,
            extra_roots: Vec::new(),
        })
    }

    /// Register an extra root readable through the `fs_*` surface.
    /// Use cases: the worktree root (per-job checkouts and their
    /// `runs/*/handover.md` files), a tmp scratch dir for shared
    /// uploads. The path must exist and be a directory — same
    /// contract as the primary root.
    pub fn with_extra_root(mut self, root: impl Into<PathBuf>) -> Result<Self, FsError> {
        let raw = root.into();
        let canonical = std::fs::canonicalize(&raw).map_err(|_| FsError::BadRoot(raw.clone()))?;
        if !canonical.is_dir() {
            return Err(FsError::BadRoot(canonical));
        }
        self.extra_roots.push(canonical);
        Ok(self)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn allowed_under_any_root(&self, path: &Path) -> bool {
        if path.starts_with(&self.root) {
            return true;
        }
        self.extra_roots.iter().any(|r| path.starts_with(r))
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

    /// Resolve `path` against `root`, refusing anything that would
    /// escape. Two input shapes are accepted:
    ///
    /// - Relative paths (`"."`, `"src/lib.rs"`): joined onto `root`.
    ///   `ParentDir` segments are rejected up front so an obvious
    ///   traversal never touches disk.
    /// - Absolute paths (`"/home/user/proj/src/lib.rs"`): used
    ///   directly. The UI's explorer treats the result of `fs_cwd`
    ///   as the display root and ships absolute paths back over the
    ///   wire; accepting them is what makes the explorer round-trip.
    ///
    /// Both shapes finish at the same `canonicalize + starts_with
    /// (root)` check, so symlinks pointing out and absolute paths
    /// outside the root are caught identically. Missing tail
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
            self.root.join(raw)
        };
        match std::fs::canonicalize(&joined) {
            Ok(canon) => {
                if self.allowed_under_any_root(&canon) {
                    Ok(canon)
                } else {
                    Err(FsError::Escape(path.to_owned()))
                }
            }
            Err(_) => {
                let parent = joined
                    .parent()
                    .ok_or_else(|| FsError::Escape(path.to_owned()))?;
                let parent_canon =
                    std::fs::canonicalize(parent).map_err(|_| FsError::Escape(path.to_owned()))?;
                if !self.allowed_under_any_root(&parent_canon) {
                    return Err(FsError::Escape(path.to_owned()));
                }
                let tail = joined
                    .file_name()
                    .ok_or_else(|| FsError::Escape(path.to_owned()))?;
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
        let abs = match self.resolve(rel) {
            Ok(p) => p,
            Err(FsError::Escape(_)) => return Err(FsError::Escape(rel.to_owned())),
            Err(e) => return Err(e),
        };
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
    async fn parent_traversal_is_rejected() {
        let (_tmp, fs_adapter) = setup();
        let err = fs_adapter.read_dir("../etc").await.unwrap_err();
        assert!(matches!(err, FsError::Escape(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn absolute_path_outside_root_is_rejected() {
        let (_tmp, fs_adapter) = setup();
        let err = fs_adapter.read_file("/etc/passwd").await.unwrap_err();
        assert!(matches!(err, FsError::Escape(_)), "got {err:?}");
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
    async fn symlink_pointing_outside_is_rejected() {
        let (tmp, fs_adapter) = setup();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret"), "shhh").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), tmp.path().join("leak")).unwrap();
        let err = fs_adapter.read_file("leak").await.unwrap_err();
        assert!(matches!(err, FsError::Escape(_)), "got {err:?}");
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
}
