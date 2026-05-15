//! R2 — UI tree may only reach `/rpc/` via `HttpSseClient`.
//!
//! Every other shell, hook, or component goes through the typed
//! `RpcClient` boundary at `ui/codeless-ui/src/lib/rpc/`. A direct
//! `fetch('/rpc/…')` call from outside the SSE client bypasses the
//! typed surface, the auth layer, and the error mapping — exactly the
//! decoupling the boundary exists to enforce.
//!
//! The probe flags any line in a `ui/codeless-ui/` TS/JS file that
//! contains both a `fetch(` call and the `/rpc/` substring. The only
//! allowed file is `ui/codeless-ui/src/lib/rpc/http-sse-client.ts`.

use crate::{norm_path, ChangedFile, Violation};

const PROBE: &str = "no-direct-rpc-fetch-outside-sse-client";
const UI_PREFIX: &str = "ui/codeless-ui/";
const ALLOWED_FILE: &str = "ui/codeless-ui/src/lib/rpc/http-sse-client.ts";
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
        if path == ALLOWED_FILE {
            continue;
        }
        if !TS_EXTS.iter().any(|ext| path.ends_with(ext)) {
            continue;
        }
        for (idx, line) in file.content.lines().enumerate() {
            if line.contains("fetch(") && line.contains("/rpc/") {
                out.push(Violation {
                    probe: PROBE,
                    path: file.path.clone(),
                    line: Some(idx + 1),
                    message: "direct fetch() to /rpc/ must go through HttpSseClient".to_string(),
                });
            }
        }
    }
    out
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
    fn flags_fetch_to_rpc_outside_sse_client() {
        let files = vec![file(
            "ui/codeless-ui/src/components/widget.tsx",
            "const r = await fetch('/rpc/jobs.list');\n",
        )];
        let v = run(&files);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].line, Some(1));
    }

    #[test]
    fn allows_fetch_inside_sse_client() {
        let files = vec![file(
            "ui/codeless-ui/src/lib/rpc/http-sse-client.ts",
            "const res = await fetch(url + '/rpc/' + method);\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn ignores_unrelated_fetch_calls() {
        let files = vec![file(
            "ui/codeless-ui/src/components/avatar.tsx",
            "const img = await fetch('/static/avatar.png');\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn ignores_rpc_mentions_without_fetch() {
        let files = vec![file(
            "ui/codeless-ui/src/types.ts",
            "const PATH = '/rpc/jobs.list';\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn ignores_files_outside_ui_tree() {
        let files = vec![file(
            "crates/codeless-server/src/handler.rs",
            "fetch('/rpc/x')\n",
        )];
        assert!(run(&files).is_empty());
    }

    #[test]
    fn skips_self_crate() {
        let files = vec![file(
            "crates/codeless-predicates/src/probes/direct_fetch.rs",
            "// flags fetch( near /rpc/\n",
        )];
        assert!(run(&files).is_empty());
    }
}
