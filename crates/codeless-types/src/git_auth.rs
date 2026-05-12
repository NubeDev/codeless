use serde::{Deserialize, Serialize};

/// How the core authenticates to a given repo's remote. Stored as JSON in
/// the `repos.git_auth` column (SCOPE.md Appendix A). Variant kinds are
/// stable wire labels; renaming any of them is a breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitAuth {
    Ssh {
        key_path: String,
    },
    Token {
        env_var: String,
    },
    GithubApp {
        app_id: String,
        installation_id: String,
    },
}
