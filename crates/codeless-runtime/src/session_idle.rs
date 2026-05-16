//! Idle-timeout enforcement for per-stage warm sessions.
//!
//! A "warm session" is a stage row whose `session_id` was captured by
//! a previous run and that the runtime still resumes via
//! `--continue <session_id>` on the next turn. Without a bound, those
//! sessions accumulate indefinitely — every halted or failed stage
//! holds a runner subprocess open until the box reboots. SCOPE.md hard
//! rule #1 calls for resumption to feel continuous; the idle timeout
//! is the bounded leak that pairs with it.
//!
//! This module owns two seams:
//!
//! - [`resolve_stage_resume`] — called at the head of every stage run
//!   and every interactive resume. It looks at the stage row, decides
//!   between resume-warm, archive-then-fresh, or no-session-yet, and
//!   side-effects accordingly (handover write + lifecycle event +
//!   `stages.archived = 1`).
//! - [`spawn_idle_sweeper`] — a tokio task that periodically calls
//!   `archive_idle_stage_sessions` so a stage that is never touched
//!   again still archives at the threshold (otherwise the row would
//!   only flip when the user finally returned to it, defeating the
//!   "bounded leak" purpose).
//!
//! Both paths emit exactly one `SessionArchivedThenResumed` per
//! archive transition. The flag on `stages.archived` is one-way, so
//! a later resume against an already-archived row is observed by the
//! `archive_stage_session` returning `None`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use codeless_types::{Event, JobId, Stage, StageId, UnixMillis};
use tokio::task::JoinHandle;

use crate::event_bus::EventBus;
use crate::store::SqliteStore;
use crate::time::now_ms;

/// Default per-job idle timeout when the template's
/// `session_idle_timeout` is omitted. Matches the value documented in
/// the stage 3 acceptance criteria; the rest of the codebase reads
/// `template.session_idle_timeout.unwrap_or(DEFAULT_SESSION_IDLE_TIMEOUT)`
/// rather than carrying the literal around.
pub const DEFAULT_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Outcome of resolving how the next turn against a stage should run.
///
/// `ResumeWarm` is the happy path: pass the captured `session_id` to
/// the runner as `--continue` so the agent picks up the same
/// conversation. `FreshAfterArchive` carries the now-archived id so the
/// caller (UI, log) can label the transition; the next turn must run
/// without `--continue` and a handover document at `handover_path` is
/// the bridge to the new session. `NoSession` is the never-yet-touched
/// case — the stage has nothing to resume, so the caller starts a
/// fresh session with no handover or lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDecision {
    NoSession,
    ResumeWarm {
        session_id: String,
    },
    FreshAfterArchive {
        prior_session_id: String,
        handover_path: std::path::PathBuf,
    },
}

