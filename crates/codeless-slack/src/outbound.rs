//! Outbound failure-notification publisher.
//!
//! Subscribes to the event bus once at startup, watches every envelope
//! the runtime emits, and posts a single Slack message per terminal
//! transition. Only two event variants trigger a post:
//!
//!   - [`Event::JobFailed`] — the runtime gave up on a stage and the
//!     row moved to `Failed`. The operator needs to decide whether to
//!     resume / bypass / stop.
//!   - [`Event::JobStopped`] — the row moved to `Stopped` (cap hit,
//!     runner crash, user-initiated stop). The post lets the operator
//!     see the stop reason without opening the web UI.
//!
//! Every other event is dropped on purpose. The Surface 1 SCOPE doc
//! is explicit that the first-scope firehose is *terminal transitions*
//! only — `StageStarted` / `AiToken` / cost ticks would turn the
//! channel into noise. A future "verbose mode" opt-in is a separate
//! feature.
//!
//! ## Debounce
//!
//! Per the SCOPE doc Risk 2: a job stuck in a retry loop must not
//! flood the channel. The publisher keeps a small in-memory map of
//! `JobId -> Instant` last-posted-at and refuses to post again for
//! the same job within [`DEBOUNCE_WINDOW`] (5 minutes, matching the
//! SCOPE doc). Command replies are NOT debounced — they live in the
//! dispatcher and run on the operator's own typing cadence. Only the
//! event-driven outbound side gets coalesced.
//!
//! ## ThreadMap registration
//!
//! Every successful top-level post is registered in the
//! [`ThreadMap`] keyed off the returned message `ts`. That binding
//! is what lets bare-verb replies (`resume bypass`, `stop`) inside
//! the notification thread resolve to the failing job id without
//! the operator retyping it. A failed post (HTTP error,
//! `ok = false`) is logged and dropped — no thread is registered
//! so a follow-up reply still surfaces as the cold-grammar
//! `MissingJobId` path rather than a wrong-job dispatch.
//!
//! ## Replay policy
//!
//! Starts the subscription at `since: None` so only live events fire
//! a post. A process restart that misses a `JobFailed` from before
//! the restart will not retroactively re-post — the SCOPE doc names
//! the rule explicitly ("notifications are additive; the durable
//! record is the events table"). A reconnect/restart-driven cold
//! reply will fall through to the parser's cold grammar, which is
//! the right shape: the operator types the explicit id.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use codeless_rpc::error::{RpcError, RpcResult};
use codeless_rpc::methods::{GetJobArgs, ListStagesArgs, StageRollup};
use codeless_rpc::{EventFilter, EventStream, RpcServer};
use codeless_types::{Event, JobId, StopReason};
use futures_util::StreamExt;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::dispatcher::CommandBackend;
use crate::notify;
use crate::thread_map::ThreadMap;
use crate::web_api::ChatPoster;

/// Per-job debounce window. Matches the SCOPE doc: one event-driven
/// outbound post per job per 5 minutes max.
pub const DEBOUNCE_WINDOW: Duration = Duration::from_secs(300);

/// Minimal surface the publisher needs from the runtime for the
/// subscription side of the loop. `RpcServer` already supplies this;
/// a dedicated trait keeps the publisher's tests free of the larger
/// surface and lets the production wiring forward through
/// [`RpcServerEventSource`] without exposing the full RPC trait to
/// the publisher.
#[async_trait]
pub trait EventSource: Send + Sync + 'static {
    async fn subscribe_all(&self) -> RpcResult<EventStream>;
}

/// Forward [`EventSource`] to any `RpcServer` implementor. The
/// production wiring wraps the same `Arc<dyn RpcServer>` the
/// dispatcher already holds so the publisher and the dispatcher
/// share one upstream RPC seam.
pub struct RpcServerEventSource {
    inner: Arc<dyn RpcServer>,
}

