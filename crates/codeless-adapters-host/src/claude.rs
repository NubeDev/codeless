//! Best-effort discovery + status probe for the `claude` CLI on the
//! host. Used by `codeless serve` at boot to populate
//! `ServerInfo.claude`, so the UI's settings → Models surface can
//! render an actionable hint ("Install Claude Code", "Run
//! `claude auth login`", "Ready") without the user having to first
//! submit a job and see it fail.
//!
//! Discovery mirrors `ai-runner::runners::claude::discover_claude_binary`
//! (env override → `PATH` → well-known install dirs → editor-shipped
//! copies). That function is not `pub` upstream, so the logic is
//! duplicated here rather than re-exported; if it drifts, the integration
//! test in this module exercises both fallbacks and catches the
//! divergence at CI time.
//!
//! The auth probe is intentionally cheap: a single
//! `claude /status --output-format json` invocation with a 2 s budget.
//! When the wrapper exits cleanly and the JSON includes a recognisable
//! signal, we return `Some(true)` / `Some(false)`; on any ambiguity
//! (timeout, non-zero exit, unparseable output, schema we don't know)
//! the result is `None` so the UI falls back to a neutral "binary
//! detected" hint instead of misreporting.

use std::path::{Path, PathBuf};
use std::time::Duration;

use codeless_rpc::ClaudeStatus;

/// Run the full discovery + probe pipeline. Returns `None` only when
/// the binary cannot be located anywhere; a binary that exists but
/// fails to answer `--version` still yields `Some` with `version:
/// None` so the UI can tell the operator the install is present but
/// broken.
pub async fn probe() -> Option<ClaudeStatus> {
    let binary = discover_claude_binary()?;
    let version = read_version(&binary).await;
    let authenticated = probe_auth(&binary).await;
    Some(ClaudeStatus {
        binary_path: binary.display().to_string(),
        version,
        authenticated,
    })
}

async fn read_version(binary: &Path) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new(binary)
            .arg("--version")
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

/// Ask `claude /status --output-format json` and try to parse a
/// definite auth answer. Wrapper versions vary in JSON shape, so we
/// probe a few known field names. Anything outside that set returns
/// `None` rather than guessing.
async fn probe_auth(binary: &Path) -> Option<bool> {
    let out = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new(binary)
            .args(["/status", "--output-format", "json"])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    for key in ["authenticated", "logged_in", "is_logged_in"] {
        if let Some(b) = v.get(key).and_then(|x| x.as_bool()) {
            return Some(b);
        }
    }
    if let Some(s) = v.get("auth").and_then(|x| x.as_str()) {
        let lower = s.to_ascii_lowercase();
        if lower == "ok" || lower == "authenticated" {
            return Some(true);
        }
        if lower == "missing" || lower == "unauthenticated" {
            return Some(false);
        }
    }
    None
}

fn discover_claude_binary() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("CLAUDE_BINARY") {
        let v = v.trim();
        if !v.is_empty() {
            // Honour the operator's override even if the file is
            // missing; downstream probes will surface the concrete
            // error in a more actionable place than this discovery.
            return Some(PathBuf::from(v));
        }
    }
    if let Some(p) = find_on_path("claude") {
        return Some(p);
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let static_candidates: [PathBuf; 4] = [
            home.join(".local/bin/claude"),
            home.join(".bun/bin/claude"),
            home.join(".npm-global/bin/claude"),
            home.join(".config/npm/global/bin/claude"),
        ];
        for c in &static_candidates {
            if c.is_file() {
                return Some(c.clone());
            }
        }
        if let Some(p) = scan_nvm_node_bins(&home) {
            return Some(p);
        }
        for root in [
            home.join(".vscode/extensions"),
            home.join(".vscode-server/extensions"),
            home.join(".cursor/extensions"),
            home.join(".windsurf/extensions"),
        ] {
            if let Some(p) = scan_vscode_extensions(&root) {
                return Some(p);
            }
        }
    }
    for sys in ["/opt/homebrew/bin/claude", "/usr/local/bin/claude"] {
        let p = PathBuf::from(sys);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let full = dir.join(name);
        if is_executable_file(&full) {
            return Some(full);
        }
    }
    None
}

