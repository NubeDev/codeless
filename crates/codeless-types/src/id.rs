use serde::{Deserialize, Serialize};
use ulid::Ulid;

macro_rules! ulid_newtype {
    ($name:ident, $desc:literal) => {
        #[doc = $desc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Ulid);

        impl $name {
            pub fn new() -> Self {
                Self(Ulid::new())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Ulid> for $name {
            fn from(u: Ulid) -> Self {
                Self(u)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_str(s).map(Self)
            }
        }
    };
}

ulid_newtype!(RepoId, "Identity of a managed git repository row.");
ulid_newtype!(JobId, "Identity of one unit of work scoped to one repo.");
ulid_newtype!(StageId, "Identity of a verify-gated chunk within a job.");
ulid_newtype!(
    TaskId,
    "Identity of one atomic runner invocation within a stage."
);
ulid_newtype!(ReviewId, "Identity of a review gate attached to a stage.");
