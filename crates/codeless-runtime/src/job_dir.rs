//! Job-as-directory layout. A job's authored content lives under
//! `<repo>/.codeless/jobs/<name>/`: `template.yaml` is the spec, every
//! `*.md` file is read into the per-stage prompt. The previous shape
//! was a single `<repo>/.codeless/jobs/<name>.yaml` — both still
//! resolve so legacy jobs keep working until the first edit migrates
//! them. Design: `DOCS/JOB-DIR.md`.
//!
//! No process spawn, no async — this module is read-only resolution
//! plus filename validation. The mutating side (write/delete + the
//! flat→directory migration) lives in `codeless-runtime::rpc`.

use std::fs;
use std::path::{Path, PathBuf};

/// Which on-disk layout a job currently has.
///
/// `FlatPreferred` exists for the rare case where both a flat
/// `<name>.yaml` and a `<name>/` directory exist side by side. The
/// flat file wins because the migration step writes the directory
/// *before* deleting the flat YAML, so seeing both means migration
/// crashed half-way; the safe answer is "act as if migration never
/// started" rather than reading from a directory whose contents may
/// not reflect the user's last save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobLayout {
    None,
    Flat,
    Directory,
    FlatPreferred,
}

impl JobLayout {
    /// What the RPC `list_job_files` `layout` field reports. The
    /// `FlatPreferred` case maps to `flat` because that is the
    /// surface the UI hint already explains ("legacy flat layout —
    /// your next save migrates").
    pub fn wire_name(&self) -> &'static str {
        match self {
            JobLayout::None => "none",
            JobLayout::Flat | JobLayout::FlatPreferred => "flat",
            JobLayout::Directory => "directory",
        }
    }
}

/// Resolve the on-disk state of a job's authored content.
pub fn resolve(repo: &Path, name: &str) -> JobLayout {
    let flat = flat_yaml_path(repo, name);
    let dir = directory_path(repo, name);
    let flat_exists = flat.is_file();
    let dir_exists = dir.is_dir();
    match (flat_exists, dir_exists) {
        (true, true) => JobLayout::FlatPreferred,
        (true, false) => JobLayout::Flat,
        (false, true) => JobLayout::Directory,
        (false, false) => JobLayout::None,
    }
}

/// `<repo>/.codeless/jobs/<name>.yaml` — the legacy single-file path.
pub fn flat_yaml_path(repo: &Path, name: &str) -> PathBuf {
    repo.join(".codeless")
        .join("jobs")
        .join(format!("{name}.yaml"))
}

/// `<repo>/.codeless/jobs/<name>/` — the directory layout root.
pub fn directory_path(repo: &Path, name: &str) -> PathBuf {
    repo.join(".codeless").join("jobs").join(name)
}

/// `<repo>/.codeless/jobs/<name>/template.yaml` — the spec, inside
/// the directory layout.
pub fn template_yaml_path(repo: &Path, name: &str) -> PathBuf {
    directory_path(repo, name).join("template.yaml")
}

/// Return the `.md` files inside `<repo>/.codeless/jobs/<name>/` in
/// filename-ascending order. Non-markdown files (including
/// `template.yaml`) are excluded — this powers the prompt-builder
/// path, where only markdown is folded into the agent's input.
pub fn list_markdown(repo: &Path, name: &str) -> Vec<PathBuf> {
    let dir = directory_path(repo, name);
    let mut out: Vec<PathBuf> = match fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

/// Why a filename was rejected. The RPC layer maps these to
/// `RpcError::invalid_argument` with a human-readable message; tests
/// pattern-match on the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilenameError {
    /// Contained `/`, `\`, or any `..` segment.
    PathTraversal,
    /// Started with `.`. Hidden files are out of scope; the user
    /// wants every file in this directory to be visible in `ls`.
    Dotfile,
    /// `template.yaml` is reserved on `write_job_file` and
    /// `delete_job_file` — the spec has its own RPC that runs the
    /// YAML validator and the rename guard.
    ReservedTemplateYaml,
    /// Empty after trimming.
    Empty,
}

