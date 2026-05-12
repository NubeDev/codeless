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

/// Extract a `Handover` from the assistant's accumulated text by
/// finding a fenced ```handover (or `handover-md`) code block and
/// parsing its body via [`Handover::from_markdown`]. The fenced-block
/// convention is what the headless system prompt tells the model to
/// emit, so the success path is: find one block, parse it cleanly,
/// return it.
///
/// When no block is present (or it does not parse), the caller decides
/// the fallback — typically [`default_handover`] augmented with a
/// truncated tail of the assistant text in `done`. We intentionally
/// do not fall back inside this function: the caller knows the runner
/// id and the run status, and "no block" vs "malformed block" are
/// worth distinguishing in logs.
pub fn extract_handover(assistant_text: &str) -> Option<Handover> {
    let body = find_fenced_block(assistant_text)?;
    Handover::from_markdown(body).ok()
}

/// Build a fallback handover that preserves the final assistant
/// message (or its tail) in `done`. Used when the runner did not emit
/// a structured `handover` block. Truncates to `max_chars` so a
/// rambling final reply does not produce an unreadable handover; the
/// limit is generous on purpose (a handover is meant to be read by a
/// human or a fresh agent, not skimmed by a UI badge).
pub fn fallback_handover_from_text(
    runner: &str,
    status: JobStatus,
    assistant_text: &str,
    max_chars: usize,
) -> Handover {
    let mut h = default_handover(runner, status);
    let trimmed = assistant_text.trim();
    if trimmed.is_empty() {
        return h;
    }
    let body = if trimmed.chars().count() > max_chars {
        let tail: String = trimmed
            .chars()
            .rev()
            .take(max_chars)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    } else {
        trimmed.to_string()
    };
    h.done = vec![format!(
        "`{runner}` did not emit a structured handover; final message follows."
    )];
    h.what_you_need_to_know = vec![body];
    h
}

fn find_fenced_block(text: &str) -> Option<&str> {
    // Look for ```handover (case-insensitive, with optional -md / -markdown
    // suffix to allow the model to spell it either way). We accept the
    // first matching block; later ones are ignored on the assumption
    // that the canonical block lands last in a well-behaved reply but
    // a malformed model dump that includes the marker twice should
    // not silently flip-flop.
    let lower = text.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find("```") {
        let abs = search_from + rel;
        let after_fence = abs + 3;
        // Find the end of the fence's info string (the line break).
        let info_end = match text[after_fence..].find('\n') {
            Some(off) => after_fence + off,
            None => return None,
        };
        let info = lower[after_fence..info_end].trim();
        if info == "handover" || info == "handover-md" || info == "handover-markdown" {
            let body_start = info_end + 1;
            // Find the matching closing fence on its own line.
            let body_rest = &text[body_start..];
            let body_end_rel = body_rest.find("\n```")?;
            return Some(&body_rest[..body_end_rel]);
        }
        search_from = info_end + 1;
    }
    None
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

    #[test]
    fn extract_returns_none_when_no_block() {
        let text = "Here is a normal assistant reply with no fenced handover.";
        assert!(extract_handover(text).is_none());
    }

    #[test]
    fn extract_finds_fenced_handover_block() {
        let text = "\
Some prose first.

```handover
## Done

- landed the thing

## Next

- write a test

## What you need to know

- the API changed

## Open questions

- (none)
```

trailing prose";
        let h = extract_handover(text).expect("block parses");
        assert_eq!(h.done, vec!["landed the thing".to_string()]);
        assert_eq!(h.next, vec!["write a test".to_string()]);
        assert_eq!(h.what_you_need_to_know, vec!["the API changed".to_string()]);
        assert!(h.open_questions.is_empty());
    }

    #[test]
    fn extract_accepts_handover_md_info_string() {
        let text = "```handover-md\n## Done\n- ok\n## Next\n- (none)\n## What you need to know\n- (none)\n## Open questions\n- (none)\n```\n";
        let h = extract_handover(text).expect("alias parses");
        assert_eq!(h.done, vec!["ok".to_string()]);
    }

    #[test]
    fn fallback_truncates_long_text_and_stamps_metadata() {
        let long: String = "x".repeat(2000);
        let h = fallback_handover_from_text("claude", JobStatus::Completed, &long, 200);
        assert!(h.done[0].contains("`claude`"));
        assert!(h.done[0].contains("did not emit"));
        // The truncated body lives in what_you_need_to_know, prefixed
        // with the ellipsis marker we put there to signal a cut.
        let body = &h.what_you_need_to_know[0];
        assert!(body.starts_with('…'));
        assert!(body.chars().count() <= 201);
    }

    #[test]
    fn fallback_passes_short_text_through_unchanged() {
        let h = fallback_handover_from_text(
            "claude",
            JobStatus::Completed,
            "I made one small change.",
            200,
        );
        assert_eq!(
            h.what_you_need_to_know,
            vec!["I made one small change.".to_string()]
        );
    }
}
