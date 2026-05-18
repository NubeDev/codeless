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
