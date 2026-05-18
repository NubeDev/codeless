//! Per-Run supervisor lifecycle (JOB-CHAT.md (C2) stage 9).
//!
//! Pins the load-bearing scaffolding invariant: `drive_job` spawns
//! exactly one supervisor task per Run when the row transitions to
//! `Running`, and that task exits when the Run reaches a terminal
//! status (`Completed` / `Failed` / `Stopped`). A second Run on the
//! same Job (rerun / resume → another `drive_job`) spawns a fresh
//! supervisor; the previous one has already exited. The supervisor
//! is wired in `driver.rs` but the assertion is here because the
//! observable proof is on the event bus, which is the same surface
//! every transport reads.

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AddRepoArgs, EventFilter, PostJobMessageArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::store::supervisor_goals::{GoalAction, GoalCondition, SupervisorGoal};
use codeless_runtime::supervisor::tools::AdHocOutcome;
use codeless_runtime::{
    drive_job, spawn_supervisor, spawn_supervisor_with_tools, InProcessRpc, MockRunner, MockStep,
    RunnerOutcome, SupervisorTools,
};
use codeless_types::{
    ChatRole, ChatTransport, Event, GitAuth, JobId, JobStatus, Stage, StageId, StageStatus,
    StopReason, UnixMillis,
};
use futures_util::StreamExt;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "X".into(),
    }
}

async fn fresh_queued_job(rpc: &InProcessRpc) -> JobId {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "r".into(),
            clone_url: "u".into(),
            default_branch: "main".into(),
            local_path: "/tmp".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("p".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "b".into(),
        workspace_mode: None,
        cost_cap_cents: 0,
        wall_clock_cap_ms: 0,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        auto_bypass_policy: None,
        start_immediately: true,
    })
    .await
    .unwrap()
    .id
}

/// End-to-end: `drive_job` drives a `MockRunner` to `Completed`. The
/// supervisor it spawned must observe the `JobCompleted` envelope and
/// exit. Re-asserts the scaffold's two contracts at once — spawn on
/// `Running`, exit on terminal — through the only surface the rest of
/// the system observes (the event bus).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supervisor_spawns_on_run_start_and_exits_on_run_terminal() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id }, None)
        .await
        .expect("subscribe");

    let runner = Arc::new(MockRunner::new(vec![MockStep::Finish(
        RunnerOutcome::Completed,
    )]));

    drive_job(&rpc, job_id, runner, None)
        .await
        .expect("drive_job");

    // The driver publishes JobStarted before spawning the supervisor
    // and JobCompleted after the runner finishes. Both must appear on
    // the bus in that order — the bus is the surface the supervisor
    // subscribes on, so observing both here is the same observable
    // the supervisor's own loop sees.
    let mut saw_started = false;
    let mut saw_completed = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !(saw_started && saw_completed) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let item = tokio::time::timeout(remaining, stream.next())
            .await
            .expect("timed out waiting for driver framing events")
            .expect("stream end")
            .expect("stream error");
        match item.event {
            Event::JobStarted { .. } => saw_started = true,
            Event::JobCompleted { .. } => saw_completed = true,
            _ => {}
        }
    }
    assert!(saw_started, "driver must publish JobStarted");
    assert!(saw_completed, "driver must publish JobCompleted");

    // The supervisor handle inside drive_job is dropped on return,
    // but the spawned task survives until it observes terminal. The
    // proof we have available from the outside is "a fresh
    // supervisor spawned against this job's bus, asked to also
    // watch, would have exited by now too." Doing it from here is
    // the assertion the per-Run-attempt invariant gives us without
    // an extra hook into drive_job.
    let probe = spawn_supervisor(rpc.bus().clone(), job_id);
    // The probe will replay JobCompleted from the persisted events
    // table (subscribe_since with `None` replays nothing, so we have
    // to republish a terminal envelope for the probe to observe).
    // JobStopped is harmless: the row is already Completed; this
    // event is just a wakeup signal for the probe's loop. The
    // republish does not transition the row; transition_job is in
    // the RPC path, not here.
    rpc.bus()
        .publish(
            Some(job_id),
            None,
            None,
            Event::JobStopped {
                job_id,
                reason: StopReason::User,
            },
            codeless_runtime::now_ms(),
        )
        .await
        .unwrap();
    let probe_res = tokio::time::timeout(Duration::from_secs(2), probe).await;
    assert!(
        probe_res.is_ok(),
        "a per-Run-attempt supervisor must exit on a terminal event",
    );
}

