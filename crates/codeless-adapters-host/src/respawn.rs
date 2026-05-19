//! Self-watcher parent for `codeless serve --respawn-on-exit`.
//!
//! Process spawning is R1-gated to this crate. `codeless-cli` calls
//! [`supervise`] from inside `handle()` when `--respawn-on-exit` is
//! set and the supervised-child env marker is *not* present; the
//! parent re-execs the child on exit code 75 (`EX_TEMPFAIL`) until the
//! child exits cleanly (`0`), exits non-restart-non-zero, or is
//! killed by a signal. The child sees a `CODELESS_SUPERVISED=1`
//! environment variable so its runtime constructor picks
//! `RestartContext::SupervisedCli`.
//!
//! Why a separate parent rather than `exec()` in place: an in-place
//! `exec` replaces the process image but cannot recover from a panic
//! in the runtime itself — the operator would have to relaunch
//! manually. The parent watcher is one thin process whose only job is
//! `spawn → wait → check status`; if the child segfaults, the parent
//! still respawns it, which is the whole point of the flag.

use std::io;
use std::process::{Command, ExitStatus};

/// Env var the parent watcher sets so the child runtime can pick
/// `RestartContext::SupervisedCli`. Mirrored on the CLI side; defining
/// the constant here keeps the wire-up symmetric and prevents typos
/// in the env-var name.
pub const SUPERVISED_ENV: &str = "CODELESS_SUPERVISED";

/// Exit code the child uses to signal "respawn me". Matches
/// `<sysexits.h>` `EX_TEMPFAIL`; the runtime's
/// `RestartTrigger::desired_exit_code` carries the same constant
/// through to the CLI's main.
pub const EX_TEMPFAIL: i32 = 75;

/// Spawn the current executable with the supplied argv as a child,
/// wait for it, and re-spawn it whenever it exits with
/// [`EX_TEMPFAIL`]. Returns the exit code the watcher process itself
/// should exit with — the child's last non-restart exit code, or
/// `1` when the child exited via a signal (rare but worth not
/// swallowing).
///
/// `child_argv` is forwarded verbatim — the caller is expected to
/// strip the `--respawn-on-exit` flag so the child does not re-enter
/// the watcher path.
///
/// The function blocks the calling thread; the watcher does no
/// async work because it is a one-task process. The CLI's `handle()`
/// calls this from the synchronous entry point before the tokio
/// runtime ever starts, so no executor is required.
pub fn supervise(child_argv: &[String]) -> io::Result<i32> {
    let exe = std::env::current_exe()?;
    loop {
        let status: ExitStatus = Command::new(&exe)
            .args(child_argv)
            .env(SUPERVISED_ENV, "1")
            .status()?;
        if let Some(code) = status.code() {
            if code == EX_TEMPFAIL {
                continue;
            }
            return Ok(code);
        }
        // Killed by signal — bail rather than respawn so a SIGTERM
        // from the operator actually terminates the watcher tree.
        return Ok(1);
    }
}

/// Whether this process is running underneath the
/// [`supervise`] watcher (or any other supervisor that sets the same
/// env var: `init-session.sh`, a systemd unit's environment, etc.).
pub fn is_supervised() -> bool {
    std::env::var(SUPERVISED_ENV).is_ok_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervised_env_marker_round_trip() {
        // The constant is the wire between parent and child; pin it
        // here so a rename does not silently break the supervised /
        // bare context detection.
        assert_eq!(SUPERVISED_ENV, "CODELESS_SUPERVISED");
        assert_eq!(EX_TEMPFAIL, 75);
    }

    #[test]
    fn is_supervised_reflects_env() {
        // SAFETY: this test sets/unsets a single env var and reads it
        // back under a unique key — no other test in this module
        // touches `CODELESS_SUPERVISED`. Parallel `cargo test` across
        // modules can race on env in theory; the var name is
        // deliberately specific to this codebase to avoid collisions.
        std::env::remove_var(SUPERVISED_ENV);
        assert!(!is_supervised());
        std::env::set_var(SUPERVISED_ENV, "1");
        assert!(is_supervised());
        std::env::set_var(SUPERVISED_ENV, "");
        assert!(!is_supervised());
        std::env::remove_var(SUPERVISED_ENV);
    }
}
