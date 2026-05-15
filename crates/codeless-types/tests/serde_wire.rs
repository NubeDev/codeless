//! Wire-format checks. The `type` discriminant strings here are part of
//! the public protocol — every label below appears verbatim in
//! `DOCS/SCOPE.md` "What each level means" and Appendix A. A failing
//! test means a serde rename slipped and clients on older builds will
//! drop the event.

use codeless_types::{
    Event, EventCursor, EventEnvelope, GitAuth, JobId, JobStatus, RepoId, ReviewId, ScopePatchId,
    ScopePatchKind, ScopePatchTarget, StageId, StageStatus, StopReason, TaskId, TaskStatus,
    UnixMillis,
};
use serde_json::json;

#[test]
fn task_enqueued_carries_depends_on_from_day_one() {
    let task = TaskId::new();
    let stage = StageId::new();
    let dep = TaskId::new();

    let ev = Event::TaskEnqueued {
        task_id: task,
        stage_id: stage,
        depends_on: vec![dep],
    };

    let v = serde_json::to_value(&ev).unwrap();
    assert_eq!(v["type"], "task-enqueued");
    assert_eq!(v["task_id"], task.to_string());
    assert_eq!(v["stage_id"], stage.to_string());
    assert_eq!(v["depends_on"], json!([dep.to_string()]));

    let round: Event = serde_json::from_value(v).unwrap();
    assert_eq!(round, ev);
}

#[test]
fn job_lifecycle_status_wire_labels() {
    let cases = [
        (JobStatus::Draft, "draft"),
        (JobStatus::Queued, "queued"),
        (JobStatus::Running, "running"),
        (JobStatus::AwaitingReview, "awaiting-review"),
        (JobStatus::Completed, "completed"),
        (JobStatus::Failed, "failed"),
        (JobStatus::Stopped, "stopped"),
    ];
    for (status, wire) in cases {
        let v = serde_json::to_value(status).unwrap();
        assert_eq!(v, json!(wire), "JobStatus::{:?} → {}", status, wire);
    }
}

#[test]
fn stage_and_task_and_review_status_labels() {
    assert_eq!(
        serde_json::to_value(StageStatus::AwaitingReview).unwrap(),
        json!("awaiting-review"),
    );
    assert_eq!(
        serde_json::to_value(TaskStatus::Cancelled).unwrap(),
        json!("cancelled"),
    );
    assert_eq!(
        serde_json::to_value(codeless_types::ReviewStatus::RerunRequested).unwrap(),
        json!("rerun-requested"),
    );
}

#[test]
fn stop_reason_labels() {
    assert_eq!(
        serde_json::to_value(StopReason::WallClock).unwrap(),
        json!("wall-clock"),
    );
    assert_eq!(
        serde_json::to_value(StopReason::RunnerCrash).unwrap(),
        json!("runner-crash"),
    );
}

#[test]
fn git_auth_tagged_with_kind() {
    let v = serde_json::to_value(GitAuth::Token {
        env_var: "GITHUB_TOKEN".into(),
    })
    .unwrap();
    assert_eq!(v["kind"], "token");
    assert_eq!(v["env_var"], "GITHUB_TOKEN");
}

#[test]
fn event_envelope_round_trips_with_optional_ids() {
    let env = EventEnvelope {
        cursor: EventCursor(42),
        job_id: Some(JobId::new()),
        stage_id: None,
        task_id: None,
        created_at: UnixMillis(1_700_000_000_000),
        event: Event::JobStarted {
            job_id: JobId::new(),
        },
    };
    let s = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope = serde_json::from_str(&s).unwrap();
    assert_eq!(back, env);
}

#[test]
fn review_event_label_is_kebab_case() {
    let ev = Event::ReviewRequested {
        review_id: ReviewId::new(),
        stage_id: StageId::new(),
    };
    let v = serde_json::to_value(&ev).unwrap();
    assert_eq!(v["type"], "review-requested");
}

