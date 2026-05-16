//! `set_job_policy` mutates the per-job `auto_bypass_policy` column
//! and emits `JobPolicyChanged` for cross-window invalidation. Q5 in
//! `DOCS/AUTO-BYPASS-DECISIONS.md` pins the contract this exercises:
//! Running / Queued are refused with a distinct-wording `Conflict` so
//! a stage-failed handler cannot race the write; Draft / Stopped /
//! Paused are accepted; same-policy-set is a no-op success that
//! publishes no event.
//!
//! Holding the test in its own integration crate keeps the
//! per-`tokio::test` fresh-runtime cost separable from the bigger
//! pause/resume / driver-loop suites.

use codeless_rpc::{
    AddRepoArgs, GetJobArgs, PauseJobArgs, RpcError, RpcServer, SetJobPolicyArgs, SubmitJobArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_types::{AutoBypassPolicy, Event, EventCursor, GitAuth, JobId, JobStatus};
use tokio_stream::StreamExt;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn seed_draft_job(rpc: &InProcessRpc) -> codeless_types::Job {
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
    rpc.submit_job(SubmitJobArgs {
        repo_id: repo.id,
        prompt: Some("policy-able".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: "codeless/policy".into(),
        workspace_mode: None,
        cost_cap_cents: 500,
        wall_clock_cap_ms: 60_000,
        model: None,
        permission_mode: None,
        effort: None,
        system_prompt: None,
        persona_id: None,
        auto_bypass_policy: None,
        start_immediately: false,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn sets_policy_on_draft_and_publishes_event() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_draft_job(&rpc).await;
    assert!(job.auto_bypass_policy.is_none(), "draft starts unset");

    let bus = rpc.bus().clone();
    let mut stream = bus
        .subscribe_since(
            codeless_runtime::SubscribeFilter::Job(job.id),
            Some(EventCursor(0)),
        )
        .await
        .unwrap();

    rpc.set_job_policy(SetJobPolicyArgs {
        job_id: job.id,
        policy: Some(AutoBypassPolicy::Quick),
    })
    .await
    .unwrap();

    let after = rpc.get_job(GetJobArgs { job_id: job.id }).await.unwrap();
    assert_eq!(after.auto_bypass_policy, Some(AutoBypassPolicy::Quick));

    let mut saw = false;
    for _ in 0..32 {
        let item = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
        if let Ok(Some(Ok(env))) = item {
            if let Event::JobPolicyChanged {
                job_id,
                policy_name,
            } = env.event
            {
                assert_eq!(job_id, job.id);
                assert_eq!(policy_name.as_deref(), Some("Quick"));
                saw = true;
                break;
            }
        }
    }
    assert!(saw, "expected JobPolicyChanged on the bus");
}

#[tokio::test]
async fn clearing_policy_publishes_event_with_none_name() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_draft_job(&rpc).await;
    rpc.set_job_policy(SetJobPolicyArgs {
        job_id: job.id,
        policy: Some(AutoBypassPolicy::Cheap),
    })
    .await
    .unwrap();

    let bus = rpc.bus().clone();
    let mut stream = bus
        .subscribe_since(
            codeless_runtime::SubscribeFilter::Job(job.id),
            // Subscribe from "now" so the previous Cheap-set event
            // does not satisfy the assertion below.
            None,
        )
        .await
        .unwrap();

    rpc.set_job_policy(SetJobPolicyArgs {
        job_id: job.id,
        policy: None,
    })
    .await
    .unwrap();
    let after = rpc.get_job(GetJobArgs { job_id: job.id }).await.unwrap();
    assert!(after.auto_bypass_policy.is_none());

    let mut saw = false;
    for _ in 0..32 {
        let item = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await;
        if let Ok(Some(Ok(env))) = item {
            if let Event::JobPolicyChanged { policy_name, .. } = env.event {
                assert_eq!(policy_name, None);
                saw = true;
                break;
            }
        }
    }
    assert!(saw, "clear should still publish JobPolicyChanged");
}

#[tokio::test]
async fn same_policy_twice_is_noop_success_with_no_event() {
    let rpc = InProcessRpc::new().await.unwrap();
    let job = seed_draft_job(&rpc).await;
    rpc.set_job_policy(SetJobPolicyArgs {
        job_id: job.id,
        policy: Some(AutoBypassPolicy::LongTerm),
    })
    .await
    .unwrap();

    let bus = rpc.bus().clone();
    let mut stream = bus
        .subscribe_since(
            codeless_runtime::SubscribeFilter::Job(job.id),
            // Subscribe live so only the second (no-op) call could
            // possibly land on this stream.
            None,
        )
        .await
        .unwrap();

    // Second call with the same value: Ok, no event.
    rpc.set_job_policy(SetJobPolicyArgs {
        job_id: job.id,
        policy: Some(AutoBypassPolicy::LongTerm),
    })
    .await
    .unwrap();

    // Probe a short window — if anything is going to land it would
    // land immediately on the in-memory bus.
    let mut leaked = false;
    for _ in 0..4 {
        let item = tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await;
        if let Ok(Some(Ok(env))) = item {
            if matches!(env.event, Event::JobPolicyChanged { .. }) {
                leaked = true;
                break;
            }
        }
    }
    assert!(!leaked, "same-policy-set must not publish JobPolicyChanged");
}

#[tokio::test]
async fn rejects_running_with_distinct_wording() {
    let rpc = InProcessRpc::new().await.unwrap();
    let mut job = seed_draft_job(&rpc).await;
    job.status = JobStatus::Running;
    rpc.store().update_job(&job).await.unwrap();

    let err = rpc
        .set_job_policy(SetJobPolicyArgs {
            job_id: job.id,
            policy: Some(AutoBypassPolicy::JustCode),
        })
        .await
        .unwrap_err();
    match err {
        RpcError::Conflict(msg) => assert_eq!(
            msg, "job is Running; pause before changing the auto-bypass policy",
            "wording is contract — Surface F badge renders it verbatim"
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_queued_with_distinct_wording() {
    let rpc = InProcessRpc::new().await.unwrap();
    let mut job = seed_draft_job(&rpc).await;
    job.status = JobStatus::Queued;
    rpc.store().update_job(&job).await.unwrap();

    let err = rpc
        .set_job_policy(SetJobPolicyArgs {
            job_id: job.id,
            policy: Some(AutoBypassPolicy::BestJudgement),
        })
        .await
        .unwrap_err();
    match err {
        RpcError::Conflict(msg) => assert_eq!(
            msg,
            "job is Queued; pause before changing the auto-bypass policy"
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
}

#[tokio::test]
async fn accepts_paused_after_pause() {
    let rpc = InProcessRpc::new().await.unwrap();
    let mut job = seed_draft_job(&rpc).await;
    // Force Running so pause_job's transition is exercised end-to-end
    // (same trick the pause_job suite uses).
    job.status = JobStatus::Running;
    job.started_at = Some(codeless_types::UnixMillis(0));
    rpc.store().update_job(&job).await.unwrap();
    rpc.pause_job(PauseJobArgs { job_id: job.id })
        .await
        .unwrap();

    rpc.set_job_policy(SetJobPolicyArgs {
        job_id: job.id,
        policy: Some(AutoBypassPolicy::Custom {
            comment: "ship a one-line fix".into(),
        }),
    })
    .await
    .unwrap();

    let after = rpc.get_job(GetJobArgs { job_id: job.id }).await.unwrap();
    assert!(matches!(
        after.auto_bypass_policy,
        Some(AutoBypassPolicy::Custom { .. })
    ));
}

#[tokio::test]
async fn unknown_job_is_not_found() {
    let rpc = InProcessRpc::new().await.unwrap();
    let err = rpc
        .set_job_policy(SetJobPolicyArgs {
            // ULID-shaped synthetic id that has never been minted.
            job_id: JobId::new(),
            policy: Some(AutoBypassPolicy::Quick),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)));
}
