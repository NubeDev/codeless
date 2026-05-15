//! Per-RPC tests for the assistant-facing `jobs.*` surface added by
//! F3a: `update_job_scope` and `draft_job_from_conversation`. Both
//! drive an in-process runtime against a real on-disk git repo because
//! the underlying `write_job_file` / `submit_job` paths commit via
//! `commit_paths`. A `:memory:` SQLite is plenty; only the working
//! tree has to be real.

use std::path::Path;
use std::process::Command;

use codeless_rpc::{
    AddRepoArgs, AppendAssistantMessageArgs, ConfirmAssistantActionArgs,
    DraftJobFromConversationArgs, GetJobArgs, ListJobsArgs, ReadJobFileArgs, RpcError, RpcServer,
    SubmitJobArgs, UpdateJobScopeArgs,
};
use codeless_runtime::InProcessRpc;
use codeless_types::{
    AssistantActionCard, AssistantActionStatus, AssistantMessageRole, AssistantThreadId,
    CostCents, GitAuth, Job, JobId, JobStatus, Repo, RepoId, WorkspaceMode,
};
use tempfile::TempDir;

fn init_repo(dir: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "test@example.com"][..],
        &["config", "user.name", "test"][..],
        &["commit", "--allow-empty", "-q", "-m", "root"][..],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args.iter().copied())
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?}: {:?}", out);
    }
}

const TEMPLATE_YAML: &str = "name: scope-target\ngoal: g\nstages:\n  - one\n";

async fn fixture_with_template_job() -> (InProcessRpc, TempDir, JobId) {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: tmp.path().to_string_lossy().into_owned(),
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
            prompt: None,
            template_yaml: Some(TEMPLATE_YAML.into()),
            runner: "mock".into(),
            branch: "codeless/job-x".into(),
            workspace_mode: None,
            cost_cap_cents: 0,
            wall_clock_cap_ms: 0,
            model: None,
            permission_mode: None,
            effort: None,
            start_immediately: false,
        })
        .await
        .unwrap();
    (rpc, tmp, job.id)
}

