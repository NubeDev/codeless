//! Renderers for synchronous Slack replies. Each command produces one
//! plain-text block the dispatcher posts back into the same channel
//! (and thread, when present). The shape mirrors the SCOPE-MUTABLE-UI
//! Surface 1 mockups — short, scannable, mobile-friendly — with the
//! emoji glyphs in the design doc replaced by ASCII tags so the output
//! follows the repo-wide ban on emoji rendering (CLAUDE.md R2). The
//! tags carry the same meaning: `[ok]` for successful state changes,
//! `[fail]` for refusals, `[!]` for warnings the operator should look
//! at before acting.
//!
//! Renderers stay pure (no I/O, no `RpcServer` access) so the
//! dispatcher can unit-test the formatting separately from the network
//! plumbing — the only thing the message body depends on is the value
//! the RPC returned. Risk 1 in the SCOPE doc names the load-bearing
//! invariant: every action reply echoes the job's template name so the
//! operator catches a wrong-id mistake before the runtime acts.

use codeless_rpc::error::RpcError;
use codeless_rpc::methods::{ChatMode, ListJobsResult, StageRollup};
use codeless_types::time::UnixMillis;
use codeless_types::{Job, JobStatus, StageStatus, StopReason};

use crate::command::ParseError;

/// Cap on the number of jobs `format_list_jobs` renders. Slack
/// truncates long messages and the operator scanning on a phone wants
/// a glance, not a scroll. The SCOPE doc fixes the ceiling at ~10
/// lines; the extra trailing line ("…and N more") accounts for the
/// remainder so the operator knows the list was trimmed.
const STATUS_LIST_CAP: usize = 10;

/// Format the bare `status` command's reply: one header line and one
/// row per job, capped at `STATUS_LIST_CAP`. Empty result emits a
/// single line so the operator does not get a blank message back.
pub fn format_list_jobs(result: &ListJobsResult) -> String {
    if result.jobs.is_empty() {
        return "No jobs yet. Submit one from the web UI or `codeless` CLI.".to_string();
    }

    let total = result.jobs.len();
    let mut out = String::new();
    out.push_str(&format!(
        "{} active job{} across this server:\n",
        total,
        if total == 1 { "" } else { "s" }
    ));
    for (idx, job) in result.jobs.iter().take(STATUS_LIST_CAP).enumerate() {
        out.push_str(&format!(
            "  {n}. {status:<14}  {name}  ${cost:.2}\n",
            n = idx + 1,
            status = status_word(job.status),
            name = template_name(job).unwrap_or_else(|| "(no-template)".to_string()),
            cost = (job.cost_cents.as_i64() as f64) / 100.0,
        ));
    }
    if total > STATUS_LIST_CAP {
        out.push_str(&format!("  …and {} more\n", total - STATUS_LIST_CAP));
    }
    out.push_str("\nReply with the number: `status 3`, `resume 6 bypass`, `stop 2`.");
    out
}

/// Format `status <id>` (or the in-thread bare `status`). One short
/// block: name, status, wall-clock duration vs allowance, stage
/// count, and (when present) the currently-running stage plus the
/// last stage that ended. Cost is intentionally omitted — operators
/// who care open the UI; the bot reply is for the timing/progress
/// signal that drives the "is it stuck?" question.
pub fn format_get_job(job: &Job, stages: &[StageRollup]) -> String {
    let name = template_name(job).unwrap_or_else(|| "(no-template)".to_string());
    let now = now_millis();
    let elapsed = job
        .started_at
        .map(|s| (job.ended_at.unwrap_or(UnixMillis(now)).as_i64() - s.as_i64()).max(0));
    let mut out = format!(
        "Job `{id}` ({name})\n  Status: {status}\n  Time:   {elapsed} / {allowance}\n  Stages: {count}\n",
        id = short_id(&job.id.to_string()),
        name = name,
        status = status_word(job.status),
        elapsed = elapsed.map(fmt_duration_ms).unwrap_or_else(|| "—".into()),
        allowance = if job.wall_clock_cap_ms > 0 {
            fmt_duration_ms(job.wall_clock_cap_ms)
        } else {
            "—".into()
        },
        count = stages.len(),
    );
    if let Some(current) = current_stage(stages) {
        out.push_str(&format!("  Current: {}\n", fmt_stage_line(current, now)));
    }
    if let Some(last) = last_stage(stages) {
        out.push_str(&format!("  Last:    {}\n", fmt_stage_line(last, now)));
    }
    if let Some(reason) = job.stop_reason {
        out.push_str(&format!("  Reason:  {}\n", stop_reason_word(reason)));
    }
    out.push_str("\nReply: `resume [bypass | \"<comment>\"]` or `stop`.");
    out
}

