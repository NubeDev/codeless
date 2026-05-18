//! `PlanEngine` — in-memory driver that turns terminal Job events
//! into the next `JobSpawner::spawn` call along a `PlanSpec`.
//!
//! Design:
//!
//! - One subscription, dispatch by `JobId`. The host wires a single
//!   `EventEnvelope` stream into `handle_event`; this module does not
//!   itself open a subscription so the same engine works in tests
//!   (hand-driven envelopes) and in production (runtime event bus).
//! - One PlanRun = one state machine. State is the id of the step
//!   whose spawned Job we are currently waiting on. On each terminal
//!   event we look up that step, pick `on_success` or `on_failure`,
//!   and either spawn the next Job or mark the run done.
//! - All state in-memory. `HashMap<PlanId, PlanSpec>` holds registered
//!   plans, `HashMap<PlanRunId, PlanRunState>` holds in-flight runs,
//!   and `HashMap<JobId, PlanRunId>` is the join index from incoming
//!   envelopes back to the run that owns the job. Restart wipes
//!   everything, matching the P1 scope.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::sync::Mutex;

use codeless_types::event::{Event, EventEnvelope};
use codeless_types::id::JobId;

use super::spec::{PlanSpec, PlanSpecError, StepId, Transition};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlanId(pub String);

impl PlanId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlanRunId(pub String);

impl PlanRunId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the engine asks the host to do when a step needs to run.
///
/// The crate stays host-tool-agnostic: implementations live in the
/// runtime / mcp layer and translate `job_template` into whatever the
/// host's spawn surface looks like. Returning the `JobId` is the
/// contract — the engine indexes future terminal events by it.
#[async_trait]
pub trait JobSpawner: Send + Sync + 'static {
    async fn spawn(
        &self,
        plan_run_id: &PlanRunId,
        step_id: &StepId,
        job_template: &str,
    ) -> Result<JobId, SpawnError>;
}

pub type SpawnError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum PlanEngineError {
    #[error("invalid plan spec: {0}")]
    InvalidSpec(#[from] PlanSpecError),
    #[error("unknown plan id: {}", .0.as_str())]
    UnknownPlan(PlanId),
    #[error("plan has no steps: {}", .0.as_str())]
    EmptyPlan(PlanId),
    #[error("job spawner failed: {0}")]
    Spawn(SpawnError),
}

/// Terminal outcome the engine cares about. Pause/resume are not
/// terminal — see the stage-1 survey notes; only the three variants
/// below advance a PlanRun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Failed,
    Stopped,
}

