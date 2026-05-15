//! Spawn the operator's `$VISUAL` or `$EDITOR` on a file. The Step 6
//! `codeless patches edit` command in the CLI uses this to let the
//! human refine a proposed patch before approving.
//!
//! Why this lives here and not in the CLI: R1 in `codeless/CLAUDE.md`
//! pins `std::process` / `tokio::process` to this crate. An editor
//! launch is a process spawn like any other; the abstraction is the
//! same as `commit_paths` next door — the caller passes a target
//! path, the adapter handles the OS-level handoff.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitStatus};

use thiserror::Error;

/// What can go wrong launching the editor.
#[derive(Debug, Error)]
pub enum EditorError {
    #[error("neither $VISUAL nor $EDITOR is set; cannot launch an editor")]
    NoEditor,
    #[error("editor command is empty after splitting `{cmd}`")]
    EmptyCommand { cmd: String },
    #[error("spawn editor `{cmd}`: {source}")]
    Spawn {
        cmd: String,
        #[source]
        source: std::io::Error,
    },
}

/// Resolve the operator's preferred editor. `$VISUAL` wins over
/// `$EDITOR`, matching the long-standing convention git itself uses.
/// Returns `None` when neither is set or both are empty.
pub fn pick_editor() -> Option<String> {
    std::env::var("VISUAL")
        .ok()
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|s| !s.trim().is_empty())
}

/// Launch `cmd` (a shell-style invocation such as `vim` or `code
/// --wait`) on `path` and wait for it to exit. Returns the editor's
/// `ExitStatus`; the caller decides whether a non-zero status is an
/// error (for `codeless patches edit` it is, since the user asked to
/// abandon the edit).
pub fn invoke_editor(cmd: &str, path: &Path) -> Result<ExitStatus, EditorError> {
    let mut parts = shell_split(cmd);
    if parts.is_empty() {
        return Err(EditorError::EmptyCommand {
            cmd: cmd.to_string(),
        });
    }
    let program = parts.remove(0);
    let args: Vec<OsString> = parts.into_iter().map(OsString::from).collect();
    Command::new(program)
        .args(args)
        .arg(path)
        .status()
        .map_err(|source| EditorError::Spawn {
            cmd: cmd.to_string(),
            source,
        })
}

/// Lightweight whitespace split honouring single-quoted spans. The
/// CLI's `$EDITOR` is typically `vim` or `code --wait`; supporting
/// the full shell grammar (escapes, $vars, backticks) would be more
/// complexity than the interface warrants and would surprise operators
/// expecting their literal `$EDITOR` to round-trip.
fn shell_split(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    for c in cmd.chars() {
        match c {
            '\'' => in_quote = !in_quote,
            ' ' | '\t' if !in_quote => {
                if !buf.is_empty() {
                    out.push(std::mem::take(&mut buf));
                }
            }
            _ => buf.push(c),
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_split_handles_quotes() {
        assert_eq!(shell_split("vim"), vec!["vim"]);
        assert_eq!(shell_split("nano -w"), vec!["nano", "-w"]);
        assert_eq!(shell_split("code --wait"), vec!["code", "--wait"]);
        assert_eq!(
            shell_split("env 'A=1 B=2' editor"),
            vec!["env", "A=1 B=2", "editor"]
        );
        assert!(shell_split("").is_empty());
    }
}
