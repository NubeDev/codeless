//! Transport-agnostic command dispatcher.
//!
//! One `Dispatcher` is built per chat adapter (`SlackBot`,
//! `TelegramBot`, …) and shared across every inbound message the
//! adapter's transport pump receives. The dispatch path is:
//!
//!   1. The adapter extracts the inbound message body from its
//!      transport's envelope shape and constructs an [`InboundMessage`].
//!   2. Resolve the thread-context job id via the [`ThreadMap`].
//!   3. Parse the message text into a [`Command`] (`@bot` mention
//!      stripping is the adapter's responsibility — different
//!      platforms use different mention syntaxes).
//!   4. Call the matching `RpcServer` method through the
//!      [`CommandBackend`] seam.
//!   5. Format the result with [`crate::reply`] and post it back via
//!      [`BotTransport::post`].
//!
//! The dispatcher itself is `RpcServer`-shaped via the
//! [`CommandBackend`] seam so tests can implement the five methods we
//! call without re-implementing the rest of the trait. Production
//! callers wrap an existing `Arc<dyn RpcServer>` via
//! [`RpcServerBackend`].
//!
//! Posting failures are logged and dropped on purpose. The runtime
//! has already advanced; refusing to ack the inbound envelope because
//! the chat platform could not deliver our reply would just produce a
//! duplicate dispatch on the platform's retry — the wrong recovery
//! for a transient post failure against state the runtime has already
//! mutated.

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

use crate::alias_map::AliasMap;
use crate::command::{parse, Command, ParseError, ThreadContext};
use crate::reply;
use crate::thread_map::ThreadMap;
use crate::transport::BotTransport;

/// Per-dispatcher configuration. Empty for now; held as a struct so
/// future fields (per-platform mention syntax, formatting flavour)
/// land without changing every adapter's call site.
#[derive(Debug, Default, Clone)]
pub struct DispatcherConfig {}

/// The slice of `RpcServer` the dispatcher actually calls. Defining a
/// smaller trait avoids forcing every adapter test to fake all ~80
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
/// `RpcServer`, so either can power a chat adapter via this seam.
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

/// Inbound message extracted from a transport-specific envelope. Only
/// the fields the dispatcher cares about are surfaced here; the raw
/// envelope JSON is the adapter's concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundMessage {
    /// Chat the message arrived in (Slack channel id, Telegram
    /// `chat_id` rendered as a string).
    pub chat: String,
    /// Platform-side identity of the sender, when the envelope
    /// surfaces one. Logged at trace level by the adapter; the
    /// dispatcher itself does not branch on it.
    pub user: Option<String>,
    /// Message body as the user typed it. The adapter is responsible
    /// for stripping the platform's `@bot` mention prefix before
    /// passing the text in.
    pub text: String,
    /// `Some(parent_id)` when the message arrived inside a thread
    /// the bot might have posted into (Slack `thread_ts`, Telegram
    /// `reply_to_message_id` / `message_thread_id`). Top-level chat
    /// messages have `None` here.
    pub reply_to: Option<String>,
}

/// Dispatcher core. Holds the backend trait object plus the transport
/// and thread map. Cheap to clone — every internal field is already
/// reference-counted.
#[derive(Clone)]
pub struct Dispatcher {
    backend: Arc<dyn CommandBackend>,
    transport: Arc<dyn BotTransport>,
    threads: ThreadMap,
    aliases: AliasMap,
}

impl Dispatcher {
    pub fn new(
        backend: Arc<dyn CommandBackend>,
        transport: Arc<dyn BotTransport>,
        threads: ThreadMap,
    ) -> Self {
        Self {
            backend,
            transport,
            threads,
            aliases: AliasMap::new(),
        }
    }

