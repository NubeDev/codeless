//! `.codeless/jobs/<name>.yaml` parser — see `DOCS/JOB-MODEL.md` and
//! `DOCS/JOB-DIR.md`.
//!
//! Two stage shapes are accepted on the wire, and both round-trip
//! through the structured form:
//!
//! ```yaml
//! # Flat — every entry is a bare title string. `REVIEW ` prefix
//! # marks a human-gated stage.
//! stages:
//!   - scaffold main.go
//!   - REVIEW api shape
//!   - add go.mod
//! ```
//!
//! ```yaml
//! # Structured — each entry is a mapping with an optional per-stage
//! # docs list. `review: true` replaces the `REVIEW ` prefix.
//! docs:
//!   - SCOPE.md          # global: every stage reads these in order
//!   - CONVENTIONS.md
//! stages:
//!   - title: scaffold main.go
//!     docs:             # appended after global docs for this stage
//!       - design/routing.md
//!   - title: api shape
//!     review: true
//!   - title: add go.mod
//! ```
//!
//! Mixed lists are allowed; a single `stages:` block can carry both
//! flat strings and structured maps. The runtime always sees the
//! structured form after `parse_yaml`.

use serde::{Deserialize, Deserializer};

/// Parsed shape of the per-job YAML.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct JobTemplate {
    /// Human-meaningful slug; unique per repo by user convention.
    pub name: String,
    /// One-paragraph user-facing description of what this job
    /// accomplishes. Folded into every stage prompt as context so the
    /// runner sees the whole goal, not just the current stage title.
    pub goal: String,
    /// Ordered job-dir filenames every stage reads as context before
    /// running. `None` ⇒ legacy auto-discover (every `*.md`,
    /// SCOPE.md first, then WORKFLOW.md, then alpha). `Some([...])`
    /// pins the exact set and order.
    #[serde(default)]
    pub docs: Option<Vec<String>>,
    /// Ordered list of stages. Each stage carries a title, an
    /// optional REVIEW flag, and an optional per-stage docs list that
    /// the agent reads in addition to the global `docs` field. Bare
    /// title strings (`- "do thing"`) deserialize the same way as
    /// `{ title: "do thing" }` so existing YAML keeps working.
    pub stages: Vec<StageSpec>,
}

/// One stage's authored content. The wire form is permissive — a
/// bare string is just a `title`, the `REVIEW ` prefix maps to
/// `review: true`. New jobs author the structured form so the UI can
/// round-trip per-stage docs without re-deriving them from the title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSpec {
    /// Stage title. Also used as the commit subject when the runner
    /// lands the stage. Never carries the `REVIEW ` prefix in the
    /// structured form — that's split into `review` at parse time.
    pub title: String,
    /// True if this stage is a human gate (REVIEW). The runner
    /// surfaces `review-requested` rather than driving through.
    pub review: bool,
    /// Job-dir filenames the agent reads *in addition to* the
    /// template's global `docs:` list, in this order. Always appended
    /// after the globals — the global list is the shared baseline,
    /// per-stage docs add specifics. `None` and `Some([])` both mean
    /// "no extras"; we keep `Option` so the YAML round-trip can omit
    /// the key entirely when the user has not opted in.
    pub docs: Option<Vec<String>>,
}

impl StageSpec {
    fn from_title(raw: &str) -> Self {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("REVIEW ") {
            Self {
                title: rest.trim().to_string(),
                review: true,
                docs: None,
            }
        } else if trimmed == "REVIEW" {
            Self {
                title: String::new(),
                review: true,
                docs: None,
            }
        } else {
            Self {
                title: trimmed.to_string(),
                review: false,
                docs: None,
            }
        }
    }
}

