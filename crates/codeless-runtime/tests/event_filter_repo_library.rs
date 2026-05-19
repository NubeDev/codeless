//! Fan-out filter behaviour for `EventFilter::Repo` and
//! `EventFilter::Library`. Covers M5's invariant that two tabs
//! pointed at two attached workspaces never see each other's events:
//! the per-repo subscriber receives only its own job lifecycle (plus
//! library payloads that carry the matching `repo_id`), and library
//! subscribers receive events whose envelope `job_id` does not
//! resolve through `jobs` (the assistant / unbound-chat synthetic-id
//! family).

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AddRepoArgs, EventFilter, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, InProcessRpc, MockRunner, MockStep, RunnerOutcome};
use codeless_types::{CostCents, Event, GitAuth, JobId, RepoId, TaskId};
use futures_util::StreamExt;

fn token_auth() -> GitAuth {
    GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    }
}

async fn fresh_repo(rpc: &InProcessRpc, name: &str) -> RepoId {
    rpc.add_repo(AddRepoArgs {
        name: name.into(),
        clone_url: format!("https://example.test/{name}.git"),
        default_branch: "main".into(),
        local_path: format!("/tmp/{name}"),
        git_auth: token_auth(),
        concurrency_cap: None,
        default_runner: None,
    })
    .await
    .unwrap()
    .id
}

async fn fresh_job(rpc: &InProcessRpc, repo_id: RepoId, branch: &str) -> JobId {
    rpc.submit_job(SubmitJobArgs {
        repo_id,
        prompt: Some("x".into()),
        template_yaml: None,
        runner: "mock".into(),
        branch: branch.into(),
        workspace_mode: None,
        cost_cap_cents: 10_000,
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
    .unwrap()
    .id
}

async fn collect_for(
    stream: &mut codeless_rpc::EventStream,
    window: Duration,
) -> Vec<codeless_types::EventEnvelope> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return out;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(env))) => out.push(env),
            Ok(Some(Err(e))) => panic!("stream error: {e}"),
            Ok(None) => return out,
            Err(_) => return out,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repo_filter_partitions_two_repos_and_library_catches_synthetic_ids() {
    let rpc = InProcessRpc::new().await.unwrap();
    let repo_a = fresh_repo(&rpc, "alpha").await;
    let repo_b = fresh_repo(&rpc, "beta").await;

    // Subscribers open *before* the jobs exist. Repo subscribers must
    // pick up `JobQueued` from the live tail (the job-repo map is
    // empty at subscribe time and JobQueued carries `repo_id` on the
    // wire, so the fan-out matches via payload — and the map update
    // arms the per-job downstream events that follow).
    let mut sub_a = rpc
        .subscribe(EventFilter::Repo { repo_id: repo_a }, None)
        .await
        .unwrap();
    let mut sub_b = rpc
        .subscribe(EventFilter::Repo { repo_id: repo_b }, None)
        .await
        .unwrap();
    let mut sub_lib = rpc.subscribe(EventFilter::Library, None).await.unwrap();

    let job_a = fresh_job(&rpc, repo_a, "codeless/job-a").await;
    let job_b = fresh_job(&rpc, repo_b, "codeless/job-b").await;

    // Drive each job through the mock runner. The emitted
    // `AiMessageComplete` has the per-job envelope `job_id` but no
    // payload `repo_id`, so the fan-out has to consult the
    // `job -> repo` map that the live tail folded in from
    // `JobQueued` above. A miss here is the bug this stage exists to
    // prevent.
    let runner_a = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::AiMessageComplete {
            task_id: TaskId::new(),
            input_tokens: 1,
            output_tokens: 1,
            cost_cents: CostCents(1),
        }),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));
    let runner_b = Arc::new(MockRunner::new(vec![
        MockStep::Emit(Event::AiMessageComplete {
            task_id: TaskId::new(),
            input_tokens: 2,
            output_tokens: 2,
            cost_cents: CostCents(2),
        }),
        MockStep::Finish(RunnerOutcome::Completed),
    ]));
    drive_job(&rpc, job_a, runner_a, None).await.unwrap();
    drive_job(&rpc, job_b, runner_b, None).await.unwrap();

    // Synthetic-id publish: the assistant / unbound-chat surface
    // stuffs a thread / session id into the envelope's `job_id` slot
    // (DOCS/EVENT-PUBLISH-AUDIT.md "Family B / Family C"). The id
    // does not resolve through `jobs`, so the per-repo subscribers
    // must drop it and the library subscriber must pick it up.
    let synthetic = JobId::new();
    rpc.bus()
        .publish(
            Some(synthetic),
            None,
            None,
            Event::AssistantThreadTouched {
                thread_id: codeless_types::AssistantThreadId::new(),
            },
            codeless_runtime::now_ms(),
        )
        .await
        .unwrap();

    let events_a = collect_for(&mut sub_a, Duration::from_millis(400)).await;
    let events_b = collect_for(&mut sub_b, Duration::from_millis(400)).await;
    let events_lib = collect_for(&mut sub_lib, Duration::from_millis(400)).await;

    // Repo A sees its own `JobQueued` + its own `AiMessageComplete`,
    // and never sees anything from job B.
    let job_ids_a: Vec<Option<JobId>> = events_a.iter().map(|e| e.job_id).collect();
    assert!(
        job_ids_a
            .iter()
            .all(|jid| *jid == Some(job_a) || jid.is_none()),
        "repo-A subscriber leaked cross-repo envelopes: {job_ids_a:?}",
    );
    assert!(
        events_a
            .iter()
            .any(|e| matches!(e.event, Event::JobQueued { repo_id, .. } if repo_id == repo_a)),
        "repo-A subscriber missed JobQueued for its own job",
    );
    assert!(
        events_a
            .iter()
            .any(|e| matches!(e.event, Event::AiMessageComplete { .. })),
        "repo-A subscriber missed the downstream runner event \
         (job_repo map did not arm from JobQueued)",
    );
    assert!(
        !events_a
            .iter()
            .any(|e| matches!(e.event, Event::AssistantThreadTouched { .. })),
        "repo-A subscriber leaked a synthetic-id (library) envelope",
    );

    // Repo B's slice: same shape, opposite repo.
    let job_ids_b: Vec<Option<JobId>> = events_b.iter().map(|e| e.job_id).collect();
    assert!(
        job_ids_b
            .iter()
            .all(|jid| *jid == Some(job_b) || jid.is_none()),
        "repo-B subscriber leaked cross-repo envelopes: {job_ids_b:?}",
    );
    assert!(
        events_b
            .iter()
            .any(|e| matches!(e.event, Event::JobQueued { repo_id, .. } if repo_id == repo_b)),
        "repo-B subscriber missed JobQueued for its own job",
    );

    // Library catches the assistant touch and no per-job lifecycle.
    assert!(
        events_lib
            .iter()
            .any(|e| matches!(e.event, Event::AssistantThreadTouched { .. })),
        "library subscriber missed the assistant touch",
    );
    assert!(
        !events_lib
            .iter()
            .any(|e| matches!(e.event, Event::JobQueued { .. })),
        "library subscriber leaked a repo-tagged JobQueued",
    );
    assert!(
        !events_lib
            .iter()
            .any(|e| matches!(e.event, Event::AiMessageComplete { .. })),
        "library subscriber leaked a per-job runner event",
    );
}
