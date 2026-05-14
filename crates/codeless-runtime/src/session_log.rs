//! Append-only session log — the other half of JOB-MODEL.md's
//! inter-session contract. The handover answers "what did this
//! session learn"; the log answers "what happened during this
//! session". One block per run, ordered, never rewritten.
//!
//! Lives at `runs/<job_id>/log.md` inside the worktree, alongside
//! `handover.md`. Today both keys are the job ULID; when job-name
//! plumbing lands (JOB-MODEL.md "Files in the user's repo vs. the
//! runtime's data") both files migrate to `<repo>/runs/<name>/`.
//!
//! Like the handover writer, a failure to land the file is logged
//! but never propagated: the work already succeeded, and the next
//! session can reconstruct timing from the events table if needed.

use std::path::{Path, PathBuf};

use codeless_types::{CostCents, Job, JobId, JobStatus};
use tokio::fs;
use tokio::io::AsyncWriteExt;

const FILE_NAME: &str = "log.md";

/// Path the session log lives at, sibling to the handover. Free
/// function so callers can stat without constructing a full builder.
pub fn log_path(worktree_root: &Path, job_id: JobId) -> PathBuf {
    worktree_root
        .join("runs")
        .join(job_id.to_string())
        .join(FILE_NAME)
}

/// What ended the run, in the language JOB-MODEL.md's example uses
/// ("context handoff", "review gate", "wall-clock cap"). The driver
/// has the authoritative answer; this enum is the closed set we map
/// to so the log stays consistent across runs.
#[derive(Debug, Clone, Copy)]
pub enum EndReason {
    /// Completed cleanly.
    Completed,
    /// Runner reported a failure (panic, model error, etc.).
    Failed,
    /// Stopped externally — cap, user, etc. Stop reason carries the
    /// flavour.
    Stopped,
}

impl EndReason {
    fn label(self) -> &'static str {
        match self {
            EndReason::Completed => "completed",
            EndReason::Failed => "failed",
            EndReason::Stopped => "stopped",
        }
    }

    /// Map a terminal job status into an `EndReason`. Non-terminal
    /// statuses round to `Stopped` so we never panic on an unexpected
    /// transition; the log is best-effort.
    pub fn from_status(status: JobStatus) -> Self {
        match status {
            JobStatus::Completed => EndReason::Completed,
            JobStatus::Failed => EndReason::Failed,
            _ => EndReason::Stopped,
        }
    }
}

/// Append a session block to `log.md`. Creates parent directories and
/// the file if absent. The block format matches JOB-MODEL.md's three
/// fields: `Did`, `Cost`, `Reason for ending`. Section numbering is
/// derived by counting existing `## Session ` headings — we do not
/// trust an externally-set counter because the file may have been
/// edited by hand between runs.
pub async fn append_session_block(
    worktree_root: &Path,
    job: &Job,
    end: EndReason,
) -> std::io::Result<PathBuf> {
    let path = log_path(worktree_root, job.id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let existing = match fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let next_number = count_sessions(&existing) + 1;
    let block = render_block(next_number, job, end);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    if existing.is_empty() {
        file.write_all(format!("# Log — {}\n", job.id).as_bytes())
            .await?;
    }
    file.write_all(block.as_bytes()).await?;
    file.flush().await?;
    Ok(path)
}

fn count_sessions(existing: &str) -> usize {
    existing
        .lines()
        .filter(|l| l.starts_with("## Session "))
        .count()
}

fn render_block(number: usize, job: &Job, end: EndReason) -> String {
    let started = job
        .started_at
        .map(format_timestamp)
        .unwrap_or_else(|| "?".to_string());
    let ended = job
        .ended_at
        .map(format_timestamp)
        .unwrap_or_else(|| "?".to_string());
    let did = match &job.prompt {
        Some(p) if !p.trim().is_empty() => first_line(p),
        _ => "(no prompt)".to_string(),
    };
    let cost = format_cents(job.cost_cents);
    let stop_suffix = match &job.stop_reason {
        Some(r) => format!(" ({r:?})"),
        None => String::new(),
    };
    format!(
        "\n## Session {number} — {started} → {ended}\n\
         Did: {did}\n\
         Cost: {cost}. Reason for ending: {label}{stop_suffix}.\n",
        label = end.label(),
    )
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return "(empty)".to_string();
    }
    // Trim to a readable preview; the full prompt lives in SQLite and
    // the UI surface. The log entry is a tracer, not a transcript.
    if line.chars().count() > 200 {
        let snippet: String = line.chars().take(200).collect();
        format!("{snippet}…")
    } else {
        line.to_string()
    }
}