/// Validate and normalise a user-supplied filename for the job-file
/// surface.
///
/// Rules (from `DOCS/JOB-DIR.md` "Filename rules"):
///
/// * Single basename only — `/`, `\`, or any `..` segment is rejected.
/// * No dotfiles. `.env` is rejected.
/// * `template.yaml` is reserved. Callers wanting to edit the spec
///   use `update_job_template`, which carries the YAML validator and
///   the rename guard.
/// * Bare names get `.md` appended (`design` → `design.md`). Files
///   that already end in `.md`, `.yaml`, or `.yml` keep their
///   extension. Any other extension is preserved verbatim so the
///   "drop a `links.txt` next to the spec" case keeps working.
pub fn sanitise_filename(raw: &str) -> Result<String, FilenameError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FilenameError::Empty);
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(FilenameError::PathTraversal);
    }
    if trimmed.split('.').any(|seg| seg == "..") || trimmed == ".." {
        return Err(FilenameError::PathTraversal);
    }
    if trimmed.starts_with('.') {
        return Err(FilenameError::Dotfile);
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower == "template.yaml" {
        return Err(FilenameError::ReservedTemplateYaml);
    }

    let has_known_ext = lower.ends_with(".md")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || trimmed.contains('.');
    if has_known_ext {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}.md"))
    }
}

/// Build the `# Job docs` prompt section for `<name>`. The output is
/// the empty string when the directory layout is not present, so the
/// caller can unconditionally splice it in without checking the
/// layout first.
///
/// Ordering: `SCOPE.md` first under `## Scope`, then `WORKFLOW.md`
/// under `## Workflow`, then every other `*.md` in filename order
/// each under `## <filename>`. The casing match is ASCII
/// case-insensitive so `scope.md` and `Scope.md` both win the special
/// section. Read failures (permission, racing delete) are silently
/// skipped — the prompt should degrade gracefully rather than fail a
/// run because one note file vanished mid-build.
pub fn read_docs_for_prompt(repo: &Path, name: &str) -> String {
    let files = list_markdown(repo, name);
    if files.is_empty() {
        return String::new();
    }

    let mut scope: Option<(PathBuf, String)> = None;
    let mut workflow: Option<(PathBuf, String)> = None;
    let mut extras: Vec<(PathBuf, String)> = Vec::new();

    for path in files {
        let body = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let base = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let lower = base.to_ascii_lowercase();
        if lower == "scope.md" {
            scope = Some((path, body));
        } else if lower == "workflow.md" {
            workflow = Some((path, body));
        } else {
            extras.push((path, body));
        }
    }

    let mut out = String::from("# Job docs\n");
    if let Some((_, body)) = scope {
        out.push_str("\n## Scope\n\n");
        out.push_str(body.trim_end());
        out.push('\n');
    }
    if let Some((_, body)) = workflow {
        out.push_str("\n## Workflow\n\n");
        out.push_str(body.trim_end());
        out.push('\n');
    }
    for (path, body) in extras {
        let base = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
        out.push_str(&format!("\n## {base}\n\n"));
        out.push_str(body.trim_end());
        out.push('\n');
    }
    out
}