/// Force the job row into a non-`Draft` status without going through the
/// driver. The two new RPCs only branch on `JobStatus`, so a direct
/// store write is the cleanest way to probe the guard without spinning
/// up a runner.
async fn set_status(rpc: &InProcessRpc, job_id: JobId, status: JobStatus) {
    let mut job = rpc.store().get_job(job_id).await.unwrap().unwrap();
    job.status = status;
    rpc.store().update_job(&job).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_scope_writes_through_paused_job() {
    let (rpc, tmp, job_id) = fixture_with_template_job().await;
    set_status(&rpc, job_id, JobStatus::Paused).await;
    let new_body = "# New scope\n\nrewritten by assistant\n";
    let res = rpc
        .update_job_scope(UpdateJobScopeArgs {
            job_id,
            content: new_body.into(),
        })
        .await
        .unwrap();
    assert_eq!(res.filename, "SCOPE.md");
    let on_disk =
        std::fs::read_to_string(tmp.path().join(".codeless/jobs/scope-target/SCOPE.md")).unwrap();
    assert_eq!(on_disk, new_body);
    // And the chat-readable path round-trips through `read_job_file`,
    // matching the Spec pane's view.
    let read = rpc
        .read_job_file(ReadJobFileArgs {
            job_id,
            filename: "SCOPE.md".into(),
        })
        .await
        .unwrap();
    assert_eq!(read.content, new_body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_scope_rejects_running_job_with_conflict() {
    let (rpc, _tmp, job_id) = fixture_with_template_job().await;
    set_status(&rpc, job_id, JobStatus::Running).await;
    let err = rpc
        .update_job_scope(UpdateJobScopeArgs {
            job_id,
            content: "# Should not land\n".into(),
        })
        .await
        .unwrap_err();
    // Typed Conflict — the UI keys off this to surface the "pause
    // first" affordance. `Internal` or `InvalidArgument` would be
    // wrong; the row is fine, the action is just disallowed in this
    // state.
    assert!(matches!(err, RpcError::Conflict(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_scope_rejects_queued_and_awaiting_review() {
    let (rpc, _tmp, job_id) = fixture_with_template_job().await;
    for status in [JobStatus::Queued, JobStatus::AwaitingReview] {
        set_status(&rpc, job_id, status).await;
        let err = rpc
            .update_job_scope(UpdateJobScopeArgs {
                job_id,
                content: "x".repeat(10),
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, RpcError::Conflict(_)),
            "{status:?} should be Conflict, got {err:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_scope_rejects_unknown_job() {
    let (rpc, _tmp, _job_id) = fixture_with_template_job().await;
    let err = rpc
        .update_job_scope(UpdateJobScopeArgs {
            job_id: JobId::new(),
            content: "body".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_scope_rejects_empty_body() {
    let (rpc, _tmp, job_id) = fixture_with_template_job().await;
    let err = rpc
        .update_job_scope(UpdateJobScopeArgs {
            job_id,
            content: "   \n\t".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_from_conversation_consumes_latest_pending_card() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: tmp.path().to_string_lossy().into_owned(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    // Slash-command parser plants a pending DraftJob card on the thread.
    rpc.append_assistant_message(AppendAssistantMessageArgs {
        thread_id: thread.id,
        content: format!("/draft {} -- add dark mode", repo.id),
    })
    .await
    .unwrap();

    let job: Job = rpc
        .draft_job_from_conversation(DraftJobFromConversationArgs {
            thread_id: thread.id,
        })
        .await
        .unwrap();
    // SCOPE.md Decisions §3 — the row lands in `Draft`, not `Queued`.
    assert_eq!(job.status, JobStatus::Draft);
    assert_eq!(job.repo_id, repo.id);
    assert_eq!(job.prompt.as_deref(), Some("add dark mode"));
    // Defaults plumbed through from the parse step. A regression that
    // dropped them would silently submit a job with no caps.
    assert_eq!(job.runner, "claude");
    assert_eq!(job.branch, "assistant/draft");
    assert!(job.cost_cap_cents.0 > 0);
    assert!(job.wall_clock_cap_ms > 0);

    let fetched = rpc.get_job(GetJobArgs { job_id: job.id }).await.unwrap();
    assert_eq!(fetched.id, job.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_from_conversation_errors_without_pending_card() {
    let rpc = InProcessRpc::new().await.unwrap();
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    // Plain prose, no card. The call must distinguish this from a
    // missing-thread error so the UI knows whether to surface a
    // "draft something first" hint or a "this thread is gone" toast.
    rpc.append_assistant_message(AppendAssistantMessageArgs {
        thread_id: thread.id,
        content: "hello, no card here".into(),
    })
    .await
    .unwrap();
    let err = rpc
        .draft_job_from_conversation(DraftJobFromConversationArgs {
            thread_id: thread.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_from_conversation_rejects_unknown_thread() {
    let rpc = InProcessRpc::new().await.unwrap();
    let err = rpc
        .draft_job_from_conversation(DraftJobFromConversationArgs {
            thread_id: AssistantThreadId::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::NotFound(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_from_conversation_skips_non_draft_cards() {
    // A `/start` card is not a DraftJob proposal — the dispatcher
    // should walk past it and surface "no pending card" rather than
    // mis-routing the action. Repo doesn't matter because we never
    // reach `submit_job`; the parser plants the wrong-variant card and
    // the RPC bounces.
    let rpc = InProcessRpc::new().await.unwrap();
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    let other_job = JobId::new();
    rpc.append_assistant_message(AppendAssistantMessageArgs {
        thread_id: thread.id,
        content: format!("/start {other_job}"),
    })
    .await
    .unwrap();
    let err = rpc
        .draft_job_from_conversation(DraftJobFromConversationArgs {
            thread_id: thread.id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RpcError::InvalidArgument(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_from_conversation_picks_most_recent_proposal() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let rpc = InProcessRpc::new().await.unwrap();
    let repo_a = rpc
        .add_repo(AddRepoArgs {
            name: "a".into(),
            clone_url: "https://example.test/a.git".into(),
            default_branch: "main".into(),
            local_path: tmp.path().to_string_lossy().into_owned(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    // Two repos exercised; only the local one needs a real worktree
    // because submit_job for prompt-only jobs does not write template
    // files. Keeping both pointed at `tmp.path()` is fine — neither
    // submit reaches the filesystem beyond the row insert.
    let repo_b = RepoId::new();
    // Plant a phantom repo so the second `/draft` card parses but the
    // first is the one we expect to be ignored. The phantom never gets
    // reached because the latest pending card is honoured first.
    rpc.store()
        .insert_repo(&Repo {
            id: repo_b,
            name: "b".into(),
            clone_url: "https://example.test/b.git".into(),
            default_branch: "main".into(),
            local_path: tmp.path().to_string_lossy().into_owned(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
            created_at: codeless_types::UnixMillis(0),
            updated_at: codeless_types::UnixMillis(0),
        })
        .await
        .unwrap();
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    rpc.append_assistant_message(AppendAssistantMessageArgs {
        thread_id: thread.id,
        content: format!("/draft {} -- first proposal", repo_a.id),
    })
    .await
    .unwrap();
    // Eligible-newest rule: the second `/draft` is what `draft_job_from_conversation`
    // must pick, not the first.
    rpc.append_assistant_message(AppendAssistantMessageArgs {
        thread_id: thread.id,
        content: format!("/draft {repo_b} branch=feature -- ship the new thing"),
    })
    .await
    .unwrap();
    let job = rpc
        .draft_job_from_conversation(DraftJobFromConversationArgs {
            thread_id: thread.id,
        })
        .await
        .unwrap();
    assert_eq!(job.repo_id, repo_b);
    assert_eq!(job.branch, "feature");
    assert_eq!(job.prompt.as_deref(), Some("ship the new thing"));
    // Sanity check we did not silently fall back to the in_repo guard
    // by submitting an unmoded job — explicit defaults match parse_draft.
    assert!(matches!(job.workspace_mode, WorkspaceMode::InRepo));
    assert_eq!(job.cost_cents, CostCents::ZERO);
}

// F3b — action-card dispatcher routing. Each test confirms a card from
// a real assistant thread and asserts the resulting state came from
// the named RPC, not from an inline submit/write. Per-action coverage
// for `start/stop/pause/resume/restart/update` lives in the next
// MockRunner stage; the wiring those arms hit is one-line and the
// outer confirm_dispatches_list_jobs_and_writes_tool_message test in
// `assistant.rs` already exercises that path.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirm_edit_scope_routes_through_update_job_scope() {
    // Happy path: a /edit-scope card lands SCOPE.md on disk through
    // `update_job_scope`. The tool message carries a diff and the
    // round-trip read matches the dispatched body, which proves the
    // dispatcher reached the new RPC rather than an inline write.
    let (rpc, tmp, job_id) = fixture_with_template_job().await;
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    // The slash parser trims trailing whitespace from the body so the
    // expected on-disk content drops the surrounding newline. We assert
    // on what the dispatcher actually wrote rather than the raw input.
    let new_body = "# Rewritten\n\nstage 1: do the thing";
    let appended = rpc
        .append_assistant_message(AppendAssistantMessageArgs {
            thread_id: thread.id,
            content: format!("/edit-scope {job_id} -- {new_body}"),
        })
        .await
        .unwrap();
    let confirm = rpc
        .confirm_assistant_action(ConfirmAssistantActionArgs {
            thread_id: thread.id,
            message_id: appended.assistant_message.id,
        })
        .await
        .unwrap();
    let card: AssistantActionCard =
        serde_json::from_str(confirm.card.meta_json.as_deref().unwrap()).unwrap();
    assert!(matches!(card.status, AssistantActionStatus::Confirmed));
    assert!(matches!(
        confirm.tool_message.role,
        AssistantMessageRole::Tool
    ));
    assert!(confirm.tool_message.content.contains("SCOPE.md"));

    let on_disk =
        std::fs::read_to_string(tmp.path().join(".codeless/jobs/scope-target/SCOPE.md")).unwrap();
    assert_eq!(on_disk, new_body);
    let read = rpc
        .read_job_file(ReadJobFileArgs {
            job_id,
            filename: "SCOPE.md".into(),
        })
        .await
        .unwrap();
    assert_eq!(read.content, new_body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirm_edit_scope_rejects_running_job_with_pause_hint() {
    // The paused-job guard now lives inside `update_job_scope`. The
    // dispatcher must surface that typed Conflict as a Failed card
    // whose tool message tells the user how to recover. A regression
    // that dropped the guard (or bypassed `update_job_scope`) would
    // either land the write or swallow the message — both visible
    // here.
    let (rpc, _tmp, job_id) = fixture_with_template_job().await;
    set_status(&rpc, job_id, JobStatus::Running).await;
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    let appended = rpc
        .append_assistant_message(AppendAssistantMessageArgs {
            thread_id: thread.id,
            content: format!("/edit-scope {job_id} -- # rewritten\n"),
        })
        .await
        .unwrap();
    let confirm = rpc
        .confirm_assistant_action(ConfirmAssistantActionArgs {
            thread_id: thread.id,
            message_id: appended.assistant_message.id,
        })
        .await
        .unwrap();
    let card: AssistantActionCard =
        serde_json::from_str(confirm.card.meta_json.as_deref().unwrap()).unwrap();
    assert!(matches!(card.status, AssistantActionStatus::Failed));
    assert!(
        confirm.tool_message.content.contains("pause"),
        "tool message should mention pause: {}",
        confirm.tool_message.content,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirm_edit_scope_rejects_non_scope_filename() {
    // The chat surface contract is "edit_scope rewrites SCOPE.md only".
    // The parser still accepts `filename=…` so a misuse from the model
    // or a power user must surface as a typed Failed card rather than
    // sneaking through to `write_job_file`.
    let (rpc, _tmp, job_id) = fixture_with_template_job().await;
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    let appended = rpc
        .append_assistant_message(AppendAssistantMessageArgs {
            thread_id: thread.id,
            content: format!("/edit-scope {job_id} filename=WORKFLOW.md -- body"),
        })
        .await
        .unwrap();
    let confirm = rpc
        .confirm_assistant_action(ConfirmAssistantActionArgs {
            thread_id: thread.id,
            message_id: appended.assistant_message.id,
        })
        .await
        .unwrap();
    let card: AssistantActionCard =
        serde_json::from_str(confirm.card.meta_json.as_deref().unwrap()).unwrap();
    assert!(matches!(card.status, AssistantActionStatus::Failed));
    assert!(
        confirm.tool_message.content.contains("WORKFLOW.md")
            || confirm.tool_message.content.contains("SCOPE.md"),
        "tool message should name the offending filename: {}",
        confirm.tool_message.content,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirm_draft_routes_through_draft_job_from_conversation() {
    // The dispatcher must consume the pending DraftJob card via the
    // named RPC, not by calling `submit_job` directly. A regression to
    // the inline call would still land a Draft row, but it would not
    // exercise the thread-walk logic — so this test plants the card
    // and then asserts the resulting Job's provenance (prompt +
    // defaults) matches what `draft_job_from_conversation` produces.
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let rpc = InProcessRpc::new().await.unwrap();
    let repo = rpc
        .add_repo(AddRepoArgs {
            name: "demo".into(),
            clone_url: "https://example.test/demo.git".into(),
            default_branch: "main".into(),
            local_path: tmp.path().to_string_lossy().into_owned(),
            git_auth: GitAuth::Token {
                env_var: "GITHUB_TOKEN".into(),
            },
            concurrency_cap: None,
            default_runner: None,
        })
        .await
        .unwrap();
    let thread = rpc
        .create_assistant_thread(codeless_rpc::CreateAssistantThreadArgs { title: None })
        .await
        .unwrap();
    let appended = rpc
        .append_assistant_message(AppendAssistantMessageArgs {
            thread_id: thread.id,
            content: format!("/draft {} -- ship dark mode", repo.id),
        })
        .await
        .unwrap();

    let before = rpc.list_jobs(ListJobsArgs { repo_id: None }).await.unwrap();
    assert!(before.jobs.is_empty(), "fixture should start with no jobs");

    let confirm = rpc
        .confirm_assistant_action(ConfirmAssistantActionArgs {
            thread_id: thread.id,
            message_id: appended.assistant_message.id,
        })
        .await
        .unwrap();
    let card: AssistantActionCard =
        serde_json::from_str(confirm.card.meta_json.as_deref().unwrap()).unwrap();
    assert!(matches!(card.status, AssistantActionStatus::Confirmed));
    assert!(confirm.tool_message.content.starts_with("Drafted job"));

    let after = rpc.list_jobs(ListJobsArgs { repo_id: None }).await.unwrap();
    assert_eq!(after.jobs.len(), 1, "exactly one draft job lands");
    let job = &after.jobs[0];
    assert_eq!(job.status, JobStatus::Draft);
    assert_eq!(job.repo_id, repo.id);
    assert_eq!(job.prompt.as_deref(), Some("ship dark mode"));
    assert_eq!(job.runner, "claude");
    assert_eq!(job.branch, "assistant/draft");
}