/// First stage still in flight, in ordinal order. `Running` outranks
/// `AwaitingReview` so a stage with a pending verify gate but no
/// active runner is surfaced as the current focus rather than hidden
/// behind a later `Running` stage (which shouldn't happen in linear
/// mode, but the ordering keeps the renderer total).
fn current_stage(stages: &[StageRollup]) -> Option<&StageRollup> {
    let mut sorted: Vec<&StageRollup> = stages.iter().collect();
    sorted.sort_by_key(|s| s.stage.ordinal);
    sorted
        .iter()
        .find(|s| matches!(s.stage.status, StageStatus::Running))
        .or_else(|| {
            sorted
                .iter()
                .find(|s| matches!(s.stage.status, StageStatus::AwaitingReview))
        })
        .copied()
}

/// Most recently ended stage (`Passed` or `Failed`), by `ended_at`.
/// Falls back to highest ordinal among terminal stages so a row
/// with a missing `ended_at` (legacy data) still surfaces.
fn last_stage(stages: &[StageRollup]) -> Option<&StageRollup> {
    stages
        .iter()
        .filter(|s| matches!(s.stage.status, StageStatus::Passed | StageStatus::Failed))
        .max_by_key(|s| {
            (
                s.stage.ended_at.map(|t| t.as_i64()).unwrap_or(0),
                s.stage.ordinal as i64,
            )
        })
}

fn fmt_stage_line(s: &StageRollup, now: i64) -> String {
    let dur = match (s.stage.started_at, s.stage.ended_at) {
        (Some(start), Some(end)) => Some((end.as_i64() - start.as_i64()).max(0)),
        (Some(start), None) => Some((now - start.as_i64()).max(0)),
        _ => None,
    };
    format!(
        "{ord}. {name} ({status}) {dur}",
        ord = s.stage.ordinal,
        name = s.stage.name,
        status = stage_status_word(s.stage.status),
        dur = dur.map(fmt_duration_ms).unwrap_or_else(|| "—".into()),
    )
}

fn stage_status_word(s: StageStatus) -> &'static str {
    match s {
        StageStatus::Pending => "pending",
        StageStatus::Running => "running",
        StageStatus::AwaitingReview => "awaiting-review",
        StageStatus::Passed => "passed",
        StageStatus::Failed => "failed",
    }
}