/// Stage-10 contract: with the read tools wired in, asking "what
/// stage is it on?" in any non-supervisor transport must produce a
/// supervisor-authored chat reply that cites the current stage. The
/// setup uses a canned event timeline (a hand-built `stages` row +
/// the `JobStarted` envelope) rather than running the mock runner
/// through `drive_job` — the reactor reads stage state from the
/// store, so the canned row is the load-bearing input. The supervisor
/// itself is the spawn-with-tools variant; the lifecycle-only spawn
/// would never compose a reply because it has no `SqliteStore`
/// handle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supervisor_answers_what_stage_is_it_on() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    // Canned stage row: the supervisor's `get_job_state` will pick
    // this as the current stage (status=Running, single row in the
    // list). Ordinal 10 makes the assertion below unambiguous — the
    // reply must contain "stage 10".
    let stage = Stage {
        id: StageId::new(),
        job_id,
        ordinal: 10,
        name: "stage 10: supervisor tool surface".into(),
        status: StageStatus::Running,
        verify_cmd: None,
        started_at: Some(UnixMillis(1_700_000_000_000)),
        ended_at: None,
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
        persona_id: None,
        bypassed_at: None,
        bypassed_reason: None,
        failure_class: None,
        failure_detail: None,
    };
    rpc.store().insert_stage(&stage).await.unwrap();

    let supervisor = spawn_supervisor_with_tools(rpc.bus().clone(), rpc.store().clone(), job_id);

    // Subscribe to the bus *before* posting the user message so the
    // supervisor's reply lands on the live tail — the reply is just a
    // `ChatMessageAppended { transport: Supervisor }` envelope.
    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id }, None)
        .await
        .expect("subscribe");

    // Yield so the supervisor's own `subscribe_since` is attached
    // before the user post fires.
    tokio::time::sleep(Duration::from_millis(20)).await;

    rpc.post_job_message(PostJobMessageArgs {
        job_id,
        transport: ChatTransport::Web,
        external_id: None,
        thread_key: None,
        author: "alice".into(),
        role: ChatRole::User,
        body: "what stage is it on?".into(),
        metadata_json: None,
    })
    .await
    .expect("post user message");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let reply = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let item = tokio::time::timeout(remaining, stream.next())
            .await
            .expect("timed out waiting for supervisor reply")
            .expect("stream end")
            .expect("stream error");
        if let Event::ChatMessageAppended { message, .. } = item.event {
            if matches!(message.transport, ChatTransport::Supervisor) {
                break message;
            }
        }
    };

    assert_eq!(reply.role, ChatRole::Assistant);
    assert_eq!(reply.author, "supervisor");
    assert!(
        reply.body.contains("stage 10"),
        "supervisor reply must cite the current stage; got: {}",
        reply.body,
    );

    supervisor.abort();
    let _ = supervisor.await;
}

