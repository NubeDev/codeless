//! Spawn a shell command, capture its terminal output, time it.
//! The runtime's verify-gate uses this through a thin trait wrapper
//! (`verify_runner::HostVerifyExec`) — process spawn lives here per
//! the workspace's R1 cross-platform rule, so the runtime crate
//! cannot grow a `Command` of its own.
//!
//! Output policy: stdout and stderr are merged and only the tail is
//! kept (~16 lines, matching the verify-step wire convention). A
//! verify gate that prints megabytes of output before failing should
//! not be able to OOM the runtime; the full transcript belongs in a
//! log file the user can `tail -f`, not in the SSE event payload.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// Lines of merged stdout+stderr retained from the tail of a shell
/// run. Matches the `verify-step-failed.tail` wire convention so the
/// UI's per-gate row renders the same shape across both the runtime
/// emit and a future log-file reader.
const TAIL_LINES: usize = 16;

/// Terminal outcome of one shell invocation. `duration_ms` is
/// wall-clock from spawn to exit; `tail` is the last `TAIL_LINES`
/// lines of merged stdout+stderr regardless of success — the verify
/// gate's UI surfaces it on failure but a future debug view may
/// want the passing tail too.
#[derive(Debug, Clone)]
pub struct ShellOutcome {
    pub exit_code: i32,
    pub duration_ms: u64,
    pub tail: String,
}

/// Run `command` through `sh -c` with `cwd` as the working directory
/// when supplied. A spawn-time failure (`sh` missing, permission
/// denied) collapses to `exit_code = -1` with the OS error in the
/// tail so the caller gets a uniformly-shaped outcome it can emit
/// without an extra branch — the verify gate already treats a
/// non-zero exit as "failed", and a missing `sh` is a runtime
/// failure the operator needs to see, not a programming error to
/// unwrap on.
pub fn run_shell(cwd: Option<&Path>, command: &str) -> ShellOutcome {
    let start = Instant::now();
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    match cmd.output() {
        Ok(out) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.stderr.is_empty() {
                if !merged.is_empty() && !merged.ends_with('\n') {
                    merged.push('\n');
                }
                merged.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            ShellOutcome {
                exit_code: out.status.code().unwrap_or(-1),
                duration_ms,
                tail: tail_lines(&merged, TAIL_LINES),
            }
        }
        Err(err) => ShellOutcome {
            exit_code: -1,
            duration_ms: start.elapsed().as_millis() as u64,
            tail: format!("sh spawn failed: {err}"),
        },
    }
}

fn tail_lines(s: &str, n: usize) -> String {
    let trimmed = s.trim_end_matches('\n');
    if trimmed.is_empty() {
        return String::new();
    }
    let total = trimmed.lines().count();
    if total <= n {
        return trimmed.to_string();
    }
    trimmed
        .lines()
        .skip(total - n)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_exit_zero() {
        let r = run_shell(None, "echo hello");
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.tail.trim(), "hello");
    }

    #[test]
    fn merges_stderr_and_propagates_nonzero_exit() {
        let r = run_shell(None, "echo out; echo err 1>&2; exit 3");
        assert_eq!(r.exit_code, 3);
        assert!(r.tail.contains("out"));
        assert!(r.tail.contains("err"));
    }

    #[test]
    fn tail_keeps_last_16_lines() {
        let r = run_shell(None, "for i in $(seq 1 40); do echo line$i; done");
        assert_eq!(r.exit_code, 0);
        let lines: Vec<_> = r.tail.lines().collect();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.first().copied(), Some("line25"));
        assert_eq!(lines.last().copied(), Some("line40"));
    }

    #[test]
    fn respects_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let r = run_shell(Some(tmp.path()), "pwd");
        assert_eq!(r.exit_code, 0);
        let pwd = r.tail.trim();
        let want = tmp.path().canonicalize().unwrap();
        let got = std::path::Path::new(pwd).canonicalize().unwrap();
        assert_eq!(got, want);
    }
}