    /// Run a single already-extracted message through the parser and
    /// post the reply. Adapters call this after pulling
    /// [`InboundMessage`] out of their transport-specific envelope.
    pub async fn dispatch_message(&self, msg: InboundMessage) {
        let reply_text = self.build_reply(&msg).await;
        if reply_text.is_empty() {
            return;
        }
        // Reply in the same thread the command came from; if it was a
        // top-level message, post a top-level reply. The dispatcher
        // discards the returned post id — only the outbound failure
        // publisher needs the new message's id (to register a
        // `ThreadMap` entry); a command reply lands either in an
        // existing thread (already mapped) or as a fresh top-level
        // post that the operator did not initiate from a notification
        // thread, so there is nothing to bind a future reply to.
        if let Err(err) = self
            .transport
            .post(&msg.chat, &reply_text, msg.reply_to.as_deref())
            .await
        {
            tracing::warn!(
                chat = %msg.chat,
                error = %err,
                "bot: failed to post command reply",
            );
        }
    }

    /// Parse + dispatch + format. Pulled out of `dispatch_message`
    /// so tests can assert the reply text without a transport.
    pub async fn build_reply(&self, msg: &InboundMessage) -> String {
        let ctx = self.thread_context(msg);
        let expanded = self.expand_aliases(&msg.chat, &msg.text);
        let cmd = match parse(&expanded, ctx) {
            Ok(c) => c,
            Err(ParseError::Empty) => return String::new(),
            Err(err) => return reply::format_parse_error(&err),
        };
        match self.run_command(&msg.chat, cmd).await {
            Ok(text) => text,
            Err(err) => reply::format_rpc_error(&err),
        }
    }

    fn thread_context(&self, msg: &InboundMessage) -> ThreadContext {
        match msg.reply_to.as_deref() {
            Some(reply_key) => match self.threads.lookup(&msg.chat, reply_key) {
                Some(job_id) => ThreadContext::for_job(job_id),
                None => ThreadContext::COLD,
            },
            None => ThreadContext::COLD,
        }
    }

    /// Replace bare numeric tokens (1-99) with the full ULID from the
    /// alias map so the parser sees a valid job ID. Only the first
    /// token that looks like a small number and sits in the "id slot"
    /// is expanded — this avoids mangling quoted comments or other
    /// numeric values.
    fn expand_aliases(&self, chat: &str, text: &str) -> String {
        let trimmed = text.trim();
        let parts: Vec<&str> = trimmed.splitn(3, char::is_whitespace).collect();
        if parts.len() < 2 {
            return text.to_string();
        }
        let verb = parts[0].to_ascii_lowercase();
        let maybe_alias = parts[1].trim();
        if !matches!(
            verb.as_str(),
            "status" | "start" | "stop" | "resume"
        ) {
            return text.to_string();
        }
        if let Ok(n) = maybe_alias.parse::<usize>() {
            if n >= 1 && n <= 99 {
                if let Some(job_id) = self.aliases.resolve(chat, n) {
                    let rest = if parts.len() == 3 { parts[2] } else { "" };
                    if rest.is_empty() {
                        return format!("{} {}", parts[0], job_id);
                    }
                    return format!("{} {} {}", parts[0], job_id, rest);
                }
            }
        }
        text.to_string()
    }

