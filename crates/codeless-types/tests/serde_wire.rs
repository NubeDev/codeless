//! Wire-format checks. The `type` discriminant strings here are part of
//! the public protocol — every label below appears verbatim in
//! `DOCS/SCOPE.md` "What each level means" and Appendix A. A failing
//! test means a serde rename slipped and clients on older builds will
//! drop the event.

use codeless_types::{
    Event, EventCursor, EventEnvelope, GitAuth, JobId, JobStatus, RepoId, ReviewId, StageId,
    StageStatus, StopReason, TaskId, TaskStatus, UnixMillis,
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
fn repo_added_event_label() {
    let ev = Event::RepoAdded {
        repo_id: RepoId::new(),
    };
    let v = serde_json::to_value(&ev).unwrap();
    assert_eq!(v["type"], "repo-added");
}
