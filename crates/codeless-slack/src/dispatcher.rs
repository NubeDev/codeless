//! Glue between Slack inbound messages and the `RpcServer` surface.
//!
//! One `Dispatcher` is built per `SlackBot` and shared across every
//! envelope the Socket Mode pump receives. The dispatch path is:
//!
//!   1. Decode the envelope's payload, pull out the channel, user,
//!      message text, and optional `thread_ts`.
//!   2. Resolve the thread-context job id via the [`ThreadMap`].
//!   3. Parse the message text into a [`Command`].
//!   4. Call the matching `RpcServer` method.
//!   5. Format the result and post it back into the same channel
//!      (and thread, when present) via [`ChatPoster`].
//!
//! The dispatcher itself is `RpcServer`-shaped via the
//! [`CommandBackend`] seam so tests can implement the five methods we
//! call without re-implementing the rest of the trait. Production
//! callers wrap an existing `Arc<dyn RpcServer>` via
//! [`RpcServerBackend`].
//!
//! Posting failures are logged and dropped on purpose. The runtime
//! has already advanced; refusing to ack the envelope because Slack
//! could not deliver our reply would just produce a duplicate dispatch
//! on Slack's retry — the wrong recovery for a transient post failure
//! against state the runtime has already mutated.

use std::sync::Arc;

use async_trait::async_trait;
use codeless_rpc::{
    error::RpcResult,
    methods::{
        GetJobArgs, ListJobsArgs, ListJobsResult, ListStagesArgs, ListStagesResult, ResumeJobArgs,
        StartJobArgs, StopJobArgs,
    },
    RpcServer,
};
use codeless_types::Job;
use serde::Deserialize;

use crate::command::{parse, Command, ParseError, ThreadContext};
use crate::reply;
use crate::thread_map::ThreadMap;
use crate::web_api::ChatPoster;

/// The slice of `RpcServer` the dispatcher actually calls. Defining a
/// smaller trait avoids forcing every Slack-side test to fake all ~80
/// methods on `RpcServer` and keeps the production blanket impl
/// transparently forwarded. The outbound failure publisher
/// ([`crate::outbound`]) also goes through this trait so test fakes
/// can drive both the inbound and outbound surfaces with one stub.
#[async_trait]
pub trait CommandBackend: Send + Sync + 'static {
    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult>;
    async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job>;
    async fn start_job(&self, args: StartJobArgs) -> RpcResult<Job>;
    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()>;
    async fn resume_job(&self, args: ResumeJobArgs) -> RpcResult<Job>;
    /// Used by the outbound failure publisher to resolve the failing
    /// stage's ordinal and title at notification time. Best-effort —
    /// the publisher falls back to a bare header when this fails
    /// rather than dropping the post entirely.
    async fn list_stages(&self, args: ListStagesArgs) -> RpcResult<ListStagesResult>;
}

/// Forward `CommandBackend` to any `RpcServer` implementor. The
/// in-process runtime and the loopback HTTP client both implement
/// `RpcServer`, so either can power the bot via this adapter.
pub struct RpcServerBackend {
    inner: Arc<dyn RpcServer>,
}

