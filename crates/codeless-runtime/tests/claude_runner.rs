//! End-to-end exercise of `ClaudeRunnerAdapter` against a fake
//! `claude` binary. Pins SCOPE.md "Testing strategy" — CLI runners
//! are tested against a stubbed binary on an explicit path, never
//! the developer's host install. The fake emits the same NDJSON
//! shape the real `claude` CLI does under `--output-format stream-json`
//! so the entire chain runs: ai-runner parses NDJSON →
//! `ai_runner::Event`s → `ai_runner_bridge::forward_events` →
//! `codeless_types::Event` on the codeless `EventBus`.

use std::sync::Arc;
use std::time::Duration;

use codeless_rpc::{AddRepoArgs, EventFilter, RpcServer, SubmitJobArgs};
use codeless_runtime::{drive_job, ClaudeRunnerAdapter, InProcessRpc};
use codeless_types::{Event, GitAuth, JobStatus, TaskId};
use futures_util::StreamExt;
use tempfile::TempDir;

const FAKE_CLAUDE: &str = r#"#!/usr/bin/env bash
# Stand-in for the `claude` CLI used by codeless tests. The real
# binary emits one NDJSON line per stream event under
# `--output-format stream-json`; this fake replays a hand-written
# transcript chosen to exercise system / assistant / result handling.
cat <<'JSON'
{"type":"system","subtype":"init","session_id":"sess-fake","model":"claude-opus-4-5"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello "}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"world"}]}}
{"type":"result","subtype":"success","total_cost_usd":0.0123,"session_id":"sess-fake"}
JSON
"#;

fn install_fake_claude(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-claude");
    std::fs::write(&path, FAKE_CLAUDE).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

async fn collect_until<F>(stream: &mut codeless_rpc::EventStream, mut done: F) -> Vec<Event>
where
    F: FnMut(&Event) -> bool,
{
    let fut = async {
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            let env = item.expect("stream error");
            let stop = done(&env.event);
            out.push(env.event);
            if stop {
                return out;
            }
        }
        out
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("timed out waiting for terminal event")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_runner_streams_events_via_bridge() {
    let fake_dir = TempDir::new().unwrap();
    let fake = install_fake_claude(fake_dir.path());

    // CLAUDE_BINARY honours the explicit override (`ai_runner::runners::
    // claude::discover_claude_binary`). Set + remove in one test means
    // no other test in this binary races us; a separate `#[test]` would
    // need `serial_test` or its own process.
    std::env::set_var("CLAUDE_BINARY", &fake);

    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: "/tmp/codeless-demo-not-used".into(),
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
            prompt: Some("hi".into()),
            template_yaml: None,
            runner: "claude".into(),
            branch: "codeless/job-claude".into(),
            workspace_mode: None,
            cost_cap_cents: 500,
            wall_clock_cap_ms: 60_000,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: true,
        })
        .await
        .unwrap();

    let mut stream = rpc
        .subscribe(EventFilter::Job { job_id: job.id }, None)
        .await
        .unwrap();

    let task_id = TaskId::new();
    let adapter: Arc<dyn codeless_runtime::Runner> =
        Arc::new(ClaudeRunnerAdapter::new("hi", task_id));

    drive_job(&rpc, job.id, adapter, None).await.unwrap();
    std::env::remove_var("CLAUDE_BINARY");

    let events = collect_until(&mut stream, |e| matches!(e, Event::JobCompleted { .. })).await;

    let mut text = String::new();
    let mut saw_complete = false;
    let mut saw_started = false;
    let mut saw_job_completed = false;
    for ev in &events {
        match ev {
            Event::JobStarted { .. } => saw_started = true,
            Event::AiToken { task_id: t, delta } if *t == task_id => text.push_str(delta),
            Event::AiMessageComplete {
                task_id: t,
                cost_cents,
                ..
            } if *t == task_id => {
                saw_complete = true;
                assert_eq!(cost_cents.0, 1, "0.0123 USD rounds to 1 cent");
            }
            Event::JobCompleted { .. } => saw_job_completed = true,
            _ => {}
        }
    }
    assert!(saw_started, "missing JobStarted; got {events:#?}");
    assert!(saw_complete, "missing AiMessageComplete; got {events:#?}");
    assert!(saw_job_completed, "missing JobCompleted; got {events:#?}");
    assert_eq!(text, "hello world", "AiToken deltas concatenate in order");

    let job_row = rpc
        .get_job(codeless_rpc::GetJobArgs { job_id: job.id })
        .await
        .unwrap();
    assert_eq!(job_row.status, JobStatus::Completed);
}
