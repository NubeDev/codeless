use serde::{Deserialize, Serialize};

use crate::time::UnixMillis;

/// One entry from a directory listing. `size` and `mtime` are
/// `Option` because not every filesystem entry has them in the same
/// way — a symlink to a non-existent target carries no `mtime` from
/// the target, and a directory's `size` is platform-dependent and not
/// useful for UI display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FsEntry {
    pub name: String,
    pub kind: FsEntryKind,
    pub size: Option<i64>,
    pub mtime: Option<UnixMillis>,
}

/// Type of one directory entry. Symlinks are surfaced as their own
/// kind rather than dereferenced because the explorer UI wants to
/// distinguish them (different icon, optional target reveal); the
/// reader follows the link only when the user opens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum FsEntryKind {
    File,
    Dir,
    Symlink,
}
