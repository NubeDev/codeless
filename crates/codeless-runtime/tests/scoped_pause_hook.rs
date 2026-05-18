//! End-to-end runtime-hook test for scoped pause points (stage 6).
//!
//! The hook is the layer above the existing `pause_job` primitive:
//! given a scheduled point row and a stage/todo transition that
//! matches it, the hook moves the job to `Paused`, fires `JobPaused`
//! with `StopReason::ScopedPausePoint { point_id }`, and cancels the
//! runner's token. The job is then resumable through the existing
//! `resume_job` RPC — this test pins the full pause -> resume cycle
//! so a future refactor cannot quietly break the "scheduling on top
//! of pause_job" property the SCOPE doc spells out.
//!
//! Why a hand-rolled hook call rather than spinning the full
//! `TemplateRunner`: the runner pulls in a real `template.yaml` with
//! claude-runner glue, which is overkill for proving the hook
//! semantics. The hook itself is the new code; the wiring in
//! `template_runner.rs` is a single call site that this test stands
//! in for by invoking `check_and_pause` directly against a seeded
//! schedule. Unit-level matcher tests cover the per-target match
//! logic; this file is the resume-cycle property test.

use codeless_rpc::{AddRepoArgs, ResumeJobArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::scoped_pause_hook::{check_and_pause, HookOutcome, TransitionPoint};
use codeless_runtime::{InProcessRpc, SubscribeFilter};
use codeless_types::pause_point::{PausePoint, PausePointId, PausePointPosition, PausePointTarget};
use codeless_types::{EventCursor, GitAuth, JobStatus, StopReason, UnixMillis};
use tokio_util::sync::CancellationToken;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed_running_job_with_schedule(
    rpc: &InProcessRpc,
    schedule: Vec<PausePoint>,
) -> codeless_types::Job {
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/demo".into(),
            git_auth: token_auth(),
            concurrency_cap: None,
            default_runner: Some("mock".into()),
        })
        .await
        .unwrap();
    let mut job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("scoped-pause smoke".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/scoped-pause-test".into(),
            workspace_mode: None,
            cost_cap_cents: 500,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            auto_bypass_policy: None,
            start_immediately: true,
        })
        .await
        .unwrap();
    // submit lands in Queued; the hook fires off a Running -> Paused
    // transition, so move the row to Running by hand to bypass the
    // driver's run loop.
    job.status = JobStatus::Running;
    job.started_at = Some(UnixMillis(0));
    rpc.store().update_job(&job).await.unwrap();

    rpc.store()
        .replace_scheduled_pause_points(job.id, &schedule, UnixMillis(0))
        .await
        .unwrap();

    job
}

/// A BeforeStage(2) point fires when the runner reaches stage 2's
/// pre-`StageStarted` transition. The hook writes Paused, emits
/// `JobPaused { reason: ScopedPausePoint }`, and the resume RPC then
/// promotes the row back to Queued — exactly what a manual
/// pause/resume cycle does, with the only observable difference being
/// the `StopReason` discriminator that the chat divider keys off in
/// stage 8.
#[tokio::test]
async fn before_stage_hook_pauses_then_resume_requeues() {
    let rpc = InProcessRpc::new().await.unwrap();
    let point_id = PausePointId::new();
    let schedule = vec![PausePoint {
        id: point_id,
        target: PausePointTarget::Stage { ordinal: 2 },
        position: PausePointPosition::Before,
        reason: Some("inspect stage 1 diff".into()),
    }];
    let job = seed_running_job_with_schedule(&rpc, schedule).await;

    let bus = rpc.bus().clone();
    let mut stream = bus
        .subscribe_since(SubscribeFilter::Job(job.id), Some(EventCursor(0)))
        .await
        .unwrap();

    let cancel = CancellationToken::new();
    let outcome = check_and_pause(
        rpc.store().as_ref(),
        rpc.bus().as_ref(),
        job.id,
        &TransitionPoint::BeforeStage { stage_ordinal: 2 },
        &cancel,
    )
    .await;
    assert_eq!(outcome, HookOutcome::Paused);
    assert!(
        cancel.is_cancelled(),
        "hook must fire the runner cancel token so the in-flight runner exits"
    );

    let after = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id: job.id })
        .await
        .unwrap();
    assert_eq!(after.status, JobStatus::Paused);
    assert_eq!(
        after.stop_reason,
        Some(StopReason::ScopedPausePoint { point_id }),
        "scoped pause must record the point_id on the row so the chat \
         divider can fetch the row's reason text on display"
    );
    assert!(after.ended_at.is_some(), "paused row records ended_at");

    // The hook publishes through the same EventBus path `pause_job`
    // uses; this assertion catches a regression where the hook
    // silently writes the row but skips the bus emit.
    use futures_util::StreamExt;
    let mut saw_paused = false;
    let drain = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while let Some(Ok(env)) = stream.next().await {
            if let codeless_types::Event::JobPaused {
                reason: StopReason::ScopedPausePoint { point_id: pid },
                ..
            } = env.event
            {
                assert_eq!(pid, point_id);
                saw_paused = true;
                break;
            }
        }
    })
    .await;
    drain.ok();
    assert!(
        saw_paused,
        "scoped pause must publish Event::JobPaused with ScopedPausePoint reason"
    );

    // Round-trip the row through resume_job to confirm the
    // ScopedPausePoint variant decodes the same way the row encoded
    // it — the codec path is what makes the variant durable across a
    // process restart, and a get_job after resume re-reads it.
    rpc.resume_job(ResumeJobArgs {
        job_id: job.id,
        additional_cost_cap_cents: None,
        additional_wall_clock_cap_ms: None,
        next_stage_comment: None,
        bypass: false,
    })
    .await
    .unwrap();
    let resumed = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id: job.id })
        .await
        .unwrap();
    assert_eq!(
        resumed.status,
        JobStatus::Queued,
        "resume_job promotes a scoped-pause row through Paused -> Queued, \
         identical to a user pause"
    );
    assert_eq!(
        resumed.stop_reason, None,
        "resume_job clears stop_reason on the row; previous_reason \
         rides on the JobResumed event instead"
    );
}