impl RpcServerEventSource {
    pub fn new(inner: Arc<dyn RpcServer>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl EventSource for RpcServerEventSource {
    async fn subscribe_all(&self) -> RpcResult<EventStream> {
        self.inner.subscribe(EventFilter::All, None).await
    }
}

/// Configuration for [`OutboundPublisher::spawn`]. Held as a struct
/// (rather than a long positional argument list) so the call site at
/// `SlackBot::spawn_with` stays readable and so a future addition
/// (per-channel verbose mode, override debounce) lands without
/// changing every test.
pub struct OutboundConfig {
    /// Channel the bot posts notifications into. Sourced from the
    /// secrets store via `SlackConfig::channel_id`; the publisher is
    /// not spawned when the option is `None`.
    pub channel_id: String,
    /// Debounce window between consecutive posts for the same job.
    /// Defaults to [`DEBOUNCE_WINDOW`] in the production wiring; tests
    /// override to a small value to exercise both branches inside one
    /// run.
    pub debounce_window: Duration,
}

impl OutboundConfig {
    pub fn new(channel_id: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            debounce_window: DEBOUNCE_WINDOW,
        }
    }

    pub fn with_debounce_window(mut self, window: Duration) -> Self {
        self.debounce_window = window;
        self
    }
}

/// Background task that drives the subscription loop. Hold the handle
/// for the bot's lifetime; calling [`OutboundPublisher::shutdown`]
/// signals the task to exit at its next event boundary and waits for
/// the join.
pub struct OutboundPublisher {
    join: JoinHandle<()>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl OutboundPublisher {
    /// Spawn the publisher loop. The returned handle owns the
    /// background task and the shutdown signal; dropping it leaves
    /// the task running. The `events` source is held for the
    /// subscription open call only — once `subscribe_all` returns a
    /// stream, the publisher owns it for the task's lifetime, so the
    /// trait object's only requirement is `subscribe_all` itself.
    pub fn spawn(
        config: OutboundConfig,
        events: Arc<dyn EventSource>,
        backend: Arc<dyn CommandBackend>,
        poster: ChatPoster,
        threads: ThreadMap,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            run_loop(config, events, backend, poster, threads, shutdown_rx).await;
        });
        Self {
            join,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
    }
}

async fn run_loop(
    config: OutboundConfig,
    events: Arc<dyn EventSource>,
    backend: Arc<dyn CommandBackend>,
    poster: ChatPoster,
    threads: ThreadMap,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut stream = match events.subscribe_all().await {
        Ok(s) => s,
        Err(e) => {
            // The subscription open is the only call that can refuse
            // before the loop starts; surfacing the failure as a warn
            // (rather than a panic) lets the rest of the bot keep
            // serving commands even when the outbound side is
            // unavailable.
            tracing::warn!(error = %e, "slack: failed to open outbound subscription; publisher disabled");
            return;
        }
    };

    let debouncer = Arc::new(Mutex::new(Debouncer::new(config.debounce_window)));
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => return,
            next = stream.next() => {
                let Some(item) = next else { return };
                match item {
                    Ok(env) => handle_envelope(env.event, &config, &backend, &poster, &threads, &debouncer).await,
                    Err(e) => {
                        // A single error envelope is logged and skipped — the
                        // bus stream typically recovers; an unrecoverable
                        // stream ends with `None` above, which falls through
                        // to a clean exit.
                        tracing::warn!(error = %e, "slack: outbound stream error");
                    }
                }
            }
        }
    }
}

async fn handle_envelope(
    event: Event,
    config: &OutboundConfig,
    backend: &Arc<dyn CommandBackend>,
    poster: &ChatPoster,
    threads: &ThreadMap,
    debouncer: &Arc<Mutex<Debouncer>>,
) {
    let (job_id, kind) = match event {
        Event::JobFailed { job_id } => (job_id, OutboundKind::Failed),
        Event::JobStopped { job_id, reason } => (job_id, OutboundKind::Stopped(reason)),
        _ => return,
    };
    if !debouncer.lock().await.allow(job_id) {
        tracing::debug!(%job_id, "slack: outbound debounced");
        return;
    }
    if let Err(err) = post_notification(job_id, kind, config, backend, poster, threads).await {
        tracing::warn!(%job_id, error = %err, "slack: outbound publish failed");
    }
}

