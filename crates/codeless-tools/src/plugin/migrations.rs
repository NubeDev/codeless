//! Static check on a plugin's migration SQL.
//!
//! The substrate doc (item 6, OQ-PS-4) decides: every plugin-owned
//! SQLite table is named `<plugin_id>_<table>`; the manifest reader
//! rejects any `CREATE TABLE` / `ALTER TABLE` / `DROP TABLE` whose
//! target name does not start with `<plugin_id>_`. SQLite has no
//! row-level constraint that would enforce the same rule at runtime,
//! so this is the only line of defence against a plugin migration
//! squatting on a codeless-owned table.
//!
//! Scope of the parser:
//!
//! - It is **not** a SQL parser. It tokenises just enough to find
//!   `CREATE TABLE [IF NOT EXISTS] <name>`, `ALTER TABLE <name>`,
//!   `DROP TABLE [IF EXISTS] <name>`, and the same triplet for
//!   `CREATE/DROP INDEX/TRIGGER/VIEW`. Other DDL (e.g. `PRAGMA`) is
//!   allowed without restriction.
//! - Statements are split on `;` outside of single-/double-quoted
//!   strings. Comments (`--…` to EOL, `/* … */`) are stripped first.
//! - Object names may be unquoted (`notes_entries`), double-quoted
//!   (`"notes_entries"`), or backticked (`` `notes_entries` ``). All
//!   three normalise to the same value for the prefix check.
//!
//! Anything we cannot classify is left alone — the conservative call
//! here is to fail-closed on `CREATE TABLE` style statements rather
//! than to refuse statements we cannot recognise (some plugins will
//! genuinely need raw `INSERT`s in their seed migration).

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum MigrationCheckError {
    #[error("read migration {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "migration {file}: statement {kind} targets `{target}` which is not in the \
         `{plugin}_` namespace; every plugin-owned object must be prefixed \
         (DOCS/PLUGIN-SUBSTRATE.md item 6, OQ-PS-4)"
    )]
    BadPrefix {
        file: PathBuf,
        plugin: String,
        kind: &'static str,
        target: String,
    },
}

/// One migration file as parsed off disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMigration {
    pub path: PathBuf,
    pub sql: String,
}

/// Scan a plugin's migration directory and return the migrations in
/// stable `<lexicographic filename>` order. The host runtime applies
/// them in this order; codeless-tools does not run them itself
/// (sqlx + the SqliteStore live in `codeless-runtime`). This split
/// keeps `codeless-tools` host-only-but-not-runtime-coupled.
pub fn load_migrations_dir(
    plugin_id: &str,
    dir: &Path,
) -> Result<Vec<PluginMigration>, MigrationCheckError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|source| MigrationCheckError::Read {
            path: dir.to_path_buf(),
            source,
        })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    entries.sort();

    let mut out = Vec::with_capacity(entries.len());
    for path in entries {
        let sql = std::fs::read_to_string(&path).map_err(|source| MigrationCheckError::Read {
            path: path.clone(),
            source,
        })?;
        check_sql(plugin_id, &path, &sql)?;
        out.push(PluginMigration { path, sql });
    }
    Ok(out)
}

/// Parse `sql` looking for `CREATE TABLE` / `ALTER TABLE` / `DROP
/// TABLE` (and the matching INDEX/TRIGGER/VIEW forms) and reject any
/// whose target name lacks the `<plugin_id>_` prefix.
///
/// `file` is plumbed only so error messages point the operator at the
/// offending file; the function works equally well on an in-memory
/// string.
pub fn check_sql(plugin_id: &str, file: &Path, sql: &str) -> Result<(), MigrationCheckError> {
    let cleaned = strip_comments(sql);
    let prefix = format!("{plugin_id}_");
    for stmt in split_statements(&cleaned) {
        if let Some((kind, target)) = find_namespaced_target(&stmt) {
            if !target.starts_with(&prefix) {
                return Err(MigrationCheckError::BadPrefix {
                    file: file.to_path_buf(),
                    plugin: plugin_id.to_string(),
                    kind,
                    target,
                });
            }
        }
    }
    Ok(())
}