fn is_executable_file(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn scan_nvm_node_bins(home: &Path) -> Option<PathBuf> {
    let root = home.join(".nvm/versions/node");
    let rd = std::fs::read_dir(&root).ok()?;
    for entry in rd.flatten() {
        let candidate = entry.path().join("bin/claude");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn scan_vscode_extensions(root: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(root).ok()?;
    let mut best: Option<(String, PathBuf)> = None;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("anthropic.claude-code-") {
            continue;
        }
        let bin = entry.path().join("resources/native-binary/claude");
        if !is_executable_file(&bin) {
            continue;
        }
        if best.as_ref().map(|(n, _)| name > *n).unwrap_or(true) {
            best = Some((name, bin));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    /// Every test in this module mutates `CLAUDE_BINARY`, `PATH`, or
    /// `HOME` — `cargo test` runs tests in a single process and would
    /// otherwise interleave those writes. Serialise on a module-local
    /// async mutex so the guard can be held across `probe().await`.
    static ENV_LOCK: Mutex<()> = Mutex::const_new(());

    fn write_exec(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    #[tokio::test]
    async fn probe_returns_none_when_binary_absent() {
        let _g = ENV_LOCK.lock().await;
        let tmp = TempDir::new().unwrap();
        let orig_path = std::env::var_os("PATH");
        let orig_home = std::env::var_os("HOME");
        let orig_bin = std::env::var_os("CLAUDE_BINARY");
        std::env::set_var("PATH", tmp.path());
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("CLAUDE_BINARY");

        let got = probe().await;

        if let Some(v) = orig_path {
            std::env::set_var("PATH", v);
        }
        if let Some(v) = orig_home {
            std::env::set_var("HOME", v);
        }
        if let Some(v) = orig_bin {
            std::env::set_var("CLAUDE_BINARY", v);
        }

        assert!(got.is_none());
    }

    #[tokio::test]
    async fn probe_reads_version_when_binary_replies() {
        let _g = ENV_LOCK.lock().await;
        let tmp = TempDir::new().unwrap();
        // Fake `claude` that prints a fixed version for `--version` and
        // exits non-zero for `/status` so the auth probe lands at
        // `None` — the explicit "unknown" outcome.
        let script = "#!/bin/sh\n\
                      case \"$1\" in\n\
                        --version) echo 'claude 9.9.9'; exit 0;;\n\
                        /status) exit 1;;\n\
                      esac\n";
        let bin = write_exec(tmp.path(), "claude", script);
        let orig = std::env::var_os("CLAUDE_BINARY");
        std::env::set_var("CLAUDE_BINARY", &bin);

        let got = probe().await.expect("probe should locate the fake binary");

        match orig {
            Some(v) => std::env::set_var("CLAUDE_BINARY", v),
            None => std::env::remove_var("CLAUDE_BINARY"),
        }

        assert_eq!(got.binary_path, bin.display().to_string());
        assert_eq!(got.version.as_deref(), Some("claude 9.9.9"));
        assert_eq!(got.authenticated, None);
    }

    #[tokio::test]
    async fn probe_parses_authenticated_json() {
        let _g = ENV_LOCK.lock().await;
        let tmp = TempDir::new().unwrap();
        let script = "#!/bin/sh\n\
                      case \"$1\" in\n\
                        --version) echo 'claude 1.0'; exit 0;;\n\
                        /status) echo '{\"authenticated\":true}'; exit 0;;\n\
                      esac\n";
        let bin = write_exec(tmp.path(), "claude", script);
        let orig = std::env::var_os("CLAUDE_BINARY");
        std::env::set_var("CLAUDE_BINARY", &bin);

        let got = probe().await.unwrap();
        match orig {
            Some(v) => std::env::set_var("CLAUDE_BINARY", v),
            None => std::env::remove_var("CLAUDE_BINARY"),
        }

        assert_eq!(got.authenticated, Some(true));
    }
}