impl RpcServerBackend {
    pub fn new(inner: Arc<dyn RpcServer>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl CommandBackend for RpcServerBackend {
    async fn list_jobs(&self, args: ListJobsArgs) -> RpcResult<ListJobsResult> {
        self.inner.list_jobs(args).await
    }
    async fn get_job(&self, args: GetJobArgs) -> RpcResult<Job> {
        self.inner.get_job(args).await
    }
    async fn start_job(&self, args: StartJobArgs) -> RpcResult<Job> {
        self.inner.start_job(args).await
    }
    async fn stop_job(&self, args: StopJobArgs) -> RpcResult<()> {
        self.inner.stop_job(args).await
    }
    async fn resume_job(&self, args: ResumeJobArgs) -> RpcResult<Job> {
        self.inner.resume_job(args).await
    }
    async fn list_stages(&self, args: ListStagesArgs) -> RpcResult<ListStagesResult> {
        self.inner.list_stages(args).await
    }
}

/// Inbound message extracted from a Slack `events_api` envelope. Only
/// the fields the dispatcher cares about are surfaced here; the raw
/// envelope JSON is logged at trace level elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundMessage {
    pub channel: String,
    pub user: Option<String>,
    pub text: String,
    /// `Some(ts)` when the message arrived inside a thread the bot
    /// might have posted into. Top-level channel messages have
    /// `None` here.
    pub thread_ts: Option<String>,
}

/// Slack `events_api` envelope shape, projected onto only the fields
/// the dispatcher reads. Everything else (team id, event timestamps,
/// reactions sub-payloads, etc.) is ignored — `#[serde(other)]` is not
/// needed because serde's default already discards unknown fields.
#[derive(Debug, Deserialize)]
pub struct EnvelopePayload {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub envelope_id: Option<String>,
    pub payload: Option<EventsApiPayload>,
}

#[derive(Debug, Deserialize)]
pub struct EventsApiPayload {
    pub event: Option<EventBody>,
}

#[derive(Debug, Deserialize)]
pub struct EventBody {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel: Option<String>,
    pub user: Option<String>,
    pub text: Option<String>,
    pub thread_ts: Option<String>,
    /// Slack messages from the bot itself echo back with `bot_id` set;
    /// ignoring them prevents a reply loop where the bot answers its
    /// own confirmation.
    pub bot_id: Option<String>,
    /// Slack message subtypes (`bot_message`, `message_changed`,
    /// `message_deleted`, …) we do not want to dispatch on. The
    /// dispatcher skips any event with a non-empty subtype so an
    /// edit of a previously-typed command does not produce a second
    /// run.
    pub subtype: Option<String>,
}

/// Pull the dispatchable subset of an envelope. Returns `None` when
/// the envelope is not a dispatchable user message (bot echo, message
/// edit, hello frame, ack-only event). The caller still acks every
/// envelope by id; this function only decides whether to *run* the
/// command parser against the body.
pub fn extract_inbound(env: &EnvelopePayload) -> Option<InboundMessage> {
    if env.kind.as_deref() != Some("events_api") {
        return None;
    }
    let event = env.payload.as_ref()?.event.as_ref()?;
    // Two event types deliver operator commands: `app_mention` (the
    // bot was @-tagged in a channel) and `message` in a DM. Other
    // event kinds (reactions, channel joins, …) are ignored — the
    // SCOPE doc rules out reactions-as-decisions explicitly, and DM
    // message events are how Slack routes a direct conversation.
    if event.kind != "app_mention" && event.kind != "message" {
        return None;
    }
    if event.bot_id.is_some() {
        return None;
    }
    if event.subtype.is_some() {
        return None;
    }
    let channel = event.channel.clone()?;
    let text = event.text.clone()?;
    Some(InboundMessage {
        channel,
        user: event.user.clone(),
        text,
        thread_ts: event.thread_ts.clone(),
    })
}

/// Dispatcher core. Holds the backend trait object plus the poster
/// and thread map. Cheap to clone — every internal field is already
/// reference-counted.
#[derive(Clone)]
pub struct Dispatcher {
    backend: Arc<dyn CommandBackend>,
    poster: ChatPoster,
    threads: ThreadMap,
}

impl Dispatcher {
    pub fn new(backend: Arc<dyn CommandBackend>, poster: ChatPoster, threads: ThreadMap) -> Self {
        Self {
            backend,
            poster,
            threads,
        }
    }

    /// Drive one inbound envelope end-to-end. The Socket Mode pump
    /// calls this after acking; failures are logged and swallowed
    /// because there is no reasonable retry — Slack already has the
    /// ack, the runtime has already mutated state (or not), and
    /// asking the operator to retype the command is the only sound
    /// recovery for a Slack-side post failure.
    pub async fn dispatch_envelope(&self, env: &EnvelopePayload) {
        let Some(msg) = extract_inbound(env) else {
            return;
        };
        self.dispatch_message(msg).await;
    }

    /// Run a single already-extracted message through the parser and
    /// post the reply. Exposed for unit tests that bypass the
    /// envelope-decoding step.
    pub async fn dispatch_message(&self, msg: InboundMessage) {
        let reply_text = self.build_reply(&msg).await;
        // Reply in the same thread the command came from; if it was a
        // top-level message, post a top-level reply. The dispatcher
        // discards the returned `PostedMessage` — only the outbound
        // failure publisher needs the new message's `ts` (to register
        // a `ThreadMap` entry); a command reply lands either in an
        // existing thread (already mapped) or as a fresh top-level
        // post that the operator did not initiate from a notification
        // thread, so there is nothing to bind a future reply to.
        if let Err(err) = self
            .poster
            .post(&msg.channel, &reply_text, msg.thread_ts.as_deref())
            .await
        {
            tracing::warn!(
                channel = %msg.channel,
                error = %err,
                "slack: failed to post command reply",
            );
        }
    }