/// Render a wall-clock duration in millis as a short two-unit
/// string. The renderer picks the largest non-zero unit and pairs
/// it with the next one down so the operator gets a single token
/// that's both readable and roughly accurate (`1h 23m`, `12m 4s`,
/// `37s`). Anything under a second collapses to `<1s` rather than
/// a noisy `0s`.
fn fmt_duration_ms(ms: i64) -> String {
    if ms < 1000 {
        return "<1s".into();
    }
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Format the `start <id>` reply. The job is now `Queued`; the runtime
/// driver picks it up on its next sweep. Echoes the template name so a
/// wrong-id mistake is visible immediately.
pub fn format_start_job(job: &Job) -> String {
    format!(
        "[ok] Starting {name}: now {status}. Watching for next event.",
        name = template_name(job).unwrap_or_else(|| format!("`{}`", short_id(&job.id.to_string()))),
        status = status_word(job.status),
    )
}

/// Format the `stop` / `stop <id>` reply. `stop_job` returns `()` so
/// the dispatcher passes the resolved id (and a previously-fetched
/// name when available) directly. The renderer takes the name as
/// `Option<&str>` so a path that did not pre-fetch the job row degrades
/// gracefully to the short-id form.
pub fn format_stop_job(job_id_display: &str, template: Option<&str>) -> String {
    match template {
        Some(name) => format!(
            "[ok] Stopped {name} (`{id}`).",
            name = name,
            id = job_id_display
        ),
        None => format!("[ok] Stopped `{id}`.", id = job_id_display),
    }
}

/// Format the resume reply. `bypass` and `comment` come straight from
/// `ResumeJobArgs` — both surface in the message because the operator
/// needs to confirm they applied what they meant (Risk 1 / Risk 3 in
/// the SCOPE doc).
pub fn format_resume_job(job: &Job, bypass: bool, comment: Option<&str>) -> String {
    let name = template_name(job).unwrap_or_else(|| format!("`{}`", short_id(&job.id.to_string())));
    let mut out = format!("[ok] Resuming {name}: now {}.", status_word(job.status));
    if bypass {
        out.push_str(" Failing stage marked bypassed.");
    }
    if let Some(text) = comment {
        // The dispatcher trims the comment; an empty string never
        // reaches here (the runtime also normalises Some("") to None,
        // but the renderer is the user-facing surface and a stray
        // empty heading on the way out would confuse a careful reader).
        let snippet = truncate_for_echo(text, 80);
        out.push_str(&format!(" Comment threaded into next stage: \"{snippet}\""));
    }
    out
}

/// Canned help block. Keep one help text and refer to it from every
/// unknown-input path so the operator sees one grammar reminder, not
/// two slightly different ones (the SCOPE doc rejects multiple quoting
/// conventions for exactly this reason).
pub fn format_help() -> String {
    [
        "Codeless bot commands:",
        "",
        "  status                 list active jobs (numbered)",
        "  status <N or id>       one-job detail",
        "  start <N or id>        promote Draft -> Queued",
        "  stop [<N or id>]       stop a Running/Queued job",
        "  resume [<N or id>] [bypass] [\"<comment>\"]",
        "                         re-queue a Stopped/Failed/Paused job",
        "  chat [<N or id>] <msg> one-shot agent chat against the job",
        "  spec [<N or id>] <msg> agent chat clamped to .codeless/jobs/<name>/",
        "",
        "Use the number from the last `status` list (e.g. `resume 3`).",
        "In a notification thread the job is implied by the thread.",
        "Full 26-char ULIDs also accepted.",
    ]
    .join("\n")
}

/// Format a `ParseError` for the operator. The parser already carries
/// enough context in each variant — this is a thin wrapper that
/// preserves the surface and adds the `[fail]` tag.
pub fn format_parse_error(err: &ParseError) -> String {
    match err {
        ParseError::Empty => format_help(),
        other => format!("[fail] {other}\nReply `help` for the grammar."),
    }
}

/// Format an `RpcError` for the operator. Most variants surface
/// verbatim; the runtime's `Conflict` and `NotFound` messages already
/// name the offending state.
pub fn format_rpc_error(err: &RpcError) -> String {
    format!("[fail] {err}")
}

/// Soft cap on the chat reply length. Telegram's message limit is
/// 4096 chars; we leave headroom for the header and the truncation
/// notice. Operators who need the full transcript can read it in
/// the UI, which subscribes to the same event stream.
const CHAT_REPLY_SOFT_CAP: usize = 3500;

/// Render an aggregated chat reply for transport. The header tags
/// the mode so an operator running both verbs in one thread can
/// tell which turn replied. An empty body becomes a placeholder
/// rather than a bare header.
pub fn format_chat_reply(mode: ChatMode, body: &str) -> String {
    let tag = match mode {
        ChatMode::Work => "[chat]",
        ChatMode::Spec => "[spec]",
    };
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return format!("{tag} (no reply within timeout — check the UI for the full transcript)");
    }
    let body = if trimmed.chars().count() > CHAT_REPLY_SOFT_CAP {
        let mut cut: String = trimmed.chars().take(CHAT_REPLY_SOFT_CAP).collect();
        cut.push_str("\n…(truncated, see UI)");
        cut
    } else {
        trimmed.to_string()
    };
    format!("{tag}\n{body}")
}

/// Extract the `name:` value from the job's `template_yaml`. A full
/// YAML parse is overkill (and a needless dep on the slack crate)
/// since the codeless template schema fixes `name` as the first
/// top-level key in every well-formed template. Skips comments and
/// blank lines; returns `None` for an unset / unparseable column.
pub fn template_name(job: &Job) -> Option<String> {
    let yaml = job.template_yaml.as_deref()?;
    for raw in yaml.lines() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Top-level keys are unindented; stop at the first indented
        // line so a nested `name:` (inside `stages:`) does not get
        // mis-read as the template's own name.
        if raw.starts_with(char::is_whitespace) {
            return None;
        }
        let rest = line.strip_prefix("name:")?;
        let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
        if value.is_empty() {
            return None;
        }
        return Some(value.to_string());
    }
    None
}

/// Compact a ULID for display: keep the first 6 and last 4 chars so
/// the prefix-based mental id-recognition the operator does
/// (`01KRPVJX…M4S59Z5D`) still works in a Slack message without
/// burning a full line on the id.
fn short_id(id: &str) -> String {
    if id.len() <= 14 {
        return id.to_string();
    }
    let head = &id[..6];
    let tail = &id[id.len() - 4..];
    format!("{head}...{tail}")
}

