//! `rerun_job` clones a previously-submitted job's row with a fresh
//! `JobId`, leaves the source untouched, and lands in `Draft` so the
//! user can review before starting.

use codeless_rpc::{AddRepoArgs, RerunJobArgs, RpcServer, SubmitJobArgs};
use codeless_runtime::InProcessRpc;
use codeless_types::{GitAuth, JobStatus};

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

#[tokio::test]
async fn rerun_job_clones_source_and_queues_fresh() {
    let rpc = InProcessRpc::new().await.unwrap();
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
    let source = rpc
        .submit_job(SubmitJobArgs {
            repo_id: repo.id,
            prompt: Some("list files".into()),
            template_yaml: None,
            runner: "mock".into(),
            branch: "feature/wizard-typed".into(),
            workspace_mode: None,
            cost_cap_cents: 500,
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

    let rerun = rpc
        .rerun_job(RerunJobArgs {
            source_job_id: source.id,
        })
        .await
        .unwrap();

    assert_ne!(rerun.id, source.id, "rerun must mint a new id");
    assert_eq!(rerun.repo_id, source.repo_id);
    assert_eq!(rerun.prompt, source.prompt);
    assert_eq!(rerun.template_yaml, source.template_yaml);
    assert_eq!(rerun.runner, source.runner);
    assert_eq!(rerun.cost_cap_cents, source.cost_cap_cents);
    assert_eq!(rerun.wall_clock_cap_ms, source.wall_clock_cap_ms);
    assert_eq!(rerun.status, JobStatus::Draft);
    assert_eq!(
        rerun.branch, "",
        "rerun starts with an empty branch so WorktreeManager picks the canonical fallback",
    );
    assert!(rerun.worktree_path.is_none());
    assert!(rerun.started_at.is_none());
    assert!(rerun.ended_at.is_none());

    let source_again = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id: source.id })
        .await
        .unwrap();
    assert_eq!(
        source_again.branch, "feature/wizard-typed",
        "source job branch must not be mutated by a rerun",
    );
}

#[tokio::test]
async fn rerun_unknown_job_is_not_found() {
    use codeless_rpc::RpcError;
    let rpc = InProcessRpc::new().await.unwrap();
    let err = rpc
        .rerun_job(RerunJobArgs {
            source_job_id: codeless_types::JobId::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
}