    /// Parse + dispatch + format. Pulled out of `dispatch_message`
    /// so tests can assert the reply text without faking a Slack
    /// poster.
    pub async fn build_reply(&self, msg: &InboundMessage) -> String {
        let ctx = self.thread_context(msg);
        let cmd = match parse(&msg.text, ctx) {
            Ok(c) => c,
            Err(ParseError::Empty) => return String::new(),
            Err(err) => return reply::format_parse_error(&err),
        };
        match self.run_command(cmd).await {
            Ok(text) => text,
            Err(err) => reply::format_rpc_error(&err),
        }
    }

    fn thread_context(&self, msg: &InboundMessage) -> ThreadContext {
        match msg.thread_ts.as_deref() {
            Some(ts) => match self.threads.lookup(&msg.channel, ts) {
                Some(job_id) => ThreadContext::for_job(job_id),
                None => ThreadContext::COLD,
            },
            None => ThreadContext::COLD,
        }
    }

    async fn run_command(&self, cmd: Command) -> RpcResult<String> {
        match cmd {
            Command::Help => Ok(reply::format_help()),
            Command::ListJobs => {
                let res = self
                    .backend
                    .list_jobs(ListJobsArgs { repo_id: None })
                    .await?;
                Ok(reply::format_list_jobs(&res))
            }
            Command::GetJob { job_id } => {
                let job = self.backend.get_job(GetJobArgs { job_id }).await?;
                Ok(reply::format_get_job(&job))
            }
            Command::StartJob { job_id } => {
                let job = self.backend.start_job(StartJobArgs { job_id }).await?;
                Ok(reply::format_start_job(&job))
            }
            Command::StopJob { job_id } => {
                // Best-effort name lookup before the stop so the reply
                // can echo the template name (Risk 1 in the SCOPE doc).
                // A `NotFound` on the pre-fetch still lets the dispatch
                // fall through to the real `stop_job` for the proper
                // error path.
                let pre = self.backend.get_job(GetJobArgs { job_id }).await.ok();
                self.backend.stop_job(StopJobArgs { job_id }).await?;
                let id_display = job_id.to_string();
                let name = pre.as_ref().and_then(reply::template_name);
                Ok(reply::format_stop_job(&id_display, name.as_deref()))
            }
            Command::ResumeJob {
                job_id,
                bypass,
                comment,
            } => {
                let job = self
                    .backend
                    .resume_job(ResumeJobArgs {
                        job_id,
                        additional_cost_cap_cents: None,
                        additional_wall_clock_cap_ms: None,
                        bypass,
                        next_stage_comment: comment.clone(),
                    })
                    .await?;
                Ok(reply::format_resume_job(&job, bypass, comment.as_deref()))
            }
        }
    }
}

/// Decode one Slack text frame into an `EnvelopePayload`. Returns
/// `Err` only for clearly malformed JSON; callers downcast the
/// `Option<...>` fields to decide whether to dispatch.
pub fn decode_envelope(text: &str) -> Result<EnvelopePayload, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeless_rpc::error::RpcError;
    use codeless_types::{CostCents, JobId, JobStatus, RepoId, WorkspaceMode};
    use std::sync::Mutex;