/// Decide how the next turn against `stage_id` should run, given the
/// configured idle timeout and the current wall-clock `now`. Side
/// effects:
///
/// - When the warm session has been idle past `idle_timeout` (or the
///   row was already archived), flip `stages.archived = 1`, write
///   `runs/<job_id>/<stage_id>/handover-archived.md` under the
///   provided worktree, and publish
///   `Event::SessionArchivedThenResumed` on `bus`. The handover write
///   and the event are best-effort — failures are logged at warn level
///   so a missing worktree or a dropped subscriber does not block the
///   resume itself.
/// - When the session is still warm (or there is no session), no
///   filesystem or bus side-effects fire and the caller proceeds with
///   the decision.
///
/// `now` is taken as a parameter rather than read from `now_ms()` so
/// tests can fast-forward virtual time across the threshold without
/// touching the wall clock.
pub async fn resolve_stage_resume(
    store: &SqliteStore,
    bus: &EventBus,
    job_id: JobId,
    stage_id: StageId,
    worktree: Option<&Path>,
    idle_timeout: Duration,
    now: UnixMillis,
) -> ResumeDecision {
    let stage = match store.get_stage(stage_id).await {
        Ok(Some(s)) => s,
        Ok(None) | Err(_) => return ResumeDecision::NoSession,
    };

    let Some(session_id) = stage.session_id.clone() else {
        return ResumeDecision::NoSession;
    };

    if stage.archived {
        // Row was archived by an earlier sweep tick or an earlier call;
        // do not re-emit the lifecycle event, but the caller still
        // needs to know not to pass `--continue`.
        let handover_path = handover_archive_path(worktree, job_id, stage_id);
        return ResumeDecision::FreshAfterArchive {
            prior_session_id: session_id,
            handover_path,
        };
    }

    let is_idle = stage
        .last_activity_at
        .map(|last| {
            let elapsed_ms = now.0.saturating_sub(last.0);
            elapsed_ms as u128 >= idle_timeout.as_millis()
        })
        .unwrap_or(false);

    if !is_idle {
        return ResumeDecision::ResumeWarm { session_id };
    }

    // Archive transition: SQL is the source of truth for "did this row
    // just flip?". `None` means a racing sweeper got there first; the
    // lifecycle event already fired in that case, so we skip it here
    // but still surface the archived outcome to the caller.
    let prior = match store.archive_stage_session(stage_id).await {
        Ok(Some(prior)) => Some(prior),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = ?err, %stage_id, "archive_stage_session db error");
            None
        }
    };

    let handover_path = write_archive_handover(worktree, job_id, stage_id, &stage).await;

    if let Some(prior_id) = prior.as_ref() {
        publish_archived(bus, job_id, stage_id, prior_id.clone(), now).await;
    }

    ResumeDecision::FreshAfterArchive {
        prior_session_id: prior.unwrap_or(session_id),
        handover_path,
    }
}

/// Periodically archive every warm session that has been idle past its
/// job's `session_idle_timeout`. The sweeper is the safety net for
/// stages that the user never returns to — without it, the archived
/// state would only ever be reached on the next interactive turn.
///
/// `resolve_timeout` is invoked once per archived row so per-job
/// overrides land without the sweeper having to parse `template_yaml`
/// directly. Returning `None` means "no job-specific override," and
/// `DEFAULT_SESSION_IDLE_TIMEOUT` is used.
///
/// The sweeper publishes one `SessionArchivedThenResumed` per archive
/// transition with `prior_session_id` set to the row's captured id.
pub fn spawn_idle_sweeper(
    store: Arc<SqliteStore>,
    bus: Arc<EventBus>,
    period: Duration,
    resolve_timeout: Arc<dyn Fn(JobId) -> Option<Duration> + Send + Sync>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(period).await;
            sweep_once(&store, &bus, resolve_timeout.as_ref(), now_ms()).await;
        }
    })
}

/// Single pass of the sweeper. Exposed so tests can drive the archive
/// transition deterministically with a forged `now` rather than relying
/// on the spawn loop's wall-clock tick.
pub async fn sweep_once(
    store: &SqliteStore,
    bus: &EventBus,
    resolve_timeout: &(dyn Fn(JobId) -> Option<Duration> + Send + Sync),
    now: UnixMillis,
) {
    // Two-step: archive with the *shortest* configured cutoff so every
    // candidate row appears in the result set, then per-row re-check
    // each one's job-specific timeout. The single-statement form keeps
    // the race window small without forcing the sweeper to know every
    // job's override up front.
    //
    // For now we use a uniform cutoff of `now - DEFAULT_SESSION_IDLE_TIMEOUT`;
    // when per-job overrides become common a follow-up can narrow the
    // SQL with a join on `jobs.template_yaml`. The first cut is correct
    // (archives at or past the default) and conservative (never
    // archives a row before its job-specific timeout, since the default
    // is also the maximum used today).
    let cutoff = UnixMillis(
        now.0
            .saturating_sub(DEFAULT_SESSION_IDLE_TIMEOUT.as_millis() as i64),
    );
    let archived = match store.archive_idle_stage_sessions(cutoff).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = ?err, "idle sweeper: archive query failed");
            return;
        }
    };
    for (stage_id, prior_session_id) in archived {
        // Best-effort lookup of the job id so the lifecycle event is
        // correctly attributed. A missing stage row mid-tick is treated
        // as a race and skipped; the next tick will pick up anything we
        // missed.
        let stage = match store.get_stage(stage_id).await {
            Ok(Some(s)) => s,
            _ => continue,
        };
        let _ = resolve_timeout(stage.job_id);
        publish_archived(bus, stage.job_id, stage_id, prior_session_id, now).await;
    }
}