#[test]
fn stage_session_captured_round_trips() {
    let stage = StageId::new();
    let ev = Event::StageSessionCaptured {
        stage_id: stage,
        session_id: "sess-01HXYZ".into(),
    };
    let v = serde_json::to_value(&ev).unwrap();
    assert_eq!(v["type"], "stage-session-captured");
    assert_eq!(v["stage_id"], stage.to_string());
    assert_eq!(v["session_id"], "sess-01HXYZ");

    let round: Event = serde_json::from_value(v).unwrap();
    assert_eq!(round, ev);
}

#[test]
fn verify_step_event_labels_and_payloads_round_trip() {
    let stage = StageId::new();

    let started = Event::VerifyStepStarted {
        stage_id: stage,
        step_index: 0,
        name: "cargo check".into(),
    };
    let v = serde_json::to_value(&started).unwrap();
    assert_eq!(v["type"], "verify-step-started");
    assert_eq!(v["stage_id"], stage.to_string());
    assert_eq!(v["step_index"], 0);
    assert_eq!(v["name"], "cargo check");
    let back: Event = serde_json::from_value(v).unwrap();
    assert_eq!(back, started);

    let passed = Event::VerifyStepPassed {
        stage_id: stage,
        step_index: 1,
        name: "cargo test".into(),
        duration_ms: 4321,
    };
    let v = serde_json::to_value(&passed).unwrap();
    assert_eq!(v["type"], "verify-step-passed");
    assert_eq!(v["duration_ms"], 4321);
    let back: Event = serde_json::from_value(v).unwrap();
    assert_eq!(back, passed);

    let failed = Event::VerifyStepFailed {
        stage_id: stage,
        step_index: 2,
        name: "cargo clippy".into(),
        exit_code: 101,
        tail: "error: lints failed\n".into(),
    };
    let v = serde_json::to_value(&failed).unwrap();
    assert_eq!(v["type"], "verify-step-failed");
    assert_eq!(v["exit_code"], 101);
    assert_eq!(v["tail"], "error: lints failed\n");
    let back: Event = serde_json::from_value(v).unwrap();
    assert_eq!(back, failed);

    let skipped = Event::VerifyStepSkipped {
        stage_id: stage,
        step_index: 3,
        name: "cargo fmt".into(),
        reason: "prior-gate-red".into(),
    };
    let v = serde_json::to_value(&skipped).unwrap();
    assert_eq!(v["type"], "verify-step-skipped");
    assert_eq!(v["reason"], "prior-gate-red");
    let back: Event = serde_json::from_value(v).unwrap();
    assert_eq!(back, skipped);
}

#[test]
fn scope_patch_proposed_event_wire_shape() {
    let stage = StageId::new();
    let review = ReviewId::new();
    let patch = ScopePatchId::new();
    let evidence = StageId::new();
    let ev = Event::ScopePatchProposed {
        stage_id: stage,
        review_id: review,
        patch_id: patch,
        kind: ScopePatchKind::Loosen,
        target: ScopePatchTarget::JobScopeMd,
        target_path: ".codeless/jobs/foo/SCOPE.md".into(),
        evidence_stage_id: Some(evidence),
        has_predicate: false,
    };
    let v = serde_json::to_value(&ev).unwrap();
    assert_eq!(v["type"], "scope-patch-proposed");
    assert_eq!(v["kind"], "loosen");
    assert_eq!(v["target"], "job-scope-md");
    assert_eq!(v["target_path"], ".codeless/jobs/foo/SCOPE.md");
    assert_eq!(v["evidence_stage_id"], evidence.to_string());
    assert_eq!(v["has_predicate"], false);
    let round: Event = serde_json::from_value(v).unwrap();
    assert_eq!(round, ev);
}

#[test]
fn repo_added_event_label() {
    let ev = Event::RepoAdded {
        repo_id: RepoId::new(),
    };
    let v = serde_json::to_value(&ev).unwrap();
    assert_eq!(v["type"], "repo-added");
}
