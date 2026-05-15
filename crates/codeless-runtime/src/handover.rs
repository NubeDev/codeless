//! Writes the JOB-MODEL.md session-handover file on stage termination.
//!
//! `runs/<job_id>/<stage_id>/handover.md` is the contract between
//! sessions (DOCS/JOB-MODEL.md). The file is written per-stage, not
//! per-job: each stage produces exactly one handover, addressed by the
//! `(job_id, stage_id)` key that the next session can resolve from the
//! store. The mtime-ranked discovery that this module used to support
//! is gone — picking the newest handover on disk could (and did)
//! straddle unrelated jobs.
//!
//! Validation (H7): a handover whose `done` or `next` is empty is
//! rejected at write time. `done` may not be empty because a stage
//! that completes legitimately always landed *something* (even an
//! aborted stage records the abort). `next` may not be empty because
//! the canonical next action is the seed prompt for the next session;
//! a blank `next` would force the next session to re-derive its first
//! move from the diff, defeating the contract.

use std::path::Path;

use codeless_types::{Handover, JobId, JobStatus, StageId};
use tokio::fs;

/// Construct the conventional handover path for a stage:
/// `<worktree_root>/runs/<job_id>/<stage_id>/handover.md`. Kept as a
/// free function so the UI and the archive path
/// (`session_idle::handover_archive_path`) can mirror the same
/// convention without re-exporting through TS.
pub fn handover_path(worktree_root: &Path, job_id: JobId, stage_id: StageId) -> std::path::PathBuf {
    worktree_root
        .join("runs")
        .join(job_id.to_string())
        .join(stage_id.to_string())
        .join("handover.md")
}

/// Validation error emitted by [`validate_handover`]. The two empty-
/// section variants are the H7 floor: a handover that fails either
/// check is shipped against the runtime's intent, not what the
/// JOB-MODEL.md contract promises the next session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoverValidationError {
    EmptyDone,
    EmptyNext,
}

impl std::fmt::Display for HandoverValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandoverValidationError::EmptyDone => f.write_str(
                "handover `Done` section is empty; a stage that finished must record what landed \
                 (even an aborted stage records the abort)",
            ),
            HandoverValidationError::EmptyNext => f.write_str(
                "handover `Next` section is empty; the canonical next action is the seed prompt \
                 for the next session and may not be blank",
            ),
        }
    }
}

impl std::error::Error for HandoverValidationError {}

/// Error type returned by [`write_handover`]. Distinguishes the "you
/// gave me a malformed handover" case from the "the filesystem said
/// no" case so callers (and logs) can act on each separately.
#[derive(Debug)]
pub enum HandoverWriteError {
    Validation(HandoverValidationError),
    Io(std::io::Error),
}

impl std::fmt::Display for HandoverWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandoverWriteError::Validation(e) => write!(f, "handover validation failed: {e}"),
            HandoverWriteError::Io(e) => write!(f, "handover write failed: {e}"),
        }
    }
}

impl std::error::Error for HandoverWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HandoverWriteError::Validation(e) => Some(e),
            HandoverWriteError::Io(e) => Some(e),
        }
    }
}

impl From<HandoverValidationError> for HandoverWriteError {
    fn from(value: HandoverValidationError) -> Self {
        HandoverWriteError::Validation(value)
    }
}

impl From<std::io::Error> for HandoverWriteError {
    fn from(value: std::io::Error) -> Self {
        HandoverWriteError::Io(value)
    }
}

/// Reject handovers whose `Done` or `Next` sections are empty. The
/// runtime calls this from `write_handover` so a malformed write
/// surfaces as an error rather than landing a useless markdown file.
/// Callers that want to skip the file write entirely (because the
/// runner emitted no usable text) should not call `write_handover` at
/// all — there is no "write a placeholder anyway" path.
pub fn validate_handover(h: &Handover) -> Result<(), HandoverValidationError> {
    if h.done.iter().all(|s| s.trim().is_empty()) {
        return Err(HandoverValidationError::EmptyDone);
    }
    if h.next.iter().all(|s| s.trim().is_empty()) {
        return Err(HandoverValidationError::EmptyNext);
    }
    Ok(())
}