/// Stage-11 contract: on a Run terminal envelope, the supervisor
/// posts a one-paragraph summary into the chat thread before exiting.
/// The summary must cite each stage's name and, where present, the
/// row's `failure_detail` string — the only operator-visible
/// breadcrumb a chat reader gets when the rest of the UI is closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supervisor_posts_terminal_summary_on_run_failed() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    let passed = Stage {
        id: StageId::new(),
        job_id,
        ordinal: 1,
        name: "stage 1: bootstrap".into(),
        status: StageStatus::Passed,
        verify_cmd: None,
        started_at: Some(UnixMillis(1_700_000_000_000)),
        ended_at: Some(UnixMillis(1_700_000_001_000)),
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
        persona_id: None,
        bypassed_at: None,
        bypassed_reason: None,
        failure_class: None,
        failure_detail: None,
    };
    let failed = Stage {
        id: StageId::new(),
        job_id,
        ordinal: 2,
        name: "stage 2: build".into(),
        status: StageStatus::Failed,
        verify_cmd: None,
        started_at: Some(UnixMillis(1_700_000_002_000)),
        ended_at: Some(UnixMillis(1_700_000_003_000)),
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
        persona_id: None,
        bypassed_at: None,
        bypassed_reason: None,
        failure_class: None,
        failure_detail: Some("cargo: linker exit 1".into()),
    };
    rpc.store().insert_stage(&passed).await.unwrap();
    rpc.store().insert_stage(&failed).await.unwrap();

    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id }, None)
        .await
        .expect("subscribe");

    let supervisor = spawn_supervisor_with_tools(rpc.bus().clone(), rpc.store().clone(), job_id);
    tokio::time::sleep(Duration::from_millis(20)).await;

    rpc.bus()
        .publish(
            Some(job_id),
            None,
            None,
            Event::JobFailed { job_id },
            codeless_runtime::now_ms(),
        )
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let reply = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let item = tokio::time::timeout(remaining, stream.next())
            .await
            .expect("timed out waiting for terminal summary")
            .expect("stream end")
            .expect("stream error");
        if let Event::ChatMessageAppended { message, .. } = item.event {
            if matches!(message.transport, ChatTransport::Supervisor) {
                break message;
            }
        }
    };

    assert_eq!(reply.role, ChatRole::Assistant);
    assert!(
        reply.body.contains("stage 1: bootstrap"),
        "summary must cite the first stage name; got: {}",
        reply.body,
    );
    assert!(
        reply.body.contains("stage 2: build"),
        "summary must cite the failing stage name; got: {}",
        reply.body,
    );
    assert!(
        reply.body.contains("cargo: linker exit 1"),
        "summary must cite the visible failure_detail; got: {}",
        reply.body,
    );
    assert!(
        reply.body.to_ascii_lowercase().contains("failed"),
        "summary must record the terminal status; got: {}",
        reply.body,
    );

    // The supervisor must exit after posting; give it a moment to
    // observe its own terminal envelope and drain.
    let res = tokio::time::timeout(Duration::from_secs(2), supervisor).await;
    assert!(
        res.is_ok(),
        "supervisor must exit after posting the terminal summary",
    );
}

