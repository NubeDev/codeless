use serde::{Deserialize, Serialize};

/// USD amount in integer cents. Per SCOPE.md "All money is stored as
/// `INTEGER` cents-USD (no floats, no rounding surprises)." Conversions
/// to display strings live in the UI layer, not here.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    specta::Type,
)]
#[serde(transparent)]
#[specta(transparent)]
pub struct CostCents(pub i64);

impl CostCents {
    pub const ZERO: Self = Self(0);

    pub fn as_i64(self) -> i64 {
        self.0
    }
}

impl From<i64> for CostCents {
    fn from(v: i64) -> Self {
        Self(v)
    }
}
