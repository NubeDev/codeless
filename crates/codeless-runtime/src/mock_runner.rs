use std::time::Duration;

use async_trait::async_trait;
use codeless_types::Event;
use parking_lot::Mutex;

use crate::runner::{Runner, RunnerContext, RunnerOutcome};
use crate::time::now_ms;

/// One step in a scripted `MockRunner` run. Stages 7+ replace this with
/// real runner behaviour; today the harness exists so the state
/// machine, store, and event bus have something to drive end-to-end
/// without spawning a child process.
#[derive(Debug, Clone)]
pub enum MockStep {
    /// Emit an `Event` through the bus exactly as a real runner would.
    /// `job_id`, `stage_id`, `task_id` envelope fields are passed
    /// through `None` unless the variant carries them; tests assert on
    /// the inner `Event` payload, not the envelope keys.
    Emit(Event),
    /// Park the run for `duration` before continuing. Used by tests
    /// that need to observe an in-flight state (e.g. `Running` before
    /// the final event lands).
    Sleep(Duration),
    /// Final outcome. Must be the last step; later steps are ignored.
    Finish(RunnerOutcome),
}

/// Scripted `Runner` used by the in-process harness and by integration
/// tests. Steps are consumed in order on each `run` call; the same
/// `MockRunner` instance can be re-armed by passing a fresh script via
/// `set_script`.
pub struct MockRunner {
    script: Mutex<Vec<MockStep>>,
}

impl MockRunner {
    pub fn new(script: Vec<MockStep>) -> Self {
        Self {
            script: Mutex::new(script),
        }
    }

    pub fn set_script(&self, script: Vec<MockStep>) {
        *self.script.lock() = script;
    }
}

#[async_trait]
impl Runner for MockRunner {
    async fn run(&self, ctx: RunnerContext) -> RunnerOutcome {
        let steps: Vec<MockStep> = std::mem::take(&mut *self.script.lock());
        for step in steps {
            match step {
                MockStep::Emit(event) => {
                    ctx.bus
                        .publish(Some(ctx.job_id), None, None, event, now_ms());
                }
                MockStep::Sleep(d) => tokio::time::sleep(d).await,
                MockStep::Finish(outcome) => return outcome,
            }
        }
        RunnerOutcome::Failed {
            reason: "mock runner script ended without Finish".into(),
        }
    }
}