fn format_cents(c: CostCents) -> String {
    // Render in dollars with two decimals for any non-zero amount;
    // sub-cent rounding is fine here because CostCents is integer
    // cents already. Zero gets the same shape so the column lines up.
    format!("${:.02}", c.0 as f64 / 100.0)
}

fn format_timestamp(ms: codeless_types::UnixMillis) -> String {
    // Format a UTC RFC3339 timestamp trimmed to the second without
    // pulling chrono / humantime / time as a new dependency. The log
    // is read by humans; sub-second precision and timezone gymnastics
    // do not earn their weight here. If a richer date helper lands
    // later (`crate::time` is the natural home), swap this for that.
    let total_secs = ms.0 / 1000;
    let (y, mo, d, h, mi, s) = epoch_secs_to_utc(total_secs);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}Z")
}

fn epoch_secs_to_utc(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    // Civil-from-days, adapted from Howard Hinnant's
    // "date" algorithm — public domain, the canonical 32-line
    // implementation that every stdlib reimplements somewhere.
    let days = secs.div_euclid(86_400);
    let secs_in_day = secs.rem_euclid(86_400) as u32;
    let h = secs_in_day / 3600;
    let mi = (secs_in_day % 3600) / 60;
    let s = secs_in_day % 60;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y0 = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y0 + 1 } else { y0 };
    (y, mo, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{CostCents, Job, JobStatus, RepoId, UnixMillis, WorkspaceMode};

    fn sample_job(prompt: &str) -> Job {
        Job {
            id: JobId::new(),
            repo_id: RepoId::new(),
            status: JobStatus::Completed,
            stop_reason: None,
            template_yaml: None,
            prompt: Some(prompt.into()),
            runner: "mock".into(),
            branch: "codeless/test".into(),
            workspace_mode: WorkspaceMode::default(),
            worktree_path: None,
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            cost_cents: CostCents(42),
            started_at: Some(UnixMillis(1_778_000_000_000)),
            ended_at: Some(UnixMillis(1_778_000_060_000)),
            created_at: UnixMillis(1_778_000_000_000),
        }
    }

    #[tokio::test]
    async fn first_session_writes_header_and_block() {
        let tmp = tempfile::tempdir().unwrap();
        let job = sample_job("do a thing");
        let path = append_session_block(tmp.path(), &job, EndReason::Completed)
            .await
            .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("# Log — "));
        assert!(body.contains("## Session 1 — "));
        assert!(body.contains("Did: do a thing"));
        assert!(body.contains("Cost: $0.42. Reason for ending: completed."));
    }

    #[tokio::test]
    async fn second_session_increments_and_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let job1 = sample_job("first");
        let job2 = Job {
            id: job1.id,
            ..sample_job("second")
        };
        append_session_block(tmp.path(), &job1, EndReason::Completed)
            .await
            .unwrap();
        let path = append_session_block(tmp.path(), &job2, EndReason::Failed)
            .await
            .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("## Session 1 —"));
        assert!(body.contains("## Session 2 —"));
        assert!(body.contains("Did: second"));
        assert!(body.contains("Reason for ending: failed"));
    }

    #[test]
    fn truncates_long_prompt_first_line() {
        let long = "x".repeat(500);
        let job = sample_job(&long);
        let block = render_block(1, &job, EndReason::Completed);
        // 200-char cap plus ellipsis; the rendered Did line is one
        // line so we can count chars in the block easily.
        let did_line = block
            .lines()
            .find(|l| l.starts_with("Did: "))
            .expect("did line");
        assert!(did_line.ends_with('…'));
    }

    #[test]
    fn empty_prompt_renders_placeholder() {
        let mut job = sample_job("ignored");
        job.prompt = None;
        let block = render_block(1, &job, EndReason::Completed);
        assert!(block.contains("Did: (no prompt)"));
    }
}
