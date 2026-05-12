use serde::{Deserialize, Serialize};

/// Unix-milliseconds, UTC. Matches the `INTEGER` timestamp columns in
/// `DOCS/SCOPE.md` Appendix A. Stored as `i64` rather than `u64` so SQLite
/// `INTEGER` round-trips with `sqlx` (which surfaces signed integers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnixMillis(pub i64);

impl UnixMillis {
    pub const ZERO: Self = Self(0);

    pub fn as_i64(self) -> i64 {
        self.0
    }
}

impl From<i64> for UnixMillis {
    fn from(v: i64) -> Self {
        Self(v)
    }
}