/// Truncate the echoed comment so a multi-paragraph comment does not
/// overflow the one-line reply. Slack renders newlines inside the
/// quoted echo as line breaks; the truncate keeps the echo on one
/// line and adds an ellipsis so the operator can tell the comment was
/// trimmed.
fn truncate_for_echo(text: &str, max_chars: usize) -> String {
    let mut buf = String::with_capacity(max_chars + 1);
    let mut chars = text.chars();
    for _ in 0..max_chars {
        match chars.next() {
            Some('\n') => {
                buf.push(' ');
            }
            Some(c) => buf.push(c),
            None => return buf,
        }
    }
    if chars.next().is_some() {
        buf.push('…');
    }
    buf
}

fn status_word(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Draft => "draft",
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::AwaitingReview => "awaiting-review",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Stopped => "stopped",
        JobStatus::Paused => "paused",
    }
}

fn stop_reason_word(r: StopReason) -> &'static str {
    match r {
        StopReason::User => "user-stopped",
        StopReason::CostCap => "cost-cap exceeded",
        StopReason::WallClock => "wall-clock exceeded",
        StopReason::RunnerCrash => "runner crashed",
        StopReason::AutoBypassThrashing => "auto-bypass thrashing",
        StopReason::ReviewPreCheck => "review pre-check failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_types::{CostCents, JobId, RepoId, WorkspaceMode};

    fn sample_job(name: &str) -> Job {
        Job {
            id: JobId::new(),
            repo_id: RepoId::new(),
            status: JobStatus::Running,
            stop_reason: None,
            template_yaml: Some(format!("name: {name}\ngoal: x\nstages: []\n")),
            prompt: None,
            runner: "claude".to_string(),
            branch: "codeless/x".to_string(),
            workspace_mode: WorkspaceMode::InRepo,
            worktree_path: None,
            cost_cap_cents: CostCents(15000),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(5264),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: codeless_types::time::UnixMillis(0),
        }
    }

    #[test]
    fn template_name_handles_well_formed_yaml() {
        let job = sample_job("scope-mutable-ui");
        assert_eq!(template_name(&job).as_deref(), Some("scope-mutable-ui"));
    }

    #[test]
    fn template_name_returns_none_when_yaml_absent() {
        let mut job = sample_job("x");
        job.template_yaml = None;
        assert!(template_name(&job).is_none());
    }

    #[test]
    fn template_name_skips_comments_and_blanks() {
        let mut job = sample_job("ignored");
        job.template_yaml = Some("# header comment\n\nname: real-name\nstages: []\n".to_string());
        assert_eq!(template_name(&job).as_deref(), Some("real-name"));
    }

    #[test]
    fn template_name_ignores_nested_name_inside_stages() {
        let mut job = sample_job("ignored");
        // A nested `name:` under `stages:` must not be picked up as
        // the template's own name; the helper bails as soon as it
        // hits an indented line without seeing a top-level `name:`.
        job.template_yaml = Some("stages:\n  - name: step-one\n    run: noop\n".to_string());
        assert!(template_name(&job).is_none());
    }

    #[test]
    fn format_list_jobs_renders_summary_with_cap() {
        let mut result = ListJobsResult { jobs: Vec::new() };
        for i in 0..(STATUS_LIST_CAP + 3) {
            let mut j = sample_job(&format!("job-{i}"));
            j.status = JobStatus::Failed;
            j.cost_cents = CostCents(123);
            result.jobs.push(j);
        }
        let body = format_list_jobs(&result);
        let cap_hint = format!("…and {} more", 3);
        assert!(body.contains(&cap_hint), "cap hint missing in:\n{body}");
        // The first job's template name should appear, the last
        // (over-cap) one's should not.
        assert!(body.contains("job-0"));
        assert!(!body.contains("job-12"));
    }

    #[test]
    fn format_list_jobs_handles_empty() {
        let result = ListJobsResult { jobs: vec![] };
        let body = format_list_jobs(&result);
        assert!(body.starts_with("No jobs"), "got: {body}");
    }

    #[test]
    fn format_get_job_renders_duration_and_stages() {
        let mut job = sample_job("smscope-smoke");
        job.status = JobStatus::Failed;
        job.stop_reason = Some(StopReason::CostCap);
        job.started_at = Some(UnixMillis(0));
        job.ended_at = Some(UnixMillis(125_000));
        job.wall_clock_cap_ms = 3_600_000;
        let body = format_get_job(&job, &[]);
        assert!(body.contains("smscope-smoke"));
        assert!(body.contains("failed"));
        assert!(body.contains("Time:"));
        assert!(body.contains("2m 5s"));
        assert!(body.contains("1h 0m"));
        assert!(body.contains("Stages: 0"));
        assert!(body.contains("cost-cap exceeded"));
        assert!(!body.contains("Cost:"));
    }

    fn stage_row(
        ordinal: u32,
        name: &str,
        status: StageStatus,
        started: Option<i64>,
        ended: Option<i64>,
    ) -> StageRollup {
        use codeless_types::id::{JobId, StageId};
        StageRollup {
            stage: codeless_types::Stage {
                id: StageId::new(),
                job_id: JobId::new(),
                ordinal,
                name: name.to_string(),
                status,
                verify_cmd: None,
                started_at: started.map(UnixMillis),
                ended_at: ended.map(UnixMillis),
                session_id: None,
                goal: None,
                acceptance: None,
                last_activity_at: None,
                archived: false,
                persona_id: None,
                bypassed_at: None,
                bypassed_reason: None,
            },
            cost_cents: 0,
            task_count: 0,
        }
    }

    #[test]
    fn format_get_job_surfaces_current_and_last_stage() {
        let mut job = sample_job("plugin-substrate");
        job.started_at = Some(UnixMillis(0));
        let stages = vec![
            stage_row(1, "PS2", StageStatus::Passed, Some(0), Some(60_000)),
            stage_row(2, "PS3", StageStatus::Passed, Some(60_000), Some(180_000)),
            stage_row(3, "PS4", StageStatus::Running, Some(180_000), None),
            stage_row(4, "PS5", StageStatus::Pending, None, None),
        ];
        let body = format_get_job(&job, &stages);
        assert!(body.contains("Stages: 4"), "body: {body}");
        assert!(body.contains("Current: 3. PS4"), "body: {body}");
        assert!(body.contains("Last:    2. PS3"), "body: {body}");
        assert!(body.contains("2m 0s"), "body: {body}");
    }

    #[test]
    fn format_start_job_echoes_template_name() {
        let mut job = sample_job("hello-gin");
        job.status = JobStatus::Queued;
        let body = format_start_job(&job);
        assert!(body.contains("[ok]"));
        assert!(body.contains("hello-gin"));
        assert!(body.contains("queued"));
    }

    #[test]
    fn format_stop_job_handles_both_paths() {
        let with_name = format_stop_job("01KRP...", Some("scope-mutable-ui"));
        assert!(with_name.contains("scope-mutable-ui"));
        let bare = format_stop_job("01KRP...", None);
        assert!(bare.contains("`01KRP...`"));
    }

    #[test]
    fn format_resume_includes_bypass_and_comment() {
        let mut job = sample_job("scope-mutable-ui");
        job.status = JobStatus::Queued;
        let body = format_resume_job(&job, true, Some("please redo this stage"));
        assert!(body.contains("scope-mutable-ui"));
        assert!(body.contains("bypassed"));
        assert!(body.contains("please redo this stage"));
    }

    #[test]
    fn format_resume_without_bypass_or_comment() {
        let mut job = sample_job("smscope-smoke");
        job.status = JobStatus::Queued;
        let body = format_resume_job(&job, false, None);
        assert!(!body.contains("bypassed"));
        assert!(!body.contains("Comment"));
    }

    #[test]
    fn format_resume_truncates_long_comments() {
        let mut job = sample_job("smscope-smoke");
        job.status = JobStatus::Queued;
        let big = "a".repeat(500);
        let body = format_resume_job(&job, false, Some(&big));
        assert!(body.contains('…'), "expected ellipsis in:\n{body}");
    }

    #[test]
    fn format_parse_error_routes_empty_to_help() {
        let body = format_parse_error(&ParseError::Empty);
        assert!(body.contains("Codeless bot commands"));
    }

    #[test]
    fn format_parse_error_surfaces_specific_message() {
        let body = format_parse_error(&ParseError::MissingJobId { verb: "resume" });
        assert!(body.contains("[fail]"));
        assert!(body.contains("resume"));
        assert!(body.contains("help"));
    }

    #[test]
    fn format_rpc_error_renders_typed_variants() {
        let body = format_rpc_error(&RpcError::NotFound("job 01KRP".into()));
        assert!(body.starts_with("[fail]"));
        assert!(body.contains("not found"));
    }
}