    fn sample_job(name: &str, status: JobStatus) -> Job {
        Job {
            id: JobId::new(),
            repo_id: RepoId::new(),
            status,
            stop_reason: None,
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

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
        // Pre-seeded behaviour: index into call name -> response.
        list_jobs: Mutex<Option<ListJobsResult>>,
        get_job: Mutex<Option<RpcResult<Job>>>,
        start_job: Mutex<Option<RpcResult<Job>>>,
        stop_job: Mutex<Option<RpcResult<()>>>,
        resume_job: Mutex<Option<RpcResult<Job>>>,
        last_resume: Mutex<Option<ResumeJobArgs>>,
    }

    impl FakeBackend {
        fn record(&self, label: &str) {
            self.calls.lock().unwrap().push(label.to_string());
        }
    }

    #[async_trait]
    impl CommandBackend for FakeBackend {
        async fn list_jobs(&self, _args: ListJobsArgs) -> RpcResult<ListJobsResult> {
            self.record("list_jobs");
            Ok(self
                .list_jobs
                .lock()
                .unwrap()
                .take()
                .unwrap_or(ListJobsResult { jobs: vec![] }))
        }
        async fn get_job(&self, _args: GetJobArgs) -> RpcResult<Job> {
            self.record("get_job");
            self.get_job
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Err(RpcError::NotFound("unset".into())))
        }
        async fn start_job(&self, _args: StartJobArgs) -> RpcResult<Job> {
            self.record("start_job");
            self.start_job
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Err(RpcError::NotFound("unset".into())))
        }
        async fn stop_job(&self, _args: StopJobArgs) -> RpcResult<()> {
            self.record("stop_job");
            self.stop_job.lock().unwrap().take().unwrap_or(Ok(()))
        }
        async fn resume_job(&self, args: ResumeJobArgs) -> RpcResult<Job> {
            self.record("resume_job");
            *self.last_resume.lock().unwrap() = Some(args);
            self.resume_job
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Err(RpcError::NotFound("unset".into())))
        }
        async fn list_stages(&self, _args: ListStagesArgs) -> RpcResult<ListStagesResult> {
            self.record("list_stages");
            Ok(ListStagesResult { stages: vec![] })
        }
    }

    fn dispatcher_with(backend: Arc<FakeBackend>) -> (Dispatcher, ThreadMap) {
        let http = Arc::new(reqwest::Client::new());
        // The poster is never actually called in build_reply tests
        // (which exercise the format path directly); a base_url
        // pointing at a non-listener is fine because the helper is
        // not invoked. dispatch_message uses a wiremock in the
        // dedicated tests below.
        let poster = ChatPoster::new(http, "xoxb-test").with_base_url("http://127.0.0.1:1/api");
        let threads = ThreadMap::new();
        (Dispatcher::new(backend, poster, threads.clone()), threads)
    }

    #[tokio::test]
    async fn list_jobs_command_calls_backend_and_formats() {
        let backend = Arc::new(FakeBackend::default());
        let job = sample_job("scope-mutable-ui", JobStatus::Failed);
        backend.list_jobs.lock().unwrap().replace(ListJobsResult {
            jobs: vec![job.clone()],
        });
        let (disp, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: Some("U1".into()),
                text: "<@U_BOT> status".into(),
                thread_ts: None,
            })
            .await;
        assert!(body.contains("scope-mutable-ui"), "got: {body}");
        assert_eq!(backend.calls.lock().unwrap().as_slice(), ["list_jobs"]);
    }

    #[tokio::test]
    async fn start_command_routes_to_start_job() {
        let backend = Arc::new(FakeBackend::default());
        let job = sample_job("hello-gin", JobStatus::Queued);
        backend.start_job.lock().unwrap().replace(Ok(job.clone()));
        let (disp, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: None,
                text: format!("start {}", job.id),
                thread_ts: None,
            })
            .await;
        assert!(body.starts_with("[ok]"), "got: {body}");
        assert!(body.contains("hello-gin"));
        assert_eq!(backend.calls.lock().unwrap().as_slice(), ["start_job"]);
    }

    #[tokio::test]
    async fn stop_in_thread_resolves_job_from_thread_map() {
        let backend = Arc::new(FakeBackend::default());
        let job = sample_job("smscope-smoke-2", JobStatus::Running);
        // get_job is the pre-fetch that lets the stop reply echo the
        // template name. stop_job itself returns ().
        backend.get_job.lock().unwrap().replace(Ok(job.clone()));
        let (disp, threads) = dispatcher_with(backend.clone());
        threads.record("C1", "1700.0001", job.id);
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: None,
                text: "stop".into(),
                thread_ts: Some("1700.0001".into()),
            })
            .await;
        assert!(body.contains("smscope-smoke-2"), "got: {body}");
        assert!(body.contains("[ok]"));
        // get_job is called before stop_job for the name echo.
        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            ["get_job", "stop_job"],
        );
    }

    #[tokio::test]
    async fn resume_with_bypass_and_comment_forwards_both_args() {
        let backend = Arc::new(FakeBackend::default());
        let job = sample_job("scope-mutable-ui", JobStatus::Queued);
        backend.resume_job.lock().unwrap().replace(Ok(job.clone()));
        let (disp, threads) = dispatcher_with(backend.clone());
        threads.record("C1", "1700.0002", job.id);
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: None,
                text: "resume bypass \"redo this stage; do not list the design doc\"".into(),
                thread_ts: Some("1700.0002".into()),
            })
            .await;
        assert!(body.contains("scope-mutable-ui"));
        assert!(body.contains("bypassed"));
        assert!(body.contains("redo this stage"));
        let recorded = backend.last_resume.lock().unwrap().clone().unwrap();
        assert!(recorded.bypass);
        assert_eq!(
            recorded.next_stage_comment.as_deref(),
            Some("redo this stage; do not list the design doc"),
        );
    }

    #[tokio::test]
    async fn parser_error_does_not_call_backend() {
        let backend = Arc::new(FakeBackend::default());
        let (disp, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: None,
                text: "dance now".into(),
                thread_ts: None,
            })
            .await;
        assert!(body.contains("[fail]"));
        assert!(body.contains("dance"));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_input_yields_empty_reply_and_no_call() {
        let backend = Arc::new(FakeBackend::default());
        let (disp, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: None,
                text: "   ".into(),
                thread_ts: None,
            })
            .await;
        // An empty / mention-only message is a Slack quirk; the bot
        // ignores it. A non-empty reply would spam the channel.
        assert!(body.is_empty());
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rpc_error_is_rendered_as_failure_line() {
        let backend = Arc::new(FakeBackend::default());
        backend
            .start_job
            .lock()
            .unwrap()
            .replace(Err(RpcError::Conflict("not in Draft".into())));
        let (disp, _) = dispatcher_with(backend.clone());
        let job_id = JobId::new();
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: None,
                text: format!("start {job_id}"),
                thread_ts: None,
            })
            .await;
        assert!(body.contains("[fail]"));
        assert!(body.contains("not in Draft"));
    }

    #[tokio::test]
    async fn cold_thread_with_no_mapping_falls_through_to_cold_grammar() {
        // A thread reply for which the bot has no mapping (process
        // restart, or a thread the bot did not post into) must
        // surface as `MissingJobId`, not a wrong-job dispatch.
        let backend = Arc::new(FakeBackend::default());
        let (disp, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                channel: "C1".into(),
                user: None,
                text: "resume".into(),
                thread_ts: Some("never-recorded".into()),
            })
            .await;
        assert!(body.contains("[fail]"), "got: {body}");
        assert!(body.contains("needs a job id"));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn extract_inbound_ignores_bot_echoes() {
        let raw = serde_json::json!({
            "type": "events_api",
            "envelope_id": "abc",
            "payload": {
                "event": {
                    "type": "message",
                    "channel": "C1",
                    "text": "hi",
                    "bot_id": "B1",
                },
            },
        });
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        assert!(extract_inbound(&env).is_none());
    }

    #[test]
    fn extract_inbound_ignores_subtyped_messages() {
        let raw = serde_json::json!({
            "type": "events_api",
            "envelope_id": "abc",
            "payload": {
                "event": {
                    "type": "message",
                    "channel": "C1",
                    "text": "old",
                    "subtype": "message_changed",
                },
            },
        });
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        assert!(extract_inbound(&env).is_none());
    }

    #[test]
    fn extract_inbound_picks_up_app_mention() {
        let raw = serde_json::json!({
            "type": "events_api",
            "envelope_id": "abc",
            "payload": {
                "event": {
                    "type": "app_mention",
                    "channel": "C1",
                    "user": "U1",
                    "text": "<@U_BOT> status",
                    "thread_ts": "1700.0001",
                },
            },
        });
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        let msg = extract_inbound(&env).expect("dispatchable");
        assert_eq!(msg.channel, "C1");
        assert_eq!(msg.user.as_deref(), Some("U1"));
        assert_eq!(msg.text, "<@U_BOT> status");
        assert_eq!(msg.thread_ts.as_deref(), Some("1700.0001"));
    }

    #[test]
    fn extract_inbound_ignores_non_events_api_envelopes() {
        let raw = serde_json::json!({"type": "hello"});
        let env: EnvelopePayload = serde_json::from_value(raw).unwrap();
        assert!(extract_inbound(&env).is_none());
    }

    #[tokio::test]
    async fn dispatch_message_posts_via_chat_post_message() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat.postMessage"))
            .and(body_partial_json(serde_json::json!({"channel": "C1"})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "ts": "1700.0001"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let backend = Arc::new(FakeBackend::default());
        let http = Arc::new(reqwest::Client::new());
        let poster = ChatPoster::new(http, "xoxb-test").with_base_url(server.uri() + "/api");
        let disp = Dispatcher::new(backend, poster, ThreadMap::new());
        disp.dispatch_message(InboundMessage {
            channel: "C1".into(),
            user: None,
            text: "help".into(),
            thread_ts: None,
        })
        .await;
    }
}