/// Custom deserializer: accept either a string or a map for each
/// stage entry. The string path delegates to `StageSpec::from_title`
/// so the `REVIEW ` prefix convention is preserved. The map path is
/// a plain field-by-field deserialize so YAML tooling autocompletes
/// the same shape the runtime emits.
impl<'de> Deserialize<'de> for StageSpec {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bare(String),
            Structured(StructuredStage),
        }

        #[derive(Deserialize)]
        struct StructuredStage {
            title: String,
            #[serde(default)]
            review: bool,
            #[serde(default)]
            docs: Option<Vec<String>>,
        }

        match Raw::deserialize(d)? {
            Raw::Bare(s) => Ok(StageSpec::from_title(&s)),
            Raw::Structured(s) => Ok(StageSpec {
                title: s.title,
                review: s.review,
                docs: s.docs,
            }),
        }
    }
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

    /// Orchestrator-facing view of each stage. Borrows from the
    /// template so callers don't pay an allocation per stage; the
    /// `docs` slice is empty when the stage opted out.
    pub fn planned_stages(&self) -> Vec<PlannedStage<'_>> {
        self.stages
            .iter()
            .enumerate()
            .map(|(i, s)| PlannedStage {
                index: i,
                title: s.title.as_str(),
                is_review: s.review,
                docs: s.docs.as_deref().unwrap_or(&[]),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PlannedStage<'a> {
    pub index: usize,
    pub title: &'a str,
    pub is_review: bool,
    /// Per-stage docs, in declaration order. The prompt builder
    /// concatenates these after the template's global `docs` for the
    /// specific stage being run.
    pub docs: &'a [String],
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
    fn parses_flat_string_stages() {
        let src = r#"
name: hello-gin
goal: A minimal Gin hello world server.
stages:
  - scaffold main.go
  - REVIEW api shape
  - add go.mod
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.stages.len(), 3);
        assert_eq!(t.stages[0].title, "scaffold main.go");
        assert!(!t.stages[0].review);
        assert_eq!(t.stages[1].title, "api shape");
        assert!(t.stages[1].review);
        for s in &t.stages {
            assert!(s.docs.is_none());
        }
    }

    #[test]
    fn parses_structured_stages_with_per_stage_docs() {
        let src = r#"
name: webserver
goal: Build a Go HTTP server.
docs:
  - SCOPE.md
  - CONVENTIONS.md
stages:
  - title: scaffold main.go
    docs:
      - design/routing.md
      - design/scaffolding.md
  - title: add routes
    docs:
      - design/handlers.md
  - title: review
    review: true
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(
            t.docs.as_deref(),
            Some(&["SCOPE.md".to_string(), "CONVENTIONS.md".to_string()][..])
        );
        assert_eq!(t.stages.len(), 3);
        assert_eq!(
            t.stages[0].docs.as_deref().unwrap(),
            &[
                "design/routing.md".to_string(),
                "design/scaffolding.md".to_string()
            ]
        );
        assert_eq!(
            t.stages[1].docs.as_deref().unwrap(),
            &["design/handlers.md".to_string()]
        );
        assert!(t.stages[2].docs.is_none());
        assert!(t.stages[2].review);
    }

    #[test]
    fn parses_mixed_flat_and_structured_stages() {
        let src = r#"
name: mix
goal: mixed stage shapes
stages:
  - flat-string stage
  - title: structured stage
    docs:
      - design.md
  - REVIEW gate
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.stages[0].title, "flat-string stage");
        assert!(t.stages[0].docs.is_none());
        assert_eq!(t.stages[1].title, "structured stage");
        assert_eq!(
            t.stages[1].docs.as_deref().unwrap(),
            &["design.md".to_string()]
        );
        assert!(t.stages[2].review);
        assert_eq!(t.stages[2].title, "gate");
    }

    #[test]
    fn planned_stages_exposes_docs_borrow() {
        let src = r#"
name: x
goal: y
stages:
  - title: one
    docs:
      - a.md
      - b.md
  - two
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        let planned = t.planned_stages();
        assert_eq!(planned[0].docs, &["a.md".to_string(), "b.md".to_string()]);
        assert!(planned[1].docs.is_empty());
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
