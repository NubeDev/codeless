use sqlx::migrate::Migrator;

/// Compile-time embedded migrator. The `migrations/` directory next to
/// `Cargo.toml` is the source of truth; sqlx reads SQL files at build
/// time and bakes them into the binary so an installed `codeless`
/// doesn't need its checkout to apply migrations.
///
/// Forward-only by design (SCOPE.md Appendix A: "No down-migrations").
/// New schema work appends a numbered SQL file rather than editing
/// existing ones — migrations are content-hashed by sqlx and a mismatch
/// against the recorded hash refuses to start, which is the protection
/// we want against retroactive edits.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