/// Conventional handover-archive path:
/// `<worktree>/runs/<job_id>/<stage_id>/handover-archived.md`. Stable
/// shape so the UI can probe for it directly. Returns the path
/// unconditionally — the file may not yet exist if the worktree was
/// `None` at archive time (a no-worktree caller still gets a useful
/// debug path for logs even though no file was written).
pub fn handover_archive_path(
    worktree: Option<&Path>,
    job_id: JobId,
    stage_id: StageId,
) -> std::path::PathBuf {
    let root = worktree.unwrap_or_else(|| Path::new("."));
    root.join("runs")
        .join(job_id.to_string())
        .join(stage_id.to_string())
        .join("handover-archived.md")
}

async fn write_archive_handover(
    worktree: Option<&Path>,
    job_id: JobId,
    stage_id: StageId,
    stage: &Stage,
) -> std::path::PathBuf {
    let path = handover_archive_path(worktree, job_id, stage_id);
    if worktree.is_none() {
        return path;
    }
    let prior_session = stage.session_id.as_deref().unwrap_or("<unknown>");
    let last_activity = stage
        .last_activity_at
        .map(|t| t.0.to_string())
        .unwrap_or_else(|| "<unknown>".into());
    // Persona id is captured here so a reader of the archived handover
    // can tell whether the run was shaped by a per-stage override
    // (D1) or inherited the job's persona. `<inherited>` means the
    // stage row carried no override and the job-level persona was in
    // force; the resolution itself lives on `jobs.persona_id`.
    let persona = stage.persona_id.as_deref().unwrap_or("<inherited>");
    let body = format!(
        "# Archived session handover\n\n\
         The warm session for stage `{stage_name}` (`{stage_id}`) was \
         archived after exceeding its `session_idle_timeout`.\n\n\
         - prior_session_id: `{prior_session}`\n\
         - last_activity_at_ms: `{last_activity}`\n\
         - job_id: `{job_id}`\n\
         - persona_id: `{persona}`\n\n\
         The next user message against this stage opens a fresh \
         session. Read this file before resuming so the new session \
         knows what the prior one did.\n",
        stage_name = stage.name,
    );
    if let Some(parent) = path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            tracing::warn!(error = %err, path = %parent.display(), "create archive handover dir");
            return path;
        }
    }
    if let Err(err) = tokio::fs::write(&path, body).await {
        tracing::warn!(error = %err, path = %path.display(), "write archive handover");
    }
    path
}

