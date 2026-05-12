use serde::{Deserialize, Serialize};

use crate::git_auth::GitAuth;
use crate::id::RepoId;
use crate::time::UnixMillis;

/// A managed git repository row — see SCOPE.md Appendix A `repos`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    pub id: RepoId,
    pub name: String,
    pub clone_url: String,
    pub default_branch: String,
    pub local_path: String,
    pub git_auth: GitAuth,
    /// `None` means "use the global concurrency cap".
    pub concurrency_cap: Option<u32>,
    /// Runner kind preselected for jobs that don't override it.
    pub default_runner: Option<String>,
    pub created_at: UnixMillis,
    pub updated_at: UnixMillis,
}