/// A second hook call against the same row after the first one
/// paused it must be a no-op — the `transition_job` guard rejects
/// `Paused -> Paused`, and the hook treats that as "nothing to do"
/// so a runner that races the cancel signal doesn't double-fire.
#[tokio::test]
async fn hook_is_idempotent_against_already_paused_row() {
    let rpc = InProcessRpc::new().await.unwrap();
    let point_id = PausePointId::new();
    let schedule = vec![PausePoint {
        id: point_id,
        target: PausePointTarget::Stage { ordinal: 1 },
        position: PausePointPosition::Before,
        reason: None,
    }];
    let job = seed_running_job_with_schedule(&rpc, schedule).await;

    let cancel = CancellationToken::new();
    let first = check_and_pause(
        rpc.store().as_ref(),
        rpc.bus().as_ref(),
        job.id,
        &TransitionPoint::BeforeStage { stage_ordinal: 1 },
        &cancel,
    )
    .await;
    assert_eq!(first, HookOutcome::Paused);

    // Same call again. The job is already Paused; the hook's
    // status-guard refuses the Paused -> Paused move and returns
    // without re-emitting or re-canceling. `transition_job` rejects
    // the move; the hook swallows the rejection as a no-op.
    let cancel2 = CancellationToken::new();
    let second = check_and_pause(
        rpc.store().as_ref(),
        rpc.bus().as_ref(),
        job.id,
        &TransitionPoint::BeforeStage { stage_ordinal: 1 },
        &cancel2,
    )
    .await;
    assert_eq!(
        second,
        HookOutcome::Continue,
        "a hook call against an already-Paused row must not re-fire \
         pause_job; the runner sees Continue and exits via the cancel \
         token that was already fired on the first call"
    );
}

/// A transition that does not match any scheduled point is a cheap
/// no-op. The matcher's "first row wins" rule means a single mismatch
/// must not consume the schedule — the next transition that does
/// match still fires.
#[tokio::test]
async fn non_matching_transition_does_not_pause() {
    let rpc = InProcessRpc::new().await.unwrap();
    let schedule = vec![PausePoint {
        id: PausePointId::new(),
        target: PausePointTarget::Stage { ordinal: 5 },
        position: PausePointPosition::After,
        reason: None,
    }];
    let job = seed_running_job_with_schedule(&rpc, schedule).await;

    let cancel = CancellationToken::new();
    let outcome = check_and_pause(
        rpc.store().as_ref(),
        rpc.bus().as_ref(),
        job.id,
        &TransitionPoint::BeforeStage { stage_ordinal: 2 },
        &cancel,
    )
    .await;
    assert_eq!(outcome, HookOutcome::Continue);
    assert!(!cancel.is_cancelled());

    let row = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id: job.id })
        .await
        .unwrap();
    assert_eq!(row.status, JobStatus::Running);
    assert_eq!(row.stop_reason, None);
}
