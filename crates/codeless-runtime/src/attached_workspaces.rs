//! Boot-time upsert for the `attached_workspaces` table.
//!
//! Today the server is told *where to work* via `--fs-root <path>` at
//! boot. WORKSPACE-ATTACH milestone 2 turns that flag into a bootstrap
//! convenience: instead of a hidden in-process `Option<PathBuf>`, the
//! path is persisted as a row in `attached_workspaces`, the same table
//! the future runtime `attach_workspace` RPC will write. Repeated boots
//! with `/a/b`, `/a/b/`, or a symlink resolving to `/a/b` all collapse
//! onto the single canonical row by virtue of the unique index on
//! `fs_root_canonical` — see migration `0007_attached_workspaces.sql`.
//!
//! The boot upsert also owns ensuring there *is* a `repos` row to
//! attach to: the `attached_workspaces` PK references `repos(id)`, so a
//! fresh database starting with `--fs-root /some/path` needs a repo
//! created up front. We pick a deterministic ULID derived from the
//! canonical path so the boot upsert is genuinely idempotent — a
//! second boot with the same `--fs-root` does not invent a second
//! repo row.

use std::path::{Path, PathBuf};

use codeless_types::{GitAuth, RepoId, UnixMillis};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::time::now_ms;

#[derive(Debug, thiserror::Error)]
pub enum BootAttachError {
    #[error("canonicalize {path}: {source}")]
    Canonicalize {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("attached_workspaces upsert: {0}")]
    Db(#[from] sqlx::Error),
}

/// Canonicalise `raw` (resolving symlinks, stripping `.` segments and
/// trailing slashes) and ensure an `attached_workspaces` row exists for
/// it, creating a `repos` row first if none is present.
///
/// Idempotent. Two invocations with `/a/b`, `/a/b/`, and a symlink to
/// `/a/b` will leave exactly one row in `attached_workspaces` and one
/// row in `repos`. The `fs_root_display` column captures `raw` as the
/// operator typed it; the canonical column is what the unique index
/// keys on.
pub async fn upsert_boot_workspace(
    pool: &SqlitePool,
    raw: &Path,
) -> Result<BootAttachOutcome, BootAttachError> {
    let canonical_path =
        std::fs::canonicalize(raw).map_err(|source| BootAttachError::Canonicalize {
            path: raw.display().to_string(),
            source,
        })?;
    let canonical = canonical_path.to_string_lossy().into_owned();
    let display = raw.to_string_lossy().into_owned();
    let now = now_ms();

    // If a workspace row already keys on this canonical path, the boot
    // upsert is a no-op — leave `attached_at` and `fs_root_display`
    // alone so re-boot does not churn UI ordering or rewrite the path
    // the operator first registered as.
    let existing: Option<String> =
        sqlx::query_scalar("SELECT repo_id FROM attached_workspaces WHERE fs_root_canonical = ?")
            .bind(&canonical)
            .fetch_optional(pool)
            .await?;
    if let Some(repo_id) = existing {
        return Ok(BootAttachOutcome {
            repo_id: repo_id.parse().expect("stored repo_id is a valid ULID"),
            canonical,
            created_repo: false,
            created_attachment: false,
        });
    }

    // No attachment yet. Reuse an existing repo whose `local_path`
    // matches the canonical root if one exists (operator may have
    // pre-registered via `add_repo`); otherwise mint a deterministic
    // ULID so re-boot after a wiped attached_workspaces row reuses the
    // same repo identity.
    let existing_repo: Option<String> =
        sqlx::query_scalar("SELECT id FROM repos WHERE local_path = ? LIMIT 1")
            .bind(&canonical)
            .fetch_optional(pool)
            .await?;
    let (repo_id, created_repo) = match existing_repo {
        Some(id) => (
            id.parse::<RepoId>().expect("repos.id is a valid ULID"),
            false,
        ),
        None => {
            let repo_id = deterministic_repo_id(&canonical);
            let name = unique_repo_name(pool, &canonical_path).await?;
            insert_boot_repo(pool, repo_id, &name, &canonical, now).await?;
            (repo_id, true)
        }
    };

    sqlx::query(
        "INSERT INTO attached_workspaces \
         (repo_id, fs_root_canonical, fs_root_display, attached_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(fs_root_canonical) DO NOTHING",
    )
    .bind(repo_id.to_string())
    .bind(&canonical)
    .bind(&display)
    .bind(now.0)
    .execute(pool)
    .await?;

    Ok(BootAttachOutcome {
        repo_id,
        canonical,
        created_repo,
        created_attachment: true,
    })
}

#[derive(Debug, Clone)]
pub struct BootAttachOutcome {
    pub repo_id: RepoId,
    pub canonical: String,
    pub created_repo: bool,
    pub created_attachment: bool,
}

/// Stable ULID derived from the canonical path so re-creating an
/// attachment after a manual `DELETE FROM attached_workspaces` reuses
/// the same repo identity rather than minting a fresh one. ULID layout
/// (48-bit ms timestamp + 80-bit random) is preserved; the "random"
/// half is the leading 80 bits of SHA-256(canonical) and the
/// "timestamp" half is fixed to zero so two boots produce the exact
/// same bytes.
fn deterministic_repo_id(canonical: &str) -> RepoId {
    let digest = Sha256::digest(canonical.as_bytes());
    let mut bytes = [0u8; 16];
    bytes[6..16].copy_from_slice(&digest[0..10]);
    RepoId(Ulid::from_bytes(bytes))
}

/// `repos.name` is `UNIQUE`. Boot defaults to the canonical
/// directory's basename; if a different repo already owns that name
/// (e.g. operator ran `--fs-root /a/foo` after registering a separate
/// `foo` clone) we suffix `-<short-hash>` so the boot upsert never
/// fails on the name collision.
async fn unique_repo_name(pool: &SqlitePool, canonical: &Path) -> sqlx::Result<String> {
    let base = canonical
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "workspace".to_string());
    let taken: Option<String> = sqlx::query_scalar("SELECT id FROM repos WHERE name = ?")
        .bind(&base)
        .fetch_optional(pool)
        .await?;
    if taken.is_none() {
        return Ok(base);
    }
    let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
    Ok(format!("{base}-{}", hex::encode(&hash[0..3])))
}

async fn insert_boot_repo(
    pool: &SqlitePool,
    repo_id: RepoId,
    name: &str,
    canonical: &str,
    now: UnixMillis,
) -> sqlx::Result<()> {
    // No remote on a boot-attached path; use the token variant with an
    // empty env-var as the sentinel "no credentials wired up" value.
    // `add_repo` over RPC will overwrite this row if the operator
    // later registers proper auth.
    let git_auth = serde_json::to_string(&GitAuth::Token {
        env_var: String::new(),
    })
    .expect("GitAuth always serialises");
    sqlx::query(
        "INSERT INTO repos \
         (id, name, clone_url, default_branch, local_path, git_auth, \
          concurrency_cap, default_runner, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?)",
    )
    .bind(repo_id.to_string())
    .bind(name)
    .bind("")
    .bind("main")
    .bind(canonical)
    .bind(&git_auth)
    .bind(now.0)
    .bind(now.0)
    .execute(pool)
    .await?;
    Ok(())
}

/// Test-only helper so the integration tests can assert against the
/// canonical column without re-deriving the canonicalisation logic.
#[doc(hidden)]
pub fn _canonicalise_for_test(raw: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(raw)
}