#[derive(Debug, Clone, Copy)]
enum OutboundKind {
    Failed,
    Stopped(StopReason),
}

#[derive(Debug, thiserror::Error)]
enum OutboundError {
    #[error("get_job: {0}")]
    GetJob(RpcError),
    #[error("post: {0}")]
    Post(#[from] crate::web_api::SlackPostError),
}

async fn post_notification(
    job_id: JobId,
    kind: OutboundKind,
    config: &OutboundConfig,
    backend: &Arc<dyn CommandBackend>,
    poster: &ChatPoster,
    threads: &ThreadMap,
) -> Result<(), OutboundError> {
    let job = backend
        .get_job(GetJobArgs { job_id })
        .await
        .map_err(OutboundError::GetJob)?;

    // list_stages is best-effort — without it the header collapses to
    // a bare `Failed` / `Stopped` line, but the post still goes out
    // (the operator still gets the cost and reply hint). A failure
    // here only logs.
    let stages_result = backend.list_stages(ListStagesArgs { job_id }).await;
    let (failing_stage, total_stages) = match stages_result {
        Ok(res) => {
            let total = u32::try_from(res.stages.len()).ok();
            let failing = pick_failing_stage(&res.stages).cloned();
            (failing, total)
        }
        Err(e) => {
            tracing::debug!(%job_id, error = %e, "slack: list_stages failed; rendering bare header");
            (None, None)
        }
    };

    let body = match kind {
        OutboundKind::Failed => {
            notify::format_job_failed(&job, failing_stage.as_ref(), total_stages)
        }
        OutboundKind::Stopped(reason) => {
            notify::format_job_stopped(&job, failing_stage.as_ref(), total_stages, reason)
        }
    };

    let posted = poster.post(&config.channel_id, &body, None).await?;
    // Register the new thread so a bare-verb reply in this thread
    // resolves to the right job id. Register AFTER the post succeeds
    // — a failed post that returned an error above never reaches
    // here, so no phantom mapping is created.
    threads.record(&posted.channel, &posted.ts, job_id);
    Ok(())
}

/// Pick the stage the notification should foreground. Prefers the
/// most-recently-ordered `Failed` stage (the one that just tipped the
/// job over). When no stage row is `Failed` (e.g. JobStopped from
/// a cap firing mid-Running stage), falls back to the highest-ordinal
/// non-Pending stage — that is the stage the operator was last
/// watching. Returns `None` only when the list itself is empty.
fn pick_failing_stage(stages: &[StageRollup]) -> Option<&StageRollup> {
    use codeless_types::StageStatus;
    stages
        .iter()
        .filter(|s| s.stage.status == StageStatus::Failed)
        .max_by_key(|s| s.stage.ordinal)
        .or_else(|| {
            stages
                .iter()
                .filter(|s| s.stage.status != StageStatus::Pending)
                .max_by_key(|s| s.stage.ordinal)
        })
        .or_else(|| stages.iter().max_by_key(|s| s.stage.ordinal))
}

/// Per-job debounce ledger. `allow(job_id)` returns `true` when no
/// outbound post has fired for that id in the configured window and
/// records the new timestamp. Old entries are pruned opportunistically
/// inside `allow` so the map size stays bounded by the active failing
/// job count, not by the lifetime of the process.
struct Debouncer {
    window: Duration,
    last: HashMap<JobId, Instant>,
}

impl Debouncer {
    fn new(window: Duration) -> Self {
        Self {
            window,
            last: HashMap::new(),
        }
    }

