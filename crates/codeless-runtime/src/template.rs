//! `.codeless/jobs/<name>.yaml` parser — three fields, per JOB-MODEL.md.
//!
//! ```yaml
//! name: hello-gin
//! goal: A minimal Gin "hello world" HTTP server in Go.
//! stages:
//!   - scaffold the Go module and main.go with a Gin /hello handler
//!   - REVIEW api shape before adding more routes
//!   - add go.mod (run go mod tidy)
//! ```
//!
//! A `REVIEW`-prefixed stage signals a human gate (JOB-MODEL.md
//! "REVIEW-prefixed stages are the user-authoring surface for review
//! gates"). The parser captures the prefix; whether the runner halts
//! on it is the runner's call — today's `TemplateRunner` surfaces a
//! `review-requested` event but does not block, because the
//! review-wait machinery on the runtime side is not yet plumbed
//! through the orchestrator. That's a documented gap, not a silent
//! one.

use serde::Deserialize;

/// Parsed shape of the per-job YAML.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct JobTemplate {
    /// Human-meaningful slug; unique per repo by user convention.
    /// The runtime will eventually use this as the directory key
    /// under `runs/<name>/` (replacing today's per-ULID layout).
    pub name: String,
    /// One-paragraph user-facing description of what this job
    /// accomplishes. Folded into every stage prompt as context so the
    /// runner sees the whole goal, not just the current stage title.
    pub goal: String,
    /// Ordered list of stage titles. Each entry is one stage's title
    /// AND its commit subject when the stage lands successfully.
    /// `REVIEW`-prefixed entries are gate stages.
    pub stages: Vec<String>,
    /// Ordered list of job-dir filenames the agent reads as context
    /// before each stage. `None` means "every `.md` in the job dir,
    /// SCOPE.md first, then WORKFLOW.md, then the rest in alphabetical
    /// order" — the legacy auto-discover behaviour. `Some([...])`
    /// pins the exact order and set; entries that don't exist on disk
    /// are skipped silently (the agent is best-effort about doc
    /// inclusion, not a build gate).
    ///
    /// Filenames are basenames only — same sanitisation rules as
    /// `write_job_file`. Nothing outside `.codeless/jobs/<name>/`.
    #[serde(default)]
    pub docs: Option<Vec<String>>,
}

impl JobTemplate {
    pub fn parse_yaml(src: &str) -> Result<Self, TemplateError> {
        let parsed: Self =
            serde_yaml::from_str(src).map_err(|e| TemplateError::Yaml(e.to_string()))?;
        if parsed.name.trim().is_empty() {
            return Err(TemplateError::EmptyField("name"));
        }
        if parsed.goal.trim().is_empty() {
            return Err(TemplateError::EmptyField("goal"));
        }
        if parsed.stages.is_empty() {
            return Err(TemplateError::EmptyField("stages"));
        }
        Ok(parsed)
    }

    /// Decompose into the orchestrator's view of each stage: title,
    /// review flag, zero-based index. Keeps the iteration logic in
    /// `TemplateRunner` agnostic to string parsing.
    pub fn planned_stages(&self) -> Vec<PlannedStage<'_>> {
        self.stages
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                let trimmed = raw.trim();
                let (is_review, title) = match trimmed.strip_prefix("REVIEW ") {
                    Some(rest) => (true, rest.trim()),
                    None => (false, trimmed),
                };
                PlannedStage {
                    index: i,
                    title,
                    is_review,
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PlannedStage<'a> {
    pub index: usize,
    pub title: &'a str,
    pub is_review: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    Yaml(String),
    EmptyField(&'static str),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::Yaml(s) => write!(f, "yaml parse: {s}"),
            TemplateError::EmptyField(field) => {
                write!(f, "template missing required field: {field}")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_fields() {
        let src = r#"
name: hello-gin
goal: A minimal Gin hello world server.
stages:
  - scaffold main.go
  - REVIEW api shape
  - add go.mod
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.name, "hello-gin");
        assert!(t.goal.contains("Gin"));
        assert_eq!(t.stages.len(), 3);
    }

    #[test]
    fn planned_stages_splits_review_prefix() {
        let t = JobTemplate {
            name: "x".into(),
            goal: "x".into(),
            docs: None,
            stages: vec![
                "do thing".into(),
                "REVIEW the result".into(),
                "do another thing".into(),
            ],
        };
        let planned = t.planned_stages();
        assert_eq!(planned.len(), 3);
        assert!(!planned[0].is_review);
        assert_eq!(planned[0].title, "do thing");
        assert!(planned[1].is_review);
        assert_eq!(planned[1].title, "the result");
        assert!(!planned[2].is_review);
    }

    #[test]
    fn empty_required_fields_error() {
        let src = "name: x\ngoal: y\nstages: []\n";
        match JobTemplate::parse_yaml(src) {
            Err(TemplateError::EmptyField("stages")) => {}
            other => panic!("expected EmptyField(stages), got {other:?}"),
        }
    }

    #[test]
    fn malformed_yaml_errors() {
        let src = "not: yaml: at: all";
        assert!(matches!(
            JobTemplate::parse_yaml(src),
            Err(TemplateError::Yaml(_))
        ));
    }
}