/// Write `handover` to `runs/<job_id>/<stage_id>/handover.md` inside
/// `worktree`. Creates the parent directories if missing. Validates
/// the handover first; an invalid one is rejected before any
/// filesystem state changes.
pub async fn write_handover(
    worktree: &Path,
    job_id: JobId,
    stage_id: StageId,
    handover: &Handover,
) -> Result<std::path::PathBuf, HandoverWriteError> {
    validate_handover(handover)?;
    let path = handover_path(worktree, job_id, stage_id);
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

/// Fallback handover for stages whose runner did not produce
/// structured output (mock runner, any runner that finished before we
/// wired in structured extraction). Names the runner that ran and the
/// terminal status; the "Next" section is intentionally a single hint
/// rather than empty so the H7 write-time check still accepts it.
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
        raw_tail: None,
    }
}

/// Look up the handover for a specific `(job_id, stage_id)` pair under
/// `<repo_path>/.codeless/worktrees/*/runs/<job_id>/<stage_id>/handover.md`.
/// Returns the resolved path and parsed body, or `None` if no readable
/// handover is present for that key.
///
/// This is the H3 replacement for the old `find_latest_handover` mtime
/// ranking. The caller chooses which stage they want a handover from;
/// the lookup never silently switches to a different job's handover
/// just because that file happens to be newer on disk. Multiple
/// worktree directories are scanned (a job can leave more than one
/// worktree behind across re-runs); on collision the first readable
/// match wins, which is acceptable because two worktrees writing the
/// same `(job_id, stage_id)` is already a workflow bug the user must
/// triage.
pub async fn find_handover(
    repo_path: &Path,
    job_id: JobId,
    stage_id: StageId,
) -> Option<(std::path::PathBuf, Handover)> {
    let worktrees = repo_path.join(".codeless").join("worktrees");
    let mut wt_dir = match tokio::fs::read_dir(&worktrees).await {
        Ok(d) => d,
        Err(_) => return None,
    };
    while let Ok(Some(wt_entry)) = wt_dir.next_entry().await {
        let candidate = wt_entry
            .path()
            .join("runs")
            .join(job_id.to_string())
            .join(stage_id.to_string())
            .join("handover.md");
        let Ok(meta) = tokio::fs::metadata(&candidate).await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(body) = tokio::fs::read_to_string(&candidate).await else {
            continue;
        };
        let Ok(handover) = Handover::from_markdown(&body) else {
            continue;
        };
        return Some((candidate, handover));
    }
    None
}

