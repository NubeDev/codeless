//! `EventBus::publish` rolls `AiMessageComplete.cost_cents` into the
//! affected task row and the owning job row. Asserts running totals
//! across a multi-message session.

use codeless_rpc::{AddRepoArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::{now_ms, InProcessRpc};
use codeless_types::{
    CostCents, Event, GitAuth, Stage, StageId, StageStatus, Task, TaskId, TaskStatus,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_message_complete_accumulates_cost_on_task_and_job() {
    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/unused".into(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    let job = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("x".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "codeless/job-cost".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            system_prompt: None,
            persona_id: None,
            start_immediately: true,
        })
        .await
        .unwrap();

    let store = rpc.store();
    let stage = Stage {
        id: StageId::new(),
        job_id: job.id,
        ordinal: 0,
        name: "s".into(),
        status: StageStatus::Pending,
        verify_cmd: None,
        started_at: None,
        ended_at: None,
        session_id: None,
        goal: None,
        acceptance: None,
        last_activity_at: None,
        archived: false,
    };
    store.insert_stage(&stage).await.unwrap();
    let task = Task {
        id: TaskId::new(),
        stage_id: stage.id,
        ordinal: 0,
        status: TaskStatus::Enqueued,
        depends_on: vec![],
        lease_holder: None,
        lease_expires_at: None,
        cost_cents: CostCents::ZERO,
        input_tokens: 0,
        output_tokens: 0,
        started_at: None,
        ended_at: None,
    };
    store.enqueue_task(&task).await.unwrap();

    let bus = rpc.bus();
    for cents in [3i64, 5, 2] {
        bus.publish(
            Some(job.id),
            Some(stage.id),
            Some(task.id),
            Event::AiMessageComplete {
                task_id: task.id,
                input_tokens: 100,
                output_tokens: 200,
                cost_cents: CostCents(cents),
            },
            now_ms(),
        )
        .await
        .unwrap();
    }

    let job_row = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id: job.id })
        .await
        .unwrap();
    assert_eq!(
        job_row.cost_cents.0, 10,
        "job cost is sum across all ai-message-complete envelopes"
    );

    let task_row = store.get_task(task.id).await.unwrap().expect("task row");
    assert_eq!(
        task_row.cost_cents.0, 10,
        "task cost mirrors the job rollup for a single-task session"
    );
}