    async fn run_command(&self, chat: &str, cmd: Command) -> RpcResult<String> {
        match cmd {
            Command::Help => Ok(reply::format_help()),
            Command::ListJobs => {
                let res = self
                    .backend
                    .list_jobs(ListJobsArgs { repo_id: None })
                    .await?;
                let ids: Vec<_> = res.jobs.iter().map(|j| j.id).collect();
                self.aliases.set(chat, ids);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{BotPostError, BotTransport, PostedMessage};
    use async_trait::async_trait;
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
            pending_operator_comment: None,
            precheck_override_once: false,
            started_at: None,
            ended_at: None,
            created_at: codeless_types::time::UnixMillis(0),
        }
    }

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
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

    /// Transport stub: records every post, fails nothing. `build_reply`
    /// tests never invoke the transport at all; the few that exercise
    /// `dispatch_message` use the recorded posts to assert ordering.
    #[derive(Default)]
    struct CapturingTransport {
        posts: Mutex<Vec<(String, String, Option<String>)>>,
    }

    #[async_trait]
    impl BotTransport for CapturingTransport {
        async fn post(
            &self,
            chat: &str,
            text: &str,
            reply_to: Option<&str>,
        ) -> Result<PostedMessage, BotPostError> {
            self.posts.lock().unwrap().push((
                chat.to_string(),
                text.to_string(),
                reply_to.map(String::from),
            ));
            Ok(PostedMessage {
                chat: chat.to_string(),
                id: "stub-id".to_string(),
            })
        }
    }

    fn dispatcher_with(
        backend: Arc<FakeBackend>,
    ) -> (Dispatcher, ThreadMap, Arc<CapturingTransport>) {
        let transport = Arc::new(CapturingTransport::default());
        let threads = ThreadMap::new();
        let disp = Dispatcher::new(backend, transport.clone(), threads.clone());
        (disp, threads, transport)
    }

    #[tokio::test]
    async fn list_jobs_command_calls_backend_and_formats() {
        let backend = Arc::new(FakeBackend::default());
        let job = sample_job("scope-mutable-ui", JobStatus::Failed);
        backend.list_jobs.lock().unwrap().replace(ListJobsResult {
            jobs: vec![job.clone()],
        });
        let (disp, _, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                chat: "C1".into(),
                user: Some("U1".into()),
                text: "<@U_BOT> status".into(),
                reply_to: None,
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
        let (disp, _, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                chat: "C1".into(),
                user: None,
                text: format!("start {}", job.id),
                reply_to: None,
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
        backend.get_job.lock().unwrap().replace(Ok(job.clone()));
        let (disp, threads, _) = dispatcher_with(backend.clone());
        threads.record("C1", "1700.0001", job.id);
        let body = disp
            .build_reply(&InboundMessage {
                chat: "C1".into(),
                user: None,
                text: "stop".into(),
                reply_to: Some("1700.0001".into()),
            })
            .await;
        assert!(body.contains("smscope-smoke-2"), "got: {body}");
        assert!(body.contains("[ok]"));
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
        let (disp, threads, _) = dispatcher_with(backend.clone());
        threads.record("C1", "1700.0002", job.id);
        let body = disp
            .build_reply(&InboundMessage {
                chat: "C1".into(),
                user: None,
                text: "resume bypass \"redo this stage; do not list the design doc\"".into(),
                reply_to: Some("1700.0002".into()),
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
        let (disp, _, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                chat: "C1".into(),
                user: None,
                text: "dance now".into(),
                reply_to: None,
            })
            .await;
        assert!(body.contains("[fail]"));
        assert!(body.contains("dance"));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_input_yields_empty_reply_and_no_call() {
        let backend = Arc::new(FakeBackend::default());
        let (disp, _, transport) = dispatcher_with(backend.clone());
        disp.dispatch_message(InboundMessage {
            chat: "C1".into(),
            user: None,
            text: "   ".into(),
            reply_to: None,
        })
        .await;
        // An empty / mention-only message is a platform quirk; the bot
        // ignores it. The transport must not be invoked — otherwise the
        // adapter would spam the chat with an empty post.
        assert!(transport.posts.lock().unwrap().is_empty());
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
        let (disp, _, _) = dispatcher_with(backend.clone());
        let job_id = JobId::new();
        let body = disp
            .build_reply(&InboundMessage {
                chat: "C1".into(),
                user: None,
                text: format!("start {job_id}"),
                reply_to: None,
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
        let (disp, _, _) = dispatcher_with(backend.clone());
        let body = disp
            .build_reply(&InboundMessage {
                chat: "C1".into(),
                user: None,
                text: "resume".into(),
                reply_to: Some("never-recorded".into()),
            })
            .await;
        assert!(body.contains("[fail]"), "got: {body}");
        assert!(body.contains("needs a job id"));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dispatch_message_invokes_transport_with_built_reply() {
        let backend = Arc::new(FakeBackend::default());
        let (disp, _, transport) = dispatcher_with(backend);
        disp.dispatch_message(InboundMessage {
            chat: "C1".into(),
            user: None,
            text: "help".into(),
            reply_to: None,
        })
        .await;
        let posts = transport.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, "C1");
        // The help text starts with the Surface 1 banner; anchoring on
        // it (rather than a specific verb line) keeps the test stable
        // against future re-wording of individual command rows.
        assert!(
            posts[0].1.contains("Codeless bot commands"),
            "got: {}",
            posts[0].1,
        );
        assert!(posts[0].2.is_none());
    }
}