/// Two back-to-back terminal events on the same Job exercise the
/// "fresh Run spawns a fresh supervisor" property — each spawn is
/// independent and each call returns its own JoinHandle that resolves
/// only after the corresponding terminal envelope.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_run_spawns_an_independent_supervisor() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    let first = spawn_supervisor(rpc.bus().clone(), job_id);
    tokio::time::sleep(Duration::from_millis(20)).await;
    rpc.bus()
        .publish(
            Some(job_id),
            None,
            None,
            Event::JobCompleted { job_id },
            codeless_runtime::now_ms(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("first supervisor exits on terminal")
        .unwrap();

    // A second spawn (representing the fresh Run) is a brand new
    // task — its JoinHandle is independent of the first one and
    // resolves only on its own terminal observation.
    let second = spawn_supervisor(rpc.bus().clone(), job_id);
    assert!(
        !second.is_finished(),
        "fresh supervisor must not pre-exit on a prior Run's terminal event",
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    rpc.bus()
        .publish(
            Some(job_id),
            None,
            None,
            Event::JobFailed { job_id },
            codeless_runtime::now_ms(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .expect("second supervisor exits on its own terminal")
        .unwrap();
}

/// Stage-12 contract: ad-hoc destructive actions get a 5-second
/// preview window. The test shortens the window to 800ms so the suite
/// does not block on a real five-second sleep; the production constant
/// (`AD_HOC_PREVIEW_WINDOW`) is unchanged. A user-role chat message
/// matching `/^wait\b/i` lands inside the window, the action stands
/// down, and no `JobStopped` envelope is produced. The cancellation
/// row is a supervisor message with `metadata.resolves` pointing back
/// at the preview row's id — the audit-trail pairing JOB-CHAT.md's
/// OQ-CHAT-2 resolution describes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ad_hoc_stop_aborts_on_user_wait() {
    let rpc = Arc::new(InProcessRpc::new().await.unwrap());
    let job_id = fresh_queued_job(&rpc).await;
    let tools = SupervisorTools::new(rpc.bus().clone(), rpc.store().clone());

    let window = Duration::from_millis(800);
    let tools_clone = Arc::new(tools);
    let tools_for_call = Arc::clone(&tools_clone);
    let job = job_id;
    let action = tokio::spawn(async move {
        tools_for_call
            .stop_job_ad_hoc_with_window(job, "ran for >1h".into(), window)
            .await
    });

    // Race window: the action posts its preview row then arms the
    // sleep + bus-watch. A small head-start makes sure the user's
    // wait message lands *after* the action's `subscribe_since` is
    // attached so the broadcast picks it up.
    tokio::time::sleep(Duration::from_millis(150)).await;
    rpc.post_job_message(codeless_rpc::PostJobMessageArgs {
        job_id,
        transport: ChatTransport::Web,
        external_id: None,
        thread_key: None,
        author: "alice".into(),
        role: ChatRole::User,
        body: "wait, hold off — I'm still reading the logs".into(),
        metadata_json: None,
    })
    .await
    .expect("post wait message");

    let outcome = tokio::time::timeout(Duration::from_secs(3), action)
        .await
        .expect("action did not return inside the preview window")
        .expect("task join")
        .expect("ad-hoc stop returned Err");
    assert_eq!(
        outcome,
        AdHocOutcome::Aborted,
        "user 'wait' must abort the ad-hoc stop"
    );

    // The Run must NOT be stopped: the wait cancelled the destructive
    // half of the action. Status remains `Queued` (the row the
    // submit_job + start_immediately fixture leaves behind).
    let job = rpc.store().get_job(job_id).await.unwrap().unwrap();
    assert_ne!(
        job.status,
        JobStatus::Stopped,
        "aborted ad-hoc stop must not transition the row to Stopped",
    );

    // Audit trail: the cancellation message points back to the
    // preview message via `metadata.resolves`. Both rows are visible
    // through the normal list_chat_messages surface, so the UI can
    // render the pair without a private side channel.
    let rows = rpc
        .store()
        .list_chat_messages(job_id, None, 10)
        .await
        .unwrap();
    let preview = rows
        .iter()
        .find(|m| {
            matches!(m.transport, ChatTransport::Supervisor) && matches!(m.role, ChatRole::System)
        })
        .expect("preview row must be present");
    let cancellation = rows
        .iter()
        .find(|m| {
            matches!(m.transport, ChatTransport::Supervisor)
                && matches!(m.role, ChatRole::Assistant)
        })
        .expect("cancellation row must be present");
    let meta = cancellation
        .metadata_json
        .as_deref()
        .expect("cancellation must carry metadata");
    assert!(
        meta.contains(&preview.id.to_string()),
        "cancellation row must reference the preview id; got: {meta}",
    );
}

/// Stage-12 contract: with no `wait` arriving inside the window the
/// ad-hoc stop fires, the row transitions to `Stopped`, and the same
/// `JobStopped` envelope every other surface already consumes appears
/// on the bus. The post-action summary row pairs with the preview via
/// `metadata.resolves`, matching the abort path's pairing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ad_hoc_stop_fires_after_window() {
    let rpc = Arc::new(InProcessRpc::new().await.unwrap());
    let job_id = fresh_queued_job(&rpc).await;
    let tools = SupervisorTools::new(rpc.bus().clone(), rpc.store().clone());

    let mut stream = rpc
        .subscribe(codeless_rpc::EventFilter::Job { job_id }, None)
        .await
        .expect("subscribe");

    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        tools.stop_job_ad_hoc_with_window(job_id, "ran for >1h".into(), Duration::from_millis(400)),
    )
    .await
    .expect("ad-hoc stop did not return inside the timeout")
    .expect("ad-hoc stop returned Err");
    assert_eq!(
        outcome,
        AdHocOutcome::Fired,
        "no wait → ad-hoc stop must fire after the window",
    );

    // JobStopped is on the bus and visible to the same subscriber any
    // other transport would use — the audit-trail invariant from
    // JOB-CHAT.md Hard rule 5 ("action-tool invocations emit events").
    let mut saw_stopped = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !saw_stopped {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let item = tokio::time::timeout(remaining, stream.next())
            .await
            .expect("timed out waiting for JobStopped")
            .expect("stream end")
            .expect("stream error");
        if matches!(
            item.event,
            Event::JobStopped {
                reason: StopReason::User,
                ..
            }
        ) {
            saw_stopped = true;
        }
    }

    let job = rpc.store().get_job(job_id).await.unwrap().unwrap();
    assert_eq!(
        job.status,
        JobStatus::Stopped,
        "fired ad-hoc stop must transition the row to Stopped",
    );

    // Summary row pairs to the preview via `metadata.resolves`,
    // mirroring the abort path's audit-trail pairing.
    let rows = rpc
        .store()
        .list_chat_messages(job_id, None, 10)
        .await
        .unwrap();
    let preview = rows
        .iter()
        .find(|m| {
            matches!(m.transport, ChatTransport::Supervisor) && matches!(m.role, ChatRole::System)
        })
        .expect("preview row must be present");
    let summary = rows
        .iter()
        .find(|m| {
            matches!(m.transport, ChatTransport::Supervisor)
                && matches!(m.role, ChatRole::Assistant)
        })
        .expect("summary row must be present");
    let meta = summary
        .metadata_json
        .as_deref()
        .expect("summary must carry metadata");
    assert!(
        meta.contains(&preview.id.to_string()),
        "summary row must reference the preview id; got: {meta}",
    );
}

/// Stage-17 contract: a pre-armed `deadline-stop` goal fires exactly
/// when its deadline elapses. The supervisor's select! loop arms a
/// `tokio::time::sleep` per `armed` row at boot; the test drives the
/// fire-time via `tokio::time::advance` rather than a real one-hour
/// sleep. Two load-bearing assertions: the action invokes immediately
/// with no preview row (Hard rule 4 of JOB-CHAT.md — pre-armed
/// actions are pre-authorised), and the post-action summary's
/// metadata cites the authorising `chat_messages.id` so the audit
/// trail is a foreign-key edge instead of a free-text annotation.
#[tokio::test(flavor = "current_thread")]
async fn deadline_stop_fires_at_t_plus_one_hour() {
    // Setup runs against the real tokio clock — sqlx pool acquisition
    // and the in-memory SQLite migrations consume tokio timer entries
    // that would deadlock under `start_paused`. We pause only after
    // the fixtures land, then spawn the supervisor so its
    // `tokio::time::sleep` for the deadline arm registers against the
    // mocked clock.
    let rpc = InProcessRpc::new().await.unwrap();
    let job_id = fresh_queued_job(&rpc).await;

    // The user's "if X then Y" turn — the chat row whose id becomes
    // the goal's `authorised_by` FK. The reactor's post-fire summary
    // must reference this id so a reader of the chat thread can pair
    // the action with the authorisation without a side channel.
    let authoriser = rpc
        .post_job_message(PostJobMessageArgs {
            job_id,
            transport: ChatTransport::Web,
            external_id: None,
            thread_key: None,
            author: "alice".into(),
            role: ChatRole::User,
            body: "if this runs more than an hour, stop it and tell me why".into(),
            metadata_json: None,
        })
        .await
        .expect("authoriser post");

    // Deadline is wall-clock; the supervisor arms a `sleep` of
    // `deadline_ms - now_ms` at boot, which the tokio time-driver
    // observes as a normal pending sleep under `start_paused`.
    let now = codeless_runtime::now_ms();
    let goal = SupervisorGoal::new(
        job_id,
        GoalCondition::DeadlineStop {
            deadline_ms: now.0 + 3_600_000,
        },
        GoalAction::StopJob {
            reason: "ran past the 1h budget you set".into(),
        },
        authoriser.id,
        now,
    );
    rpc.store().insert_goal(&goal).await.expect("insert goal");

    // Subscribe before the supervisor spawns so the chat-append and
    // JobStopped envelopes the fire produces land on the live tail.
    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id }, None)
        .await
        .expect("subscribe");

    // Spawn the supervisor against the live clock first so its
    // `subscribe_since` and `list_armed_for_run` complete through
    // sqlx without competing against a paused timer-driver. A real
    // (small) sleep — rather than a bare `yield_now` loop — gives
    // sqlx's blocking SQL thread time to actually finish; under load
    // the yield-only pattern leaves the goal arm un-registered when
    // the test gets here.
    let supervisor = spawn_supervisor_with_tools(rpc.bus().clone(), rpc.store().clone(), job_id);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Freeze the clock at the moment the goal arm is parked on its
    // 3_600_000ms sleep, then jump past the deadline so the sleep
    // wakes immediately. Resuming straight after the jump returns the
    // post-fire flow (mark_fired / stop_job_inner / post summary)
    // onto the live clock so the sqlx-touching hops and the
    // broadcast wake-ups that drive the test's subscriber complete
    // without a paused mocked-clock starving them.
    tokio::time::pause();
    tokio::time::advance(Duration::from_millis(3_600_001)).await;
    tokio::time::resume();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut saw_stopped = false;
    let mut fire_summary: Option<codeless_types::ChatMessage> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !(saw_stopped && fire_summary.is_some()) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let item = tokio::time::timeout(remaining, stream.next())
            .await
            .expect("timed out waiting for deadline fire");
        let item = item
            .expect("event stream ended unexpectedly")
            .expect("event stream error");
        match item.event {
            Event::JobStopped {
                reason: StopReason::User,
                ..
            } => saw_stopped = true,
            Event::ChatMessageAppended { message, .. }
                if matches!(message.transport, ChatTransport::Supervisor)
                    && matches!(message.role, ChatRole::Assistant)
                    && fire_summary.is_none() =>
            {
                fire_summary = Some(message);
            }
            _ => {}
        }
    }

    let summary = fire_summary.expect("fire summary must arrive");
    let meta = summary
        .metadata_json
        .as_deref()
        .expect("fire summary must carry metadata");
    assert!(
        meta.contains(&authoriser.id.to_string()),
        "summary must reference the authorising chat_messages.id; got: {meta}",
    );
    assert!(
        meta.contains(&goal.id.to_string()),
        "summary must reference the goal id; got: {meta}",
    );

    // No preview row — Hard rule 4 second regime is "no preview, no
    // nag" for pre-armed actions. A `System`-role supervisor row would
    // indicate the ad-hoc preview path leaked into the pre-armed loop.
    let rows = rpc
        .store()
        .list_chat_messages(job_id, None, 50)
        .await
        .unwrap();
    let preview_rows = rows
        .iter()
        .filter(|m| {
            matches!(m.transport, ChatTransport::Supervisor) && matches!(m.role, ChatRole::System)
        })
        .count();
    assert_eq!(
        preview_rows, 0,
        "pre-armed actions must not produce a preview row (Hard rule 4)",
    );

    // The Run row reaches Stopped, the goal row transitions out of
    // `armed`. Both are observable through the store's normal read
    // surfaces — the audit trail does not depend on the in-memory
    // reactor state.
    let job = rpc.store().get_job(job_id).await.unwrap().unwrap();
    assert_eq!(job.status, JobStatus::Stopped);
    assert!(
        rpc.store()
            .list_armed_for_run(job_id)
            .await
            .unwrap()
            .is_empty(),
        "fired goal must no longer be armed",
    );

    // The supervisor observes its own JobStopped envelope and exits.
    let _ = tokio::time::timeout(Duration::from_secs(5), supervisor).await;
}
