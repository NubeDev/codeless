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
    root: PathBuf,
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
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve `rel` against `root`, refusing anything that would
    /// escape. The check is done by `Component` walking so we never
    /// touch disk for an obviously-bad path, then a final
    /// `canonicalize` + prefix check catches symlinks. Missing tail
    /// segments (a path to a file the caller is about to create) are
    /// allowed: we resolve the parent and append the final component
    /// untouched.
    fn resolve(&self, rel: &str) -> Result<PathBuf, FsError> {
        let rel_path = Path::new(rel);
        for c in rel_path.components() {
            match c {
                Component::Normal(_) => {}
                Component::CurDir => {}
                _ => return Err(FsError::Escape(rel.to_owned())),
            }
        }
        let joined = self.root.join(rel_path);
        match std::fs::canonicalize(&joined) {
            Ok(canon) => {
                if canon.starts_with(&self.root) {
                    Ok(canon)
                } else {
                    Err(FsError::Escape(rel.to_owned()))
                }
            }
            Err(_) => {
                let parent = joined
                    .parent()
                    .ok_or_else(|| FsError::Escape(rel.to_owned()))?;
                let parent_canon =
                    std::fs::canonicalize(parent).map_err(|_| FsError::Escape(rel.to_owned()))?;
                if !parent_canon.starts_with(&self.root) {
                    return Err(FsError::Escape(rel.to_owned()));
                }
                let tail = joined
                    .file_name()
                    .ok_or_else(|| FsError::Escape(rel.to_owned()))?;
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
    async fn absolute_path_is_rejected() {
        let (_tmp, fs_adapter) = setup();
        let err = fs_adapter.read_file("/etc/passwd").await.unwrap_err();
        assert!(matches!(err, FsError::Escape(_)), "got {err:?}");
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