/// Render a handover as a prompt prefix the next runner sees. Names
/// the source path so the model can tell where the contract came from
/// and frames the four sections explicitly (the system prompt already
/// teaches the model the schema; this is the *previous* session's
/// answers, not the format).
pub fn prompt_prefix_for(path: &Path, h: &Handover) -> String {
    let mut out = String::new();
    out.push_str("# Prior session handover\n\n");
    out.push_str(&format!(
        "Read this before doing anything else. Source: `{}`.\n\n",
        path.display()
    ));
    out.push_str(&h.to_markdown());
    out.push_str("\n---\n\n# Your task\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::JobStatus;

    #[tokio::test]
    async fn writes_handover_to_per_stage_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let h = default_handover("mock", JobStatus::Completed);
        let written = write_handover(tmp.path(), job_id, stage_id, &h)
            .await
            .unwrap();
        let body = std::fs::read_to_string(&written).unwrap();
        assert!(body.contains("## Done"));
        assert!(body.contains("`mock` run completed"));
        assert!(written.ends_with(format!("runs/{job_id}/{stage_id}/handover.md")));
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
        let body = &h.what_you_need_to_know[0];
        assert!(body.starts_with('…'));
        assert!(body.chars().count() <= 201);
    }

    #[test]
    fn validate_rejects_empty_done() {
        let mut h = default_handover("mock", JobStatus::Completed);
        h.done.clear();
        assert_eq!(
            validate_handover(&h),
            Err(HandoverValidationError::EmptyDone)
        );
    }

    #[test]
    fn validate_rejects_whitespace_only_done() {
        let mut h = default_handover("mock", JobStatus::Completed);
        h.done = vec!["   ".into(), "\t".into()];
        assert_eq!(
            validate_handover(&h),
            Err(HandoverValidationError::EmptyDone)
        );
    }

    #[test]
    fn validate_rejects_empty_next() {
        let mut h = default_handover("mock", JobStatus::Completed);
        h.next.clear();
        assert_eq!(
            validate_handover(&h),
            Err(HandoverValidationError::EmptyNext)
        );
    }

    #[test]
    fn validate_rejects_whitespace_only_next() {
        let mut h = default_handover("mock", JobStatus::Completed);
        h.next = vec!["   ".into(), "\t\n".into()];
        assert_eq!(
            validate_handover(&h),
            Err(HandoverValidationError::EmptyNext)
        );
    }

    #[tokio::test]
    async fn write_handover_rejects_empty_next_without_touching_fs() {
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let mut h = default_handover("mock", JobStatus::Completed);
        h.next.clear();
        let err = write_handover(tmp.path(), job_id, stage_id, &h)
            .await
            .expect_err("empty next rejected");
        match err {
            HandoverWriteError::Validation(HandoverValidationError::EmptyNext) => {}
            other => panic!("expected EmptyNext, got {other:?}"),
        }
        let target = handover_path(tmp.path(), job_id, stage_id);
        assert!(
            !target.exists(),
            "handover file must not be created on validation failure"
        );
    }

    #[test]
    fn validate_accepts_populated_handover() {
        let h = default_handover("mock", JobStatus::Completed);
        assert!(validate_handover(&h).is_ok());
    }

    #[tokio::test]
    async fn write_handover_rejects_empty_done_without_touching_fs() {
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        let mut h = default_handover("mock", JobStatus::Completed);
        h.done.clear();
        let err = write_handover(tmp.path(), job_id, stage_id, &h)
            .await
            .expect_err("empty done rejected");
        match err {
            HandoverWriteError::Validation(HandoverValidationError::EmptyDone) => {}
            other => panic!("expected EmptyDone, got {other:?}"),
        }
        let target = handover_path(tmp.path(), job_id, stage_id);
        assert!(
            !target.exists(),
            "handover file must not be created on validation failure"
        );
    }

    #[tokio::test]
    async fn find_handover_resolves_keyed_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let job_id = JobId::new();
        let stage_a = StageId::new();
        let stage_b = StageId::new();
        let wt = repo
            .join(".codeless/worktrees")
            .join(format!("job-{job_id}"));
        let h_a = Handover {
            done: vec!["stage a".into()],
            next: vec!["go".into()],
            ..Default::default()
        };
        let h_b = Handover {
            done: vec!["stage b".into()],
            next: vec!["go".into()],
            ..Default::default()
        };
        write_handover(&wt, job_id, stage_a, &h_a).await.unwrap();
        write_handover(&wt, job_id, stage_b, &h_b).await.unwrap();

        let (path, parsed) = find_handover(repo, job_id, stage_a).await.expect("found");
        assert_eq!(parsed.done, vec!["stage a".to_string()]);
        assert!(path.to_string_lossy().contains(&stage_a.to_string()));

        let (_path_b, parsed_b) = find_handover(repo, job_id, stage_b).await.expect("found");
        assert_eq!(parsed_b.done, vec!["stage b".to_string()]);
    }

    #[tokio::test]
    async fn find_handover_returns_none_for_unknown_key() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let job_id = JobId::new();
        let stage_a = StageId::new();
        let wt = repo
            .join(".codeless/worktrees")
            .join(format!("job-{job_id}"));
        let h = Handover {
            done: vec!["stage a".into()],
            next: vec!["go".into()],
            ..Default::default()
        };
        write_handover(&wt, job_id, stage_a, &h).await.unwrap();

        let other_stage = StageId::new();
        assert!(find_handover(repo, job_id, other_stage).await.is_none());

        let other_job = JobId::new();
        assert!(find_handover(repo, other_job, stage_a).await.is_none());
    }

    #[tokio::test]
    async fn find_handover_returns_none_when_worktree_root_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let job_id = JobId::new();
        let stage_id = StageId::new();
        assert!(find_handover(tmp.path(), job_id, stage_id).await.is_none());
    }

    #[test]
    fn prompt_prefix_includes_path_and_sections() {
        let h = Handover {
            done: vec!["one thing".into()],
            next: vec!["another".into()],
            ..Default::default()
        };
        let p = std::path::Path::new("/repo/.codeless/worktrees/job-X/runs/X/Y/handover.md");
        let prefix = prompt_prefix_for(p, &h);
        assert!(prefix.contains("# Prior session handover"));
        assert!(prefix.contains("/repo/.codeless"));
        assert!(prefix.contains("## Done"));
        assert!(prefix.contains("one thing"));
        assert!(prefix.contains("# Your task"));
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