/// Build the `# Job docs` prompt section, but in the user-controlled
/// order given by `docs`. Each entry is a basename relative to
/// `<repo>/.codeless/jobs/<name>/`; missing files are skipped without
/// erroring (the user is asserting "include these if present", not
/// "fail if absent"). Read failures are skipped the same way as in
/// the auto-discover path.
///
/// Section headings are case-aware: a SCOPE.md entry renders under
/// `## Scope`, WORKFLOW.md under `## Workflow`, everything else under
/// `## <filename>` so the agent sees the original casing.
pub fn read_docs_ordered(repo: &Path, name: &str, docs: &[String]) -> String {
    let dir = directory_path(repo, name);
    let mut entries: Vec<(String, String)> = Vec::new();
    for raw in docs {
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.contains('/')
            || trimmed.contains('\\')
            || trimmed.starts_with('.')
            || trimmed.split('.').any(|seg| seg == "..")
        {
            continue;
        }
        let path = dir.join(trimmed);
        if let Ok(body) = fs::read_to_string(&path) {
            entries.push((trimmed.to_string(), body));
        }
    }
    if entries.is_empty() {
        return String::new();
    }

    let mut out = String::from("# Job docs\n");
    for (base, body) in entries {
        let lower = base.to_ascii_lowercase();
        let heading = if lower == "scope.md" {
            "Scope".to_string()
        } else if lower == "workflow.md" {
            "Workflow".to_string()
        } else {
            base
        };
        out.push_str(&format!("\n## {heading}\n\n"));
        out.push_str(body.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn resolve_returns_none_when_neither_layout_exists() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(resolve(tmp.path(), "alpha"), JobLayout::None);
    }

    #[test]
    fn resolve_returns_flat_for_legacy_yaml() {
        let tmp = TempDir::new().unwrap();
        touch(&flat_yaml_path(tmp.path(), "alpha"), "name: alpha\n");
        assert_eq!(resolve(tmp.path(), "alpha"), JobLayout::Flat);
    }

    #[test]
    fn resolve_returns_directory_when_dir_exists() {
        let tmp = TempDir::new().unwrap();
        touch(&template_yaml_path(tmp.path(), "alpha"), "name: alpha\n");
        assert_eq!(resolve(tmp.path(), "alpha"), JobLayout::Directory);
    }

    #[test]
    fn resolve_prefers_flat_when_both_layouts_present() {
        let tmp = TempDir::new().unwrap();
        touch(&flat_yaml_path(tmp.path(), "alpha"), "name: alpha\n");
        touch(&template_yaml_path(tmp.path(), "alpha"), "name: alpha\n");
        assert_eq!(resolve(tmp.path(), "alpha"), JobLayout::FlatPreferred);
        assert_eq!(JobLayout::FlatPreferred.wire_name(), "flat");
    }

    #[test]
    fn list_markdown_returns_only_md_files_sorted() {
        let tmp = TempDir::new().unwrap();
        let dir = directory_path(tmp.path(), "alpha");
        touch(&dir.join("template.yaml"), "name: alpha\n");
        touch(&dir.join("zeta.md"), "z");
        touch(&dir.join("alpha.md"), "a");
        touch(&dir.join("links.txt"), "ignored");
        touch(&dir.join("WORKFLOW.md"), "w");

        let found: Vec<String> = list_markdown(tmp.path(), "alpha")
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, vec!["WORKFLOW.md", "alpha.md", "zeta.md"]);
    }

    #[test]
    fn sanitise_rejects_traversal_and_dotfiles_and_reserved() {
        assert_eq!(
            sanitise_filename("../escape.md"),
            Err(FilenameError::PathTraversal),
        );
        assert_eq!(
            sanitise_filename("nested/scope.md"),
            Err(FilenameError::PathTraversal),
        );
        assert_eq!(sanitise_filename(".env"), Err(FilenameError::Dotfile));
        assert_eq!(
            sanitise_filename("template.yaml"),
            Err(FilenameError::ReservedTemplateYaml),
        );
        assert_eq!(
            sanitise_filename("Template.YAML"),
            Err(FilenameError::ReservedTemplateYaml),
        );
        assert_eq!(sanitise_filename("   "), Err(FilenameError::Empty));
    }

    #[test]
    fn read_docs_ordered_honours_explicit_order_and_skips_missing() {
        let tmp = TempDir::new().unwrap();
        let dir = directory_path(tmp.path(), "alpha");
        touch(&dir.join("SCOPE.md"), "scope-body");
        touch(&dir.join("design.md"), "design-body");
        touch(&dir.join("WORKFLOW.md"), "workflow-body");
        // Order is design → workflow → scope → ghost (missing).
        let docs = vec![
            "design.md".to_string(),
            "WORKFLOW.md".to_string(),
            "SCOPE.md".to_string(),
            "ghost.md".to_string(),
        ];
        let out = read_docs_ordered(tmp.path(), "alpha", &docs);
        let design_at = out.find("design-body").expect("design present");
        let workflow_at = out.find("workflow-body").expect("workflow present");
        let scope_at = out.find("scope-body").expect("scope present");
        assert!(
            design_at < workflow_at && workflow_at < scope_at,
            "order wrong: {out}"
        );
        assert!(!out.contains("ghost.md"), "missing file leaked into output");
        // SCOPE.md still renders under `## Scope`, casing-aware.
        assert!(out.contains("## Scope"), "scope heading missing");
        assert!(out.contains("## Workflow"), "workflow heading missing");
        assert!(out.contains("## design.md"), "design heading missing");
    }

    #[test]
    fn read_docs_ordered_rejects_traversal_and_dotfiles() {
        let tmp = TempDir::new().unwrap();
        let dir = directory_path(tmp.path(), "alpha");
        touch(&dir.join("SCOPE.md"), "ok");
        let docs = vec![
            "../escape.md".to_string(),
            ".env".to_string(),
            "nested/path.md".to_string(),
            "SCOPE.md".to_string(),
        ];
        let out = read_docs_ordered(tmp.path(), "alpha", &docs);
        assert!(out.contains("## Scope"));
        assert!(!out.contains("escape"));
        assert!(!out.contains(".env"));
        assert!(!out.contains("nested"));
    }

    #[test]
    fn sanitise_appends_md_for_bare_names_and_keeps_known_extensions() {
        assert_eq!(sanitise_filename("design").unwrap(), "design.md");
        assert_eq!(sanitise_filename("SCOPE.md").unwrap(), "SCOPE.md");
        assert_eq!(sanitise_filename("notes.yaml").unwrap(), "notes.yaml");
        assert_eq!(sanitise_filename("notes.yml").unwrap(), "notes.yml");
        assert_eq!(sanitise_filename("links.txt").unwrap(), "links.txt");
    }
}
