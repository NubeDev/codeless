//! R2 — UI tree may only reach `@tauri-apps/*` inside the desktop shell.
//!
//! The workspace UI architecture promises that the browser, iOS, and
//! Android shells are runtime-pure JS: anything Tauri-specific lives
//! behind a shell-injected capability adapter exposed only to the
//! desktop shell. The `RpcClient` boundary at `src/lib/rpc/` and the
//! capability adapters at `src/lib/shell/` exist to keep that promise.
//!
//! This probe enforces the boundary: a TypeScript/JS source under
//! `ui/codeless-ui/` that imports anything from a `@tauri-apps/*`
//! package is a violation unless the file lives under
//! `ui/codeless-ui/src/shells/desktop/`.

use crate::{norm_path, ChangedFile, Violation};

const PROBE: &str = "no-tauri-imports-outside-desktop-shell";
const UI_PREFIX: &str = "ui/codeless-ui/";
const ALLOWED_PREFIX: &str = "ui/codeless-ui/src/shells/desktop/";
const SELF_PREFIX: &str = "crates/codeless-predicates/";

const TS_EXTS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

pub fn run(files: &[ChangedFile]) -> Vec<Violation> {
    let mut out = Vec::new();
    for file in files {
        let path = norm_path(&file.path);
        if path.starts_with(SELF_PREFIX) {
            continue;
        }
        if !path.starts_with(UI_PREFIX) {
            continue;
        }
        if path.starts_with(ALLOWED_PREFIX) {
            continue;
        }
        if !TS_EXTS.iter().any(|ext| path.ends_with(ext)) {
            continue;
        }
        for (idx, line) in file.content.lines().enumerate() {
            if line_imports_tauri(line) {
                out.push(Violation {
                    probe: PROBE,
                    path: file.path.clone(),
                    line: Some(idx + 1),
                    message:
                        "@tauri-apps/* imports must live under ui/codeless-ui/src/shells/desktop/"
                            .to_string(),
                });
            }
        }
    }
    out
}

/// `import … from '@tauri-apps/x'`, `from "@tauri-apps/x"`, and the
/// dynamic `import('@tauri-apps/x')` form all qualify. A `require()`
/// in TypeScript is rare enough that we hold the check to the two
/// import shapes plus dynamic import — the rule's surface is import
/// statements, not arbitrary string mentions of the package name.
fn line_imports_tauri(line: &str) -> bool {
    let needles = [
        "from '@tauri-apps/",
        "from \"@tauri-apps/",
        "import('@tauri-apps/",
        "import(\"@tauri-apps/",
    ];
    needles.iter().any(|n| line.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(path: &str, content: &str) -> ChangedFile {
        ChangedFile {
            path: PathBuf::from(path),
            content: content.to_string(),
        }
    }

    #[test]
    fn flags_tauri_import_outside_desktop_shell() {
        let files = vec![file(
            "ui/codeless-ui/src/components/foo.tsx",
            "import { open } from '@tauri-apps/api/dialog';\n",
        )];
        let v = run(&files);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].line, Some(1));
    }

    #[test]
    fn allows_tauri_import_inside_desktop_shell() {
        let files = vec![file(
            "ui/codeless-ui/src/shells/desktop/main.ts",
            "import { invoke } from '@tauri-apps/api/core';\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn allows_double_quoted_form_inside_desktop_shell() {
        let files = vec![file(
            "ui/codeless-ui/src/shells/desktop/x.ts",
            "import { foo } from \"@tauri-apps/plugin-fs\";\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn flags_dynamic_import_outside_desktop_shell() {
        let files = vec![file(
            "ui/codeless-ui/src/components/lazy.ts",
            "const m = await import('@tauri-apps/api/path');\n",
        )];
        assert_eq!(run(&files).len(), 1);
    }

    #[test]
    fn ignores_files_outside_ui_tree() {
        let files = vec![file(
            "crates/codeless-server/notes.txt",
            "from '@tauri-apps/api'\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_self_crate() {
        let files = vec![file(
            "crates/codeless-predicates/src/probes/tauri_imports.rs",
            "// matches from '@tauri-apps/'\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn does_not_flag_mere_string_mentions() {
        let files = vec![file(
            "ui/codeless-ui/src/components/foo.tsx",
            "const docs = 'see @tauri-apps/api docs';\n",
        )];
        assert!(run(&files).is_empty());
    }
}