impl Outcome {
    fn from_event(event: &Event) -> Option<Self> {
        match event {
            Event::JobCompleted { .. } => Some(Outcome::Completed),
            Event::JobFailed { .. } => Some(Outcome::Failed),
            Event::JobStopped { .. } => Some(Outcome::Stopped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRunStatus {
    /// Waiting on `current_job` to terminate.
    Running { current_step: StepId, current_job: JobId },
    /// Reached a `Transition::Stop` after a successful or failed step.
    Done { last_step: StepId },
    /// The spawner refused to start the next step.
    Failed { at_step: StepId, error: String },
}

#[derive(Debug, Clone)]
pub struct PlanRunState {
    pub plan_id: PlanId,
    pub status: PlanRunStatus,
    /// Append-only trace of `(step, outcome)` pairs the run has
    /// already observed. Useful for the upcoming `plan.list` tool
    /// and tests; not load-bearing for state transitions.
    pub history: Vec<(StepId, Outcome)>,
}

struct Inner {
    plans: HashMap<PlanId, PlanSpec>,
    runs: HashMap<PlanRunId, PlanRunState>,
    job_index: HashMap<JobId, PlanRunId>,
    next_run_seq: AtomicU64,
}

pub struct PlanEngine {
    inner: Mutex<Inner>,
    spawner: Arc<dyn JobSpawner>,
}

impl PlanEngine {
    pub fn new(spawner: Arc<dyn JobSpawner>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                plans: HashMap::new(),
                runs: HashMap::new(),
                job_index: HashMap::new(),
                next_run_seq: AtomicU64::new(1),
            }),
            spawner,
        }
    }

    /// Validate the spec and register it under a caller-supplied id.
    /// Re-registering an id overwrites the previous spec — in-flight
    /// runs keep their original spec via the snapshot they hold? No:
    /// they read the registered spec on each step. P1 documents that
    /// re-registration mid-run is undefined; tests do not do it.
    pub async fn register_plan(
        &self,
        plan_id: PlanId,
        spec: PlanSpec,
    ) -> Result<(), PlanEngineError> {
        spec.validate()?;
        let mut g = self.inner.lock().await;
        g.plans.insert(plan_id, spec);
        Ok(())
    }

    /// Kick off a new PlanRun by spawning the first step's Job.
    pub async fn start_run(&self, plan_id: &PlanId) -> Result<PlanRunId, PlanEngineError> {
        let (first_step_id, first_template) = {
            let g = self.inner.lock().await;
            let spec = g
                .plans
                .get(plan_id)
                .ok_or_else(|| PlanEngineError::UnknownPlan(plan_id.clone()))?;
            let first = spec
                .steps
                .first()
                .ok_or_else(|| PlanEngineError::EmptyPlan(plan_id.clone()))?;
            (first.id.clone(), first.job_template.clone())
        };

        let run_id = {
            let g = self.inner.lock().await;
            let n = g.next_run_seq.fetch_add(1, Ordering::Relaxed);
            PlanRunId(format!("run-{n}"))
        };

        let job_id = self
            .spawner
            .spawn(&run_id, &first_step_id, &first_template)
            .await
            .map_err(PlanEngineError::Spawn)?;

        let mut g = self.inner.lock().await;
        g.runs.insert(
            run_id.clone(),
            PlanRunState {
                plan_id: plan_id.clone(),
                status: PlanRunStatus::Running {
                    current_step: first_step_id,
                    current_job: job_id,
                },
                history: Vec::new(),
            },
        );
        g.job_index.insert(job_id, run_id.clone());
        Ok(run_id)
    }

    /// Drive the state machine forward with one envelope from the
    /// upstream event source. Envelopes whose job_id is not tracked,
    /// or whose event is non-terminal, are ignored — exactly the
    /// behaviour of a filter over a shared bus.
    pub async fn handle_event(&self, env: &EventEnvelope) {
        let Some(outcome) = Outcome::from_event(&env.event) else {
            return;
        };
        let Some(job_id) = env.job_id else {
            return;
        };

        // Resolve job → run, then plan, then current step. Done under
        // one lock so handle_event is serialised; the spawn call for
        // the next step releases the lock first to avoid holding it
        // across an await on user code (`JobSpawner::spawn`).
        let next_step_to_spawn: Option<(PlanRunId, StepId, String)>;
        {
            let mut g = self.inner.lock().await;
            let Inner {
                plans,
                runs,
                job_index,
                ..
            } = &mut *g;
            let Some(run_id) = job_index.remove(&job_id) else {
                return;
            };
            let Some(run) = runs.get_mut(&run_id) else {
                return;
            };
            let PlanRunStatus::Running { current_step, .. } = run.status.clone() else {
                // Terminal events arriving after the run is already
                // Done/Failed are ignored. This is the well-defined
                // late-event case.
                return;
            };
            run.history.push((current_step.clone(), outcome));

            let spec = match plans.get(&run.plan_id) {
                Some(s) => s,
                None => {
                    // Plan was somehow deregistered mid-run. Mark
                    // failed and bail; in P1 there is no deregister
                    // API but this keeps the engine total.
                    let plan_id = run.plan_id.clone();
                    run.status = PlanRunStatus::Failed {
                        at_step: current_step,
                        error: format!("plan `{}` no longer registered", plan_id.as_str()),
                    };
                    return;
                }
            };
            let step = spec
                .steps
                .iter()
                .find(|s| s.id == current_step)
                .expect("current_step came from this spec and ids are validated unique");
            let transition = match outcome {
                Outcome::Completed => &step.on_success,
                // Stopped is treated like Failed for transition
                // purposes: the user-visible job did not succeed, so
                // walk the failure edge. P1 keeps the vocabulary
                // binary; if a future stage needs three edges, it
                // adds `on_stopped`.
                Outcome::Failed | Outcome::Stopped => &step.on_failure,
            };
            match transition {
                Transition::Stop => {
                    run.status = PlanRunStatus::Done {
                        last_step: current_step,
                    };
                    next_step_to_spawn = None;
                }
                Transition::Step(next_id) => {
                    let next = spec
                        .steps
                        .iter()
                        .find(|s| &s.id == next_id)
                        .expect("validate() guarantees the target exists");
                    next_step_to_spawn = Some((run_id, next.id.clone(), next.job_template.clone()));
                }
            }
        }

        if let Some((run_id, next_step_id, template)) = next_step_to_spawn {
            let spawned = self.spawner.spawn(&run_id, &next_step_id, &template).await;
            let mut g = self.inner.lock().await;
            let Some(run) = g.runs.get_mut(&run_id) else {
                return;
            };
            match spawned {
                Ok(job_id) => {
                    run.status = PlanRunStatus::Running {
                        current_step: next_step_id,
                        current_job: job_id,
                    };
                    g.job_index.insert(job_id, run_id);
                }
                Err(e) => {
                    run.status = PlanRunStatus::Failed {
                        at_step: next_step_id,
                        error: e.to_string(),
                    };
                }
            }
        }
    }

    pub async fn run_state(&self, run_id: &PlanRunId) -> Option<PlanRunState> {
        self.inner.lock().await.runs.get(run_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::spec::{PlanStep, StepId};
    use codeless_types::event::EventCursor;
    use codeless_types::id::JobId;
    use codeless_types::{StopReason, UnixMillis};
    use std::sync::Mutex as StdMutex;

    /// Hand-driven spawner: records each call, hands back a JobId
    /// from a pre-seeded queue so tests can also reuse the same id
    /// when fabricating the matching terminal envelope.
    struct MockSpawner {
        calls: StdMutex<Vec<(PlanRunId, StepId, String)>>,
        next_ids: StdMutex<Vec<Result<JobId, String>>>,
    }

    impl MockSpawner {
        fn new(seed: Vec<Result<JobId, String>>) -> Self {
            Self {
                calls: StdMutex::new(Vec::new()),
                next_ids: StdMutex::new(seed),
            }
        }
        fn calls(&self) -> Vec<(PlanRunId, StepId, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl JobSpawner for MockSpawner {
        async fn spawn(
            &self,
            plan_run_id: &PlanRunId,
            step_id: &StepId,
            job_template: &str,
        ) -> Result<JobId, SpawnError> {
            self.calls
                .lock()
                .unwrap()
                .push((plan_run_id.clone(), step_id.clone(), job_template.to_string()));
            match self.next_ids.lock().unwrap().remove(0) {
                Ok(id) => Ok(id),
                Err(e) => Err(e.into()),
            }
        }
    }

    fn step(id: &str, on_success: Transition, on_failure: Transition) -> PlanStep {
        PlanStep {
            id: StepId::new(id),
            job_template: format!("tpl-{id}"),
            on_success,
            on_failure,
        }
    }

    fn envelope(job_id: JobId, event: Event) -> EventEnvelope {
        EventEnvelope {
            cursor: EventCursor(0),
            job_id: Some(job_id),
            stage_id: None,
            task_id: None,
            created_at: UnixMillis(0),
            event,
        }
    }

    fn linear_spec() -> PlanSpec {
        PlanSpec {
            name: "linear".into(),
            steps: vec![
                step(
                    "a",
                    Transition::Step(StepId::new("b")),
                    Transition::Step(StepId::new("c")),
                ),
                step("b", Transition::Stop, Transition::Stop),
                step("c", Transition::Stop, Transition::Stop),
            ],
        }
    }

    #[tokio::test]
    async fn happy_path_walks_success_edge_to_stop() {
        let job_a = JobId::new();
        let job_b = JobId::new();
        let spawner = Arc::new(MockSpawner::new(vec![Ok(job_a), Ok(job_b)]));
        let engine = PlanEngine::new(spawner.clone());

        let pid = PlanId::new("linear");
        engine.register_plan(pid.clone(), linear_spec()).await.unwrap();
        let run = engine.start_run(&pid).await.unwrap();

        engine
            .handle_event(&envelope(job_a, Event::JobCompleted { job_id: job_a }))
            .await;
        engine
            .handle_event(&envelope(job_b, Event::JobCompleted { job_id: job_b }))
            .await;

        let state = engine.run_state(&run).await.unwrap();
        assert!(
            matches!(state.status, PlanRunStatus::Done { ref last_step } if last_step == &StepId::new("b")),
            "expected Done@b, got {:?}",
            state.status,
        );
        let calls = spawner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, StepId::new("a"));
        assert_eq!(calls[1].1, StepId::new("b"));
        assert_eq!(state.history.len(), 2);
    }

    #[tokio::test]
    async fn failure_edge_branches_to_alternate_step() {
        let job_a = JobId::new();
        let job_c = JobId::new();
        let spawner = Arc::new(MockSpawner::new(vec![Ok(job_a), Ok(job_c)]));
        let engine = PlanEngine::new(spawner.clone());

        let pid = PlanId::new("linear");
        engine.register_plan(pid.clone(), linear_spec()).await.unwrap();
        let run = engine.start_run(&pid).await.unwrap();

        engine
            .handle_event(&envelope(job_a, Event::JobFailed { job_id: job_a }))
            .await;
        engine
            .handle_event(&envelope(job_c, Event::JobCompleted { job_id: job_c }))
            .await;

        let state = engine.run_state(&run).await.unwrap();
        assert!(matches!(state.status, PlanRunStatus::Done { .. }));
        let calls = spawner.calls();
        assert_eq!(calls[1].1, StepId::new("c"));
    }

    #[tokio::test]
    async fn stopped_walks_failure_edge() {
        let job_a = JobId::new();
        let job_c = JobId::new();
        let spawner = Arc::new(MockSpawner::new(vec![Ok(job_a), Ok(job_c)]));
        let engine = PlanEngine::new(spawner.clone());

        let pid = PlanId::new("linear");
        engine.register_plan(pid.clone(), linear_spec()).await.unwrap();
        let _ = engine.start_run(&pid).await.unwrap();

        engine
            .handle_event(&envelope(
                job_a,
                Event::JobStopped {
                    job_id: job_a,
                    reason: StopReason::User,
                },
            ))
            .await;

        let calls = spawner.calls();
        assert_eq!(calls[1].1, StepId::new("c"));
    }

    #[tokio::test]
    async fn unrelated_job_id_is_ignored() {
        let job_a = JobId::new();
        let stranger = JobId::new();
        let spawner = Arc::new(MockSpawner::new(vec![Ok(job_a)]));
        let engine = PlanEngine::new(spawner.clone());
        let pid = PlanId::new("linear");
        engine.register_plan(pid.clone(), linear_spec()).await.unwrap();
        let run = engine.start_run(&pid).await.unwrap();

        engine
            .handle_event(&envelope(stranger, Event::JobCompleted { job_id: stranger }))
            .await;

        // Run still parked on step a.
        let state = engine.run_state(&run).await.unwrap();
        assert!(matches!(
            state.status,
            PlanRunStatus::Running { ref current_step, .. } if current_step == &StepId::new("a")
        ));
    }

    #[tokio::test]
    async fn non_terminal_event_does_not_advance_run() {
        let job_a = JobId::new();
        let spawner = Arc::new(MockSpawner::new(vec![Ok(job_a)]));
        let engine = PlanEngine::new(spawner.clone());
        let pid = PlanId::new("linear");
        engine.register_plan(pid.clone(), linear_spec()).await.unwrap();
        let run = engine.start_run(&pid).await.unwrap();

        engine
            .handle_event(&envelope(job_a, Event::JobStarted { job_id: job_a }))
            .await;

        let state = engine.run_state(&run).await.unwrap();
        assert!(matches!(state.status, PlanRunStatus::Running { .. }));
        assert!(state.history.is_empty());
    }

    #[tokio::test]
    async fn spawn_failure_at_next_step_marks_run_failed() {
        let job_a = JobId::new();
        let spawner = Arc::new(MockSpawner::new(vec![
            Ok(job_a),
            Err("template not found".into()),
        ]));
        let engine = PlanEngine::new(spawner.clone());
        let pid = PlanId::new("linear");
        engine.register_plan(pid.clone(), linear_spec()).await.unwrap();
        let run = engine.start_run(&pid).await.unwrap();

        engine
            .handle_event(&envelope(job_a, Event::JobCompleted { job_id: job_a }))
            .await;

        let state = engine.run_state(&run).await.unwrap();
        match state.status {
            PlanRunStatus::Failed { at_step, error } => {
                assert_eq!(at_step, StepId::new("b"));
                assert!(error.contains("template not found"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_run_on_unknown_plan_errors() {
        let spawner = Arc::new(MockSpawner::new(vec![]));
        let engine = PlanEngine::new(spawner);
        let err = engine
            .start_run(&PlanId::new("nope"))
            .await
            .unwrap_err();
        assert!(matches!(err, PlanEngineError::UnknownPlan(_)));
    }

    #[tokio::test]
    async fn late_terminal_event_after_done_is_ignored() {
        let job_a = JobId::new();
        let job_b = JobId::new();
        let spawner = Arc::new(MockSpawner::new(vec![Ok(job_a), Ok(job_b)]));
        let engine = PlanEngine::new(spawner.clone());
        let pid = PlanId::new("linear");
        engine.register_plan(pid.clone(), linear_spec()).await.unwrap();
        let run = engine.start_run(&pid).await.unwrap();

        engine
            .handle_event(&envelope(job_a, Event::JobCompleted { job_id: job_a }))
            .await;
        engine
            .handle_event(&envelope(job_b, Event::JobCompleted { job_id: job_b }))
            .await;
        // Replay the first envelope; should be a no-op now that the
        // join index has been consumed.
        engine
            .handle_event(&envelope(job_a, Event::JobCompleted { job_id: job_a }))
            .await;

        let state = engine.run_state(&run).await.unwrap();
        assert!(matches!(state.status, PlanRunStatus::Done { .. }));
        assert_eq!(state.history.len(), 2);
    }
}
