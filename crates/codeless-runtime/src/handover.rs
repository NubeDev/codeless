//! Writes the JOB-MODEL.md session-handover file on job termination.
//!
//! `runs/<job_id>/handover.md` is the contract between sessions
//! (DOCS/JOB-MODEL.md). The runtime always writes it on terminal
//! status — even if all the runner gave us was "Completed" — so the
//! next session has *something* to read. Real runners can produce
//! richer content by emitting a structured handover the driver can
//! extract; that path lands separately.
//!
//! Errors writing the file are logged at warn level but never fail
//! the job: the job already succeeded by the time we get here, and
//! losing the handover is recoverable (the next session re-derives
//! state from the diff and the events stream). The alternative —
//! marking a Completed job as Failed because a markdown file did not
//! land — is a worse failure mode.

use std::path::Path;

use codeless_types::{Handover, JobId, JobStatus};
use tokio::fs;

/// Construct the conventional handover path for a job inside its
/// worktree: `<worktree_root>/runs/<job_id>/handover.md`. Kept as a
/// free function so the UI (`HandoverPanel.tsx`) can mirror the same
/// convention by hand without us re-exporting through TS — the path
/// shape is wire-stable and the UI's two-candidate probe already
/// names this layout.
pub fn handover_path(worktree_root: &Path, job_id: JobId) -> std::path::PathBuf {
    worktree_root
        .join("runs")
        .join(job_id.to_string())
        .join("handover.md")
}

/// Write `handover` to `runs/<job_id>/handover.md` inside `worktree`.
/// Creates the parent directories if missing. Returns the path that
/// was written on success.
pub async fn write_handover(
    worktree: &Path,
    job_id: JobId,
    handover: &Handover,
) -> std::io::Result<std::path::PathBuf> {
    let path = handover_path(worktree, job_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&path, handover.to_markdown()).await?;
    Ok(path)
}

/// Fallback handover for jobs whose runner did not produce structured
/// output (mock runner, any runner that finished before we wired in
/// structured extraction). Names the runner that ran and the terminal
/// status; the "Next" section is intentionally a single hint rather
/// than empty so an operator reading it sees a sentence, not a blank.
pub fn default_handover(runner: &str, status: JobStatus) -> Handover {
    let done_line = match status {
        JobStatus::Completed => format!("`{runner}` run completed without writing a handover"),
        JobStatus::Failed => format!("`{runner}` run failed before writing a handover"),
        JobStatus::Stopped => format!("`{runner}` run was stopped before writing a handover"),
        other => format!("`{runner}` run ended in status `{other:?}` without writing a handover"),
    };
    Handover {
        done: vec![done_line],
        next: vec![
            "Read the diff (Files changed tab) and the Timeline before deciding what to do next."
                .into(),
        ],
        what_you_need_to_know: vec![
            "The runner did not emit a structured handover block; this is the default fallback."
                .into(),
        ],
        open_questions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::JobStatus;

    #[tokio::test]
    async fn writes_handover_to_runs_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let h = default_handover("mock", JobStatus::Completed);
        let written = write_handover(tmp.path(), job_id, &h).await.unwrap();
        let body = std::fs::read_to_string(&written).unwrap();
        assert!(body.contains("## Done"));
        assert!(body.contains("`mock` run completed"));
        assert!(written.ends_with(format!("runs/{job_id}/handover.md")));
    }

    #[test]
    fn default_handover_distinguishes_failure_modes() {
        let h_completed = default_handover("mock", JobStatus::Completed);
        let h_failed = default_handover("mock", JobStatus::Failed);
        let h_stopped = default_handover("mock", JobStatus::Stopped);
        assert!(h_completed.done[0].contains("completed"));
        assert!(h_failed.done[0].contains("failed"));
        assert!(h_stopped.done[0].contains("stopped"));
    }
}
