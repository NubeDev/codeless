//! `PlanSpec`, `PlanStep`, `Transition` — pure data, no engine.
//!
//! Stage P1 of the plan engine. `validate` runs at parse time so the
//! engine never has to defend against dangling targets or duplicate
//! ids: every `Transition::Step` is guaranteed to resolve to a real
//! `PlanStep`, and every step id is unique within the spec.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Reserved transition target meaning "terminate this PlanRun
/// successfully." Modelled as an explicit enum variant rather than a
/// magic step id so the wire format catches typos (`stoop` is a
/// dangling target, not a silent stop).
const STOP_LITERAL: &str = "stop";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StepId(pub String);

impl StepId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a step's success/failure edge points.
///
/// `"stop"` in the wire format → `Transition::Stop`. Any other string
/// → `Transition::Step(StepId)`. Field omission in `PlanStep` defaults
/// to `Stop` so the linear/terminal case stays terse (the last step
/// of a chain has no `on_success`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Transition {
    Step(StepId),
    #[default]
    Stop,
}

impl Serialize for Transition {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            Transition::Stop => ser.serialize_str(STOP_LITERAL),
            Transition::Step(id) => ser.serialize_str(id.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for Transition {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        Ok(if s == STOP_LITERAL {
            Transition::Stop
        } else {
            Transition::Step(StepId(s))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: StepId,
    /// Name of a job template the host knows how to spawn. The plan
    /// crate intentionally does not resolve templates — that is the
    /// `JobSpawner`'s job — so this is just an opaque string here.
    pub job_template: String,
    #[serde(default)]
    pub on_success: Transition,
    #[serde(default)]
    pub on_failure: Transition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSpec {
    pub name: String,
    pub steps: Vec<PlanStep>,
}

impl PlanSpec {
    /// Reject specs the engine cannot safely run: duplicate step ids,
    /// or a transition target that names a step that does not exist.
    pub fn validate(&self) -> Result<(), PlanSpecError> {
        let mut seen: HashSet<&str> = HashSet::with_capacity(self.steps.len());
        for step in &self.steps {
            if !seen.insert(step.id.as_str()) {
                return Err(PlanSpecError::DuplicateId(step.id.clone()));
            }
        }
        for step in &self.steps {
            check_target(&step.on_success, &seen, &step.id)?;
            check_target(&step.on_failure, &seen, &step.id)?;
        }
        Ok(())
    }
}

fn check_target(t: &Transition, known: &HashSet<&str>, from: &StepId) -> Result<(), PlanSpecError> {
    if let Transition::Step(target) = t {
        if !known.contains(target.as_str()) {
            return Err(PlanSpecError::UnknownTarget {
                from: from.clone(),
                target: target.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanSpecError {
    #[error("duplicate step id: {}", .0.as_str())]
    DuplicateId(StepId),
    #[error(
        "step `{from}` transitions to unknown target `{target}`",
        from = .from.as_str(),
        target = .target.as_str(),
    )]
    UnknownTarget { from: StepId, target: StepId },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, on_success: Transition, on_failure: Transition) -> PlanStep {
        PlanStep {
            id: StepId::new(id),
            job_template: format!("tpl-{id}"),
            on_success,
            on_failure,
        }
    }

    #[test]
    fn linear_chain_from_workflow_doc_validates() {
        // The release-prep example from DOCS/JOB-WORKFLOW.md, minus
        // the per-step yaml keys irrelevant to the spec layer.
        let spec = PlanSpec {
            name: "release-prep".into(),
            steps: vec![
                step(
                    "lint",
                    Transition::Step(StepId::new("test")),
                    Transition::Stop,
                ),
                step(
                    "test",
                    Transition::Step(StepId::new("changelog")),
                    Transition::Step(StepId::new("notify-and-stop")),
                ),
                step(
                    "changelog",
                    Transition::Step(StepId::new("publish")),
                    Transition::Stop,
                ),
                step("publish", Transition::Stop, Transition::Stop),
                step("notify-and-stop", Transition::Stop, Transition::Stop),
            ],
        };
        assert_eq!(spec.validate(), Ok(()));
    }

    #[test]
    fn duplicate_id_rejected() {
        let spec = PlanSpec {
            name: "dup".into(),
            steps: vec![
                step("a", Transition::Stop, Transition::Stop),
                step("a", Transition::Stop, Transition::Stop),
            ],
        };
        assert_eq!(
            spec.validate(),
            Err(PlanSpecError::DuplicateId(StepId::new("a")))
        );
    }

    #[test]
    fn unknown_target_rejected() {
        let spec = PlanSpec {
            name: "dangling".into(),
            steps: vec![step(
                "only",
                Transition::Step(StepId::new("ghost")),
                Transition::Stop,
            )],
        };
        assert_eq!(
            spec.validate(),
            Err(PlanSpecError::UnknownTarget {
                from: StepId::new("only"),
                target: StepId::new("ghost"),
            })
        );
    }

    #[test]
    fn transition_serde_round_trips_step_and_stop() {
        // "stop" → Stop, any other string → Step.
        let stop: Transition = serde_json::from_str("\"stop\"").unwrap();
        assert_eq!(stop, Transition::Stop);
        let step_t: Transition = serde_json::from_str("\"next\"").unwrap();
        assert_eq!(step_t, Transition::Step(StepId::new("next")));
        assert_eq!(
            serde_json::to_string(&Transition::Stop).unwrap(),
            "\"stop\""
        );
        assert_eq!(
            serde_json::to_string(&Transition::Step(StepId::new("next"))).unwrap(),
            "\"next\""
        );
    }

    #[test]
    fn omitted_transition_defaults_to_stop() {
        // PlanStep with neither `on_success` nor `on_failure` set.
        let json = r#"{"id":"only","job_template":"tpl"}"#;
        let step: PlanStep = serde_json::from_str(json).unwrap();
        assert_eq!(step.on_success, Transition::Stop);
        assert_eq!(step.on_failure, Transition::Stop);
    }
}