/// Strip `--` to-EOL comments and `/* ... */` block comments. Done up
/// front so the statement scanner does not have to mode-switch.
fn strip_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied();
        if c == b'-' && next == Some(b'-') {
            // Skip to end of line.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && next == Some(b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            // Skip the closing */ (or run off the end).
            i = (i + 2).min(bytes.len());
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            // Preserve quoted strings/identifiers verbatim so they are
            // not eaten by the comment stripper or split on `;`.
            out.push(c as char);
            i += 1;
            while i < bytes.len() {
                let q = bytes[i];
                out.push(q as char);
                i += 1;
                if q == c {
                    // Handle SQL `''` / `""` doubling: a duplicate of
                    // the quote char is an escape, not a terminator.
                    if bytes.get(i).copied() == Some(c) {
                        out.push(c as char);
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Split SQL into statements at top-level `;`. Quoted regions
/// (already preserved by `strip_comments`) keep their semicolons.
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\'' || c == b'"' || c == b'`' {
            cur.push(c as char);
            i += 1;
            while i < bytes.len() {
                let q = bytes[i];
                cur.push(q as char);
                i += 1;
                if q == c {
                    if bytes.get(i).copied() == Some(c) {
                        cur.push(c as char);
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
            continue;
        }
        if c == b';' {
            if !cur.trim().is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            i += 1;
            continue;
        }
        cur.push(c as char);
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// If `stmt` is one of the namespaced DDL forms, return
/// `(kind, target_name)`. `target_name` is unquoted.
fn find_namespaced_target(stmt: &str) -> Option<(&'static str, String)> {
    let tokens = tokenise_keywords(stmt);
    // Walk the leading keyword sequence; positional matching is enough
    // because SQLite's grammar for these statements is fixed.
    let mut iter = tokens.iter().peekable();
    let first = iter.next()?;
    let second = iter.next()?;
    let kind = match (first.kw.as_str(), second.kw.as_str()) {
        ("create", "table") => "CREATE TABLE",
        ("alter", "table") => "ALTER TABLE",
        ("drop", "table") => "DROP TABLE",
        ("create", "index") | ("create", "unique") => "CREATE INDEX",
        ("drop", "index") => "DROP INDEX",
        ("create", "trigger") => "CREATE TRIGGER",
        ("drop", "trigger") => "DROP TRIGGER",
        ("create", "view") => "CREATE VIEW",
        ("drop", "view") => "DROP VIEW",
        ("create", "virtual") => "CREATE TABLE",
        _ => return None,
    };
    // Skip optional `UNIQUE`/`VIRTUAL` second token, plus `INDEX`/`TABLE`
    // separator after `CREATE UNIQUE INDEX` / `CREATE VIRTUAL TABLE`.
    let mut consumed_extra = false;
    if second.kw == "unique" || second.kw == "virtual" {
        iter.next()?;
        consumed_extra = true;
    }
    let _ = consumed_extra; // explicit to mark intent
                            // Skip optional `IF NOT EXISTS` / `IF EXISTS`.
    if iter.peek().map(|t| t.kw.as_str()) == Some("if") {
        iter.next();
        match iter
            .next()
            .map(|t| t.kw.clone())
            .unwrap_or_default()
            .as_str()
        {
            "not" => {
                if iter.next().map(|t| t.kw.as_str()) != Some("exists") {
                    return None;
                }
            }
            "exists" => {}
            _ => return None,
        }
    }
    let name_tok = iter.next()?;
    Some((kind, unquote_name(&name_tok.raw)))
}

#[derive(Debug, Clone)]
struct Token {
    /// Lowercase keyword for matching (`create`, `table`, …) or
    /// lowercase identifier text. Quotes/backticks stripped.
    kw: String,
    /// Original token text with surrounding quotes if present.
    raw: String,
}

fn tokenise_keywords(stmt: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let bytes = stmt.as_bytes();
    let mut i = 0;
    // Take just enough tokens to identify the target — eight is more
    // than the longest legitimate header (`CREATE UNIQUE INDEX IF NOT
    // EXISTS notes_entries_idx ON …`).
    while i < bytes.len() && out.len() < 8 {
        let c = bytes[i];
        if c.is_ascii_whitespace() || c == b'(' {
            i += 1;
            continue;
        }
        if c == b'"' || c == b'`' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != c {
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            let raw = stmt[start..i].to_string();
            let kw = raw
                .trim_matches(|ch| ch == '"' || ch == '`')
                .to_ascii_lowercase();
            out.push(Token { kw, raw });
            continue;
        }
        // Read bare word: identifier characters incl. dot for
        // schema-qualified names (sqlite allows `main.tbl`).
        let start = i;
        while i < bytes.len() {
            let b = bytes[i];
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' {
                i += 1;
            } else {
                break;
            }
        }
        if i == start {
            // Non-identifier punctuation we don't care about (`,`, `=`,
            // `;`-already-split, etc.). Advance and continue.
            i += 1;
            continue;
        }
        let raw = stmt[start..i].to_string();
        let kw = raw.to_ascii_lowercase();
        out.push(Token { kw, raw });
    }
    out
}

fn unquote_name(raw: &str) -> String {
    let trimmed = raw.trim_matches(|c| c == '"' || c == '`');
    // Schema-qualified names (`main.tbl`) — keep only the rightmost
    // segment; the schema prefix doesn't change the namespace check.
    match trimmed.rsplit_once('.') {
        Some((_, name)) => name.to_string(),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> PathBuf {
        PathBuf::from("test.sql")
    }

    #[test]
    fn accepts_prefixed_create_table() {
        check_sql(
            "notes",
            &p(),
            "CREATE TABLE notes_entries (id TEXT PRIMARY KEY);",
        )
        .unwrap();
    }

    #[test]
    fn rejects_unprefixed_create_table() {
        let err =
            check_sql("notes", &p(), "CREATE TABLE entries (id TEXT PRIMARY KEY);").unwrap_err();
        let MigrationCheckError::BadPrefix { kind, target, .. } = err else {
            panic!("wrong error: {err:?}");
        };
        assert_eq!(kind, "CREATE TABLE");
        assert_eq!(target, "entries");
    }

    #[test]
    fn rejects_alter_codeless_table() {
        let err = check_sql("notes", &p(), "ALTER TABLE personas ADD COLUMN x INT;").unwrap_err();
        assert!(matches!(err, MigrationCheckError::BadPrefix { .. }));
    }

    #[test]
    fn rejects_drop_codeless_table() {
        let err = check_sql("notes", &p(), "DROP TABLE assistant_threads;").unwrap_err();
        assert!(matches!(err, MigrationCheckError::BadPrefix { .. }));
    }

    #[test]
    fn drop_if_exists_unquoted() {
        let err = check_sql("notes", &p(), "DROP TABLE IF EXISTS personas;").unwrap_err();
        assert!(matches!(err, MigrationCheckError::BadPrefix { .. }));
        check_sql("notes", &p(), "DROP TABLE IF EXISTS notes_entries;").unwrap();
    }

    #[test]
    fn ignores_inserts_and_pragmas() {
        check_sql(
            "notes",
            &p(),
            "PRAGMA foreign_keys = ON;\n\
             INSERT INTO personas (id) VALUES ('builtin:notes');",
        )
        .unwrap();
    }

    #[test]
    fn handles_quoted_target() {
        check_sql("notes", &p(), "CREATE TABLE \"notes_entries\" (id TEXT);").unwrap();
        check_sql("notes", &p(), "CREATE TABLE `notes_entries` (id TEXT);").unwrap();
        let err = check_sql("notes", &p(), "CREATE TABLE \"personas\" (x TEXT);").unwrap_err();
        assert!(matches!(err, MigrationCheckError::BadPrefix { .. }));
    }

    #[test]
    fn handles_create_if_not_exists() {
        check_sql(
            "notes",
            &p(),
            "CREATE TABLE IF NOT EXISTS notes_entries (id TEXT);",
        )
        .unwrap();
        let err = check_sql(
            "notes",
            &p(),
            "CREATE TABLE IF NOT EXISTS personas (id TEXT);",
        )
        .unwrap_err();
        assert!(matches!(err, MigrationCheckError::BadPrefix { .. }));
    }

    #[test]
    fn handles_create_unique_index() {
        check_sql(
            "notes",
            &p(),
            "CREATE UNIQUE INDEX notes_idx ON notes_entries (id);",
        )
        .unwrap();
        let err = check_sql(
            "notes",
            &p(),
            "CREATE UNIQUE INDEX repos_idx ON repos (id);",
        )
        .unwrap_err();
        assert!(matches!(err, MigrationCheckError::BadPrefix { .. }));
    }

    #[test]
    fn strips_line_comments_before_scanning() {
        check_sql(
            "notes",
            &p(),
            "-- CREATE TABLE personas (forbidden in comment);\n\
             CREATE TABLE notes_entries (id TEXT);",
        )
        .unwrap();
    }

    #[test]
    fn strips_block_comments_before_scanning() {
        check_sql(
            "notes",
            &p(),
            "/* CREATE TABLE personas; */ CREATE TABLE notes_entries (id TEXT);",
        )
        .unwrap();
    }

    #[test]
    fn semicolon_inside_string_does_not_split() {
        check_sql(
            "notes",
            &p(),
            "INSERT INTO notes_entries (body) VALUES ('one; two;'); \
             CREATE TABLE notes_b (id TEXT);",
        )
        .unwrap();
    }

    #[test]
    fn schema_qualified_target() {
        let err = check_sql("notes", &p(), "CREATE TABLE main.personas (x INT);").unwrap_err();
        assert!(matches!(err, MigrationCheckError::BadPrefix { .. }));
    }
}