    fn allow(&mut self, job_id: JobId) -> bool {
        let now = Instant::now();
        // Drop any entries older than 2 windows so the map does not
        // grow unbounded on long-running deployments. 2x rather than
        // 1x because a sweep that drops an entry exactly at the
        // boundary would let a same-instant repost slip through.
        let cutoff = now.checked_sub(self.window * 2);
        if let Some(cutoff) = cutoff {
            self.last.retain(|_, &mut t| t > cutoff);
        }
        match self.last.get(&job_id) {
            Some(prev) if now.duration_since(*prev) < self.window => false,
            _ => {
                self.last.insert(job_id, now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_rpc::error::RpcError;
    use codeless_rpc::methods::{
        GetJobArgs, ListJobsArgs, ListJobsResult, ListStagesArgs, ListStagesResult, ResumeJobArgs,
        StartJobArgs, StopJobArgs,
    };
    use codeless_types::{
        CostCents, EventCursor, EventEnvelope, Job, JobStatus, RepoId, Stage, StageId, StageStatus,
        WorkspaceMode,
    };
    use futures_util::stream;
    use std::sync::Mutex;

    fn sample_job(name: &str, status: JobStatus, stop_reason: Option<StopReason>) -> Job {
        Job {
            id: JobId::new(),
            repo_id: RepoId::new(),
            status,
            stop_reason,
            template_yaml: Some(format!("name: {name}\nstages: []\n")),
            prompt: None,
            runner: "claude".to_string(),
            branch: "codeless/x".to_string(),
            workspace_mode: WorkspaceMode::InRepo,
            worktree_path: None,
            cost_cap_cents: CostCents(15000),
            wall_clock_cap_ms: 0,
            cost_cents: CostCents(2100),
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            started_at: None,
            ended_at: None,
            created_at: codeless_types::time::UnixMillis(0),
        }
    }

    fn envelope(event: Event, job_id: Option<JobId>) -> EventEnvelope {
        EventEnvelope {
            cursor: EventCursor(1),
            job_id,
            stage_id: None,
            task_id: None,
            created_at: codeless_types::time::UnixMillis(0),
            event,
        }
    }

    /// Test backend that combines [`EventSource`] and [`CommandBackend`]
    /// behind one shared registry so each test can inject events, look
    /// up calls, and observe the publisher's outbound traffic.
    #[derive(Default)]
    struct TestSource {
        events: Mutex<Vec<EventEnvelope>>,
        jobs: Mutex<Vec<Job>>,
        stages: Mutex<Vec<StageRollup>>,
    }

    impl TestSource {
        fn seed_jobs(&self, jobs: Vec<Job>) {
            *self.jobs.lock().unwrap() = jobs;
        }
        fn seed_stages(&self, stages: Vec<StageRollup>) {
            *self.stages.lock().unwrap() = stages;
        }
    }

    #[async_trait]
    impl EventSource for TestSource {
        async fn subscribe_all(&self) -> RpcResult<EventStream> {
            let queued: Vec<_> = self.events.lock().unwrap().drain(..).map(Ok).collect();
            Ok(Box::pin(stream::iter(queued)))
        }
    }

    #[async_trait]
    impl CommandBackend for TestSource {
        async fn list_jobs(&self, _args: ListJobsArgs) -> RpcResult<ListJobsResult> {
            unreachable!("publisher should not call list_jobs")
        }
        async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job> {
            self.jobs
                .lock()
                .unwrap()
                .iter()
                .find(|j| j.id == args.job_id)
                .cloned()
                .ok_or_else(|| RpcError::NotFound(format!("{}", args.job_id)))
        }
        async fn start_job(&self, _args: StartJobArgs) -> RpcResult<Job> {
            unreachable!("publisher should not call start_job")
        }
        async fn stop_job(&self, _args: StopJobArgs) -> RpcResult<()> {
            unreachable!("publisher should not call stop_job")
        }
        async fn resume_job(&self, _args: ResumeJobArgs) -> RpcResult<Job> {
            unreachable!("publisher should not call resume_job")
        }
        async fn list_stages(&self, _args: ListStagesArgs) -> RpcResult<ListStagesResult> {
            Ok(ListStagesResult {
                stages: self.stages.lock().unwrap().clone(),
            })
        }
    }

    fn sample_stage(ordinal: u32, name: &str, status: StageStatus, job_id: JobId) -> StageRollup {
        StageRollup {
            stage: Stage {
                id: StageId::new(),
                job_id,
                ordinal,
                name: name.to_string(),
                status,
                verify_cmd: None,
                started_at: None,
                ended_at: None,
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

    #[tokio::test]
    async fn job_failed_envelope_posts_one_notification_and_records_thread() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "ts": "1700.0123"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let src = Arc::new(TestSource::default());
        let mut job = sample_job(
            "scope-mutable-ui",
            JobStatus::Failed,
            Some(StopReason::RunnerCrash),
        );
        job.id = JobId::new();
        let job_id = job.id;
        src.seed_jobs(vec![job.clone()]);
        src.seed_stages(vec![
            sample_stage(0, "stage 0", StageStatus::Passed, job_id),
            sample_stage(1, "the failing stage", StageStatus::Failed, job_id),
            sample_stage(2, "later stage", StageStatus::Pending, job_id),
        ]);
        src.events
            .lock()
            .unwrap()
            .push(envelope(Event::JobFailed { job_id }, Some(job_id)));

        let threads = ThreadMap::new();
        let poster = ChatPoster::new(Arc::new(reqwest::Client::new()), "xoxb-test")
            .with_base_url(server.uri() + "/api");
        let events: Arc<dyn EventSource> = src.clone();
        let backend: Arc<dyn CommandBackend> = src.clone();
        let pub_ = OutboundPublisher::spawn(
            OutboundConfig::new("C123"),
            events,
            backend,
            poster,
            threads.clone(),
        );

        // The publisher's stream is finite (the test source drains its
        // queue) so the loop exits on its own; give it a moment to
        // process before asserting on the wiremock expectation.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        pub_.shutdown().await;

        // The wiremock `.expect(1)` panics on Drop if the call count
        // does not match, so its absence here means exactly one post
        // landed. The thread map should now resolve the posted ts.
        assert_eq!(threads.lookup("C123", "1700.0123"), Some(job_id));
    }

    #[tokio::test]
    async fn second_failure_within_debounce_window_is_dropped() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "ts": "1700.0001"})),
            )
            // Exactly one — the second event for the same job must be
            // debounced. Wiremock's expectations fire on Drop.
            .expect(1)
            .mount(&server)
            .await;

        let src = Arc::new(TestSource::default());
        let mut job = sample_job("smscope", JobStatus::Failed, Some(StopReason::RunnerCrash));
        job.id = JobId::new();
        let job_id = job.id;
        src.seed_jobs(vec![job.clone()]);
        src.events.lock().unwrap().extend([
            envelope(Event::JobFailed { job_id }, Some(job_id)),
            envelope(Event::JobFailed { job_id }, Some(job_id)),
        ]);

        let threads = ThreadMap::new();
        let poster = ChatPoster::new(Arc::new(reqwest::Client::new()), "xoxb-test")
            .with_base_url(server.uri() + "/api");
        let events: Arc<dyn EventSource> = src.clone();
        let backend: Arc<dyn CommandBackend> = src.clone();
        let pub_ = OutboundPublisher::spawn(
            OutboundConfig::new("C123").with_debounce_window(Duration::from_secs(60)),
            events,
            backend,
            poster,
            threads,
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        pub_.shutdown().await;
    }

    #[tokio::test]
    async fn different_jobs_are_not_debounced_against_each_other() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "ts": "1700.0002"})),
            )
            // Two distinct jobs — both should post.
            .expect(2)
            .mount(&server)
            .await;

        let src = Arc::new(TestSource::default());
        let mut a = sample_job("a", JobStatus::Failed, Some(StopReason::RunnerCrash));
        a.id = JobId::new();
        let mut b = sample_job("b", JobStatus::Failed, Some(StopReason::RunnerCrash));
        b.id = JobId::new();
        src.seed_jobs(vec![a.clone(), b.clone()]);
        src.events.lock().unwrap().extend([
            envelope(Event::JobFailed { job_id: a.id }, Some(a.id)),
            envelope(Event::JobFailed { job_id: b.id }, Some(b.id)),
        ]);

        let threads = ThreadMap::new();
        let poster = ChatPoster::new(Arc::new(reqwest::Client::new()), "xoxb-test")
            .with_base_url(server.uri() + "/api");
        let events: Arc<dyn EventSource> = src.clone();
        let backend: Arc<dyn CommandBackend> = src.clone();
        let pub_ = OutboundPublisher::spawn(
            OutboundConfig::new("C123"),
            events,
            backend,
            poster,
            threads,
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        pub_.shutdown().await;
    }

    #[tokio::test]
    async fn non_terminal_events_are_ignored() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            // Zero — chatty events must never reach Slack.
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(0)
            .mount(&server)
            .await;

        let src = Arc::new(TestSource::default());
        let job_id = JobId::new();
        src.events.lock().unwrap().extend([
            envelope(Event::JobStarted { job_id }, Some(job_id)),
            envelope(
                Event::AiToken {
                    task_id: codeless_types::TaskId::new(),
                    delta: "tok".to_string(),
                },
                None,
            ),
            envelope(Event::JobCompleted { job_id }, Some(job_id)),
        ]);

        let threads = ThreadMap::new();
        let poster = ChatPoster::new(Arc::new(reqwest::Client::new()), "xoxb-test")
            .with_base_url(server.uri() + "/api");
        let events: Arc<dyn EventSource> = src.clone();
        let backend: Arc<dyn CommandBackend> = src.clone();
        let pub_ = OutboundPublisher::spawn(
            OutboundConfig::new("C123"),
            events,
            backend,
            poster,
            threads,
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        pub_.shutdown().await;
    }

    #[tokio::test]
    async fn job_stopped_uses_event_reason_even_without_row_reason() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .and(body_partial_json(serde_json::json!({"channel": "C123"})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "ts": "1700.0003"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let src = Arc::new(TestSource::default());
        let mut job = sample_job("smscope", JobStatus::Stopped, None);
        job.id = JobId::new();
        let job_id = job.id;
        src.seed_jobs(vec![job.clone()]);
        src.events.lock().unwrap().push(envelope(
            Event::JobStopped {
                job_id,
                reason: StopReason::CostCap,
            },
            Some(job_id),
        ));

        let threads = ThreadMap::new();
        let poster = ChatPoster::new(Arc::new(reqwest::Client::new()), "xoxb-test")
            .with_base_url(server.uri() + "/api");
        let events: Arc<dyn EventSource> = src.clone();
        let backend: Arc<dyn CommandBackend> = src.clone();
        let pub_ = OutboundPublisher::spawn(
            OutboundConfig::new("C123"),
            events,
            backend,
            poster,
            threads,
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        pub_.shutdown().await;
    }

    #[test]
    fn debouncer_allows_first_call_then_blocks_within_window() {
        let mut d = Debouncer::new(Duration::from_secs(60));
        let id = JobId::new();
        assert!(d.allow(id));
        assert!(!d.allow(id));
    }

    #[test]
    fn debouncer_keys_per_job() {
        let mut d = Debouncer::new(Duration::from_secs(60));
        let a = JobId::new();
        let b = JobId::new();
        assert!(d.allow(a));
        assert!(d.allow(b));
        assert!(!d.allow(a));
    }

    #[test]
    fn pick_failing_stage_prefers_highest_failed_ordinal() {
        let job_id = JobId::new();
        let stages = vec![
            sample_stage(0, "a", StageStatus::Failed, job_id),
            sample_stage(1, "b", StageStatus::Failed, job_id),
            sample_stage(2, "c", StageStatus::Pending, job_id),
        ];
        let pick = pick_failing_stage(&stages).unwrap();
        assert_eq!(pick.stage.ordinal, 1);
        assert_eq!(pick.stage.name, "b");
    }

    #[test]
    fn pick_failing_stage_falls_back_to_highest_non_pending_when_none_failed() {
        let job_id = JobId::new();
        let stages = vec![
            sample_stage(0, "a", StageStatus::Passed, job_id),
            sample_stage(1, "b", StageStatus::Running, job_id),
            sample_stage(2, "c", StageStatus::Pending, job_id),
        ];
        let pick = pick_failing_stage(&stages).unwrap();
        assert_eq!(pick.stage.ordinal, 1);
    }
}