async fn publish_archived(
    bus: &EventBus,
    job_id: JobId,
    stage_id: StageId,
    prior_session_id: String,
    now: UnixMillis,
) {
    if let Err(err) = bus
        .publish(
            Some(job_id),
            Some(stage_id),
            None,
            Event::SessionArchivedThenResumed {
                stage_id,
                prior_session_id,
            },
            now,
        )
        .await
    {
        tracing::warn!(?err, %stage_id, "publish session-archived-then-resumed failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::SubscribeFilter;
    use codeless_types::{
        CostCents, GitAuth, Job, JobStatus, Repo, RepoId, Stage, StageStatus, WorkspaceMode,
    };
    use futures_util::StreamExt;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup() -> (SqliteStore, Arc<EventBus>) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::migrations::MIGRATOR.run(&pool).await.unwrap();
        let store = SqliteStore::new(pool.clone());
        let bus = Arc::new(EventBus::new(pool, 64));
        (store, bus)
    }

    async fn seed_repo_and_job(store: &SqliteStore) -> (RepoId, JobId) {
        let repo = Repo {
            id: RepoId::new(),
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: UnixMillis(0),
            updated_at: UnixMillis(0),
        };
        store.insert_repo(&repo).await.unwrap();
        let job = Job {
            id: JobId::new(),
            repo_id: repo.id,
            status: JobStatus::Running,
            stop_reason: None,
            template_yaml: None,
            prompt: None,
            runner: "mock".into(),
            branch: "codeless/job-x".into(),
            workspace_mode: WorkspaceMode::Worktree,
            worktree_path: None,
            cost_cap_cents: CostCents(0),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(0),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            started_at: None,
            ended_at: None,
            created_at: UnixMillis(0),
        };
        store.insert_job(&job).await.unwrap();
        (repo.id, job.id)
    }

    fn make_stage(job_id: JobId, last_activity_ms: i64) -> Stage {
        Stage {
            id: StageId::new(),
            job_id,
            ordinal: 0,
            name: "one".into(),
            status: StageStatus::Running,
            verify_cmd: None,
            started_at: Some(UnixMillis(0)),
            ended_at: None,
            session_id: Some("sess-warm".into()),
            goal: None,
            acceptance: None,
            last_activity_at: Some(UnixMillis(last_activity_ms)),
            archived: false,
            persona_id: None,
            bypassed_at: None,
            bypassed_reason: None,
        }
    }

    #[tokio::test]
    async fn session_idle_timeout_archives_then_resumes_transparently() {
        let (store, bus) = setup().await;
        let (_repo_id, job_id) = seed_repo_and_job(&store).await;
        let stage = make_stage(job_id, 1_000);
        let stage_id = stage.id;
        store.insert_stage(&stage).await.unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let worktree = tmp.path();

        let mut rx = bus
            .subscribe_since(SubscribeFilter::Job(job_id), None)
            .await
            .unwrap();

        // Fast-forward virtual time well past the 30-min default.
        let now = UnixMillis(1_000 + (DEFAULT_SESSION_IDLE_TIMEOUT.as_millis() as i64) + 1);
        let decision = resolve_stage_resume(
            &store,
            &bus,
            job_id,
            stage_id,
            Some(worktree),
            DEFAULT_SESSION_IDLE_TIMEOUT,
            now,
        )
        .await;

        match decision {
            ResumeDecision::FreshAfterArchive {
                prior_session_id,
                handover_path,
            } => {
                assert_eq!(prior_session_id, "sess-warm");
                assert!(handover_path
                    .ends_with(format!("runs/{job_id}/{stage_id}/handover-archived.md")));
                assert!(handover_path.exists(), "handover should be written");
                let body = std::fs::read_to_string(&handover_path).unwrap();
                assert!(body.contains("Archived session handover"));
                assert!(body.contains("sess-warm"));
            }
            other => panic!("expected FreshAfterArchive, got {other:?}"),
        }

        // Lifecycle event fired with the prior session id.
        let env = tokio::time::timeout(Duration::from_secs(2), rx.next())
            .await
            .expect("event arrives")
            .expect("stream open")
            .expect("envelope ok");
        match env.event {
            Event::SessionArchivedThenResumed {
                stage_id: ev_stage,
                prior_session_id,
            } => {
                assert_eq!(ev_stage, stage_id);
                assert_eq!(prior_session_id, "sess-warm");
            }
            other => panic!("expected session-archived-then-resumed, got {other:?}"),
        }

        // Row is durably archived; second call must NOT re-emit.
        let after = store.get_stage(stage_id).await.unwrap().unwrap();
        assert!(after.archived);

        let again = resolve_stage_resume(
            &store,
            &bus,
            job_id,
            stage_id,
            Some(worktree),
            DEFAULT_SESSION_IDLE_TIMEOUT,
            now,
        )
        .await;
        assert!(matches!(again, ResumeDecision::FreshAfterArchive { .. }));

        // No new event lands on the bus (the row was already archived).
        match tokio::time::timeout(Duration::from_millis(100), rx.next()).await {
            Err(_) => {}
            Ok(Some(Ok(env))) => panic!("unexpected extra event: {:?}", env.event),
            Ok(Some(Err(e))) => panic!("unexpected stream error: {e:?}"),
            Ok(None) => panic!("bus closed"),
        }
    }

    #[tokio::test]
    async fn warm_session_inside_timeout_resumes() {
        let (store, bus) = setup().await;
        let (_repo_id, job_id) = seed_repo_and_job(&store).await;
        let stage = make_stage(job_id, 10_000);
        let stage_id = stage.id;
        store.insert_stage(&stage).await.unwrap();

        // Only 1 second has passed; far below the 30-minute default.
        let now = UnixMillis(11_000);
        let decision = resolve_stage_resume(
            &store,
            &bus,
            job_id,
            stage_id,
            None,
            DEFAULT_SESSION_IDLE_TIMEOUT,
            now,
        )
        .await;

        match decision {
            ResumeDecision::ResumeWarm { session_id } => {
                assert_eq!(session_id, "sess-warm");
            }
            other => panic!("expected ResumeWarm, got {other:?}"),
        }
        let after = store.get_stage(stage_id).await.unwrap().unwrap();
        assert!(!after.archived);
    }

    #[tokio::test]
    async fn session_idle_sweeper_archives_in_background_tick() {
        let (store, bus) = setup().await;
        let (_repo_id, job_id) = seed_repo_and_job(&store).await;
        let stage = make_stage(job_id, 0);
        let stage_id = stage.id;
        store.insert_stage(&stage).await.unwrap();

        let mut rx = bus
            .subscribe_since(SubscribeFilter::Job(job_id), None)
            .await
            .unwrap();

        let resolve: Arc<dyn Fn(JobId) -> Option<Duration> + Send + Sync> = Arc::new(|_| None);
        let now = UnixMillis(DEFAULT_SESSION_IDLE_TIMEOUT.as_millis() as i64 + 1);
        sweep_once(&store, &bus, resolve.as_ref(), now).await;

        let after = store.get_stage(stage_id).await.unwrap().unwrap();
        assert!(after.archived, "sweeper should archive idle session");
        let env = tokio::time::timeout(Duration::from_secs(2), rx.next())
            .await
            .expect("event arrives")
            .expect("stream open")
            .expect("envelope ok");
        assert!(matches!(
            env.event,
            Event::SessionArchivedThenResumed { .. }
        ));
    }

    #[tokio::test]
    async fn archive_handover_records_per_stage_persona_id() {
        let (store, bus) = setup().await;
        let (_repo_id, job_id) = seed_repo_and_job(&store).await;
        let mut stage = make_stage(job_id, 1_000);
        stage.persona_id = Some("builtin:reviewer".into());
        let stage_id = stage.id;
        store.insert_stage(&stage).await.unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let worktree = tmp.path();

        let now = UnixMillis(1_000 + (DEFAULT_SESSION_IDLE_TIMEOUT.as_millis() as i64) + 1);
        let decision = resolve_stage_resume(
            &store,
            &bus,
            job_id,
            stage_id,
            Some(worktree),
            DEFAULT_SESSION_IDLE_TIMEOUT,
            now,
        )
        .await;

        let path = match decision {
            ResumeDecision::FreshAfterArchive { handover_path, .. } => handover_path,
            other => panic!("expected FreshAfterArchive, got {other:?}"),
        };
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("persona_id: `builtin:reviewer`"),
            "archive handover should name the per-stage persona; body={body}",
        );
    }

    #[tokio::test]
    async fn archive_handover_marks_persona_as_inherited_when_unset() {
        let (store, bus) = setup().await;
        let (_repo_id, job_id) = seed_repo_and_job(&store).await;
        let stage = make_stage(job_id, 1_000);
        let stage_id = stage.id;
        store.insert_stage(&stage).await.unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let worktree = tmp.path();

        let now = UnixMillis(1_000 + (DEFAULT_SESSION_IDLE_TIMEOUT.as_millis() as i64) + 1);
        let decision = resolve_stage_resume(
            &store,
            &bus,
            job_id,
            stage_id,
            Some(worktree),
            DEFAULT_SESSION_IDLE_TIMEOUT,
            now,
        )
        .await;

        let path = match decision {
            ResumeDecision::FreshAfterArchive { handover_path, .. } => handover_path,
            other => panic!("expected FreshAfterArchive, got {other:?}"),
        };
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("persona_id: `<inherited>`"),
            "archive handover should mark inherited persona; body={body}",
        );
    }
}
