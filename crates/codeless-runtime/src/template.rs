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

use std::time::Duration;

use serde::de::Error as _;
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
    /// How long a warm runner session is held open for interactive
    /// resumption after the last activity, before it is archived and
    /// future input opens a fresh session. `None` ⇒ runtime default
    /// (30 minutes). YAML accepts either a humantime string (`"30m"`,
    /// `"1h"`, `"45s"`) or a bare integer interpreted as seconds.
    #[serde(default, deserialize_with = "deserialize_optional_duration")]
    pub session_idle_timeout: Option<Duration>,
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
    /// One-sentence statement of what success for this stage looks
    /// like. Persisted on the `Stage` row and surfaced in the UI
    /// overview. `None` when the YAML omitted the key.
    pub goal: Option<String>,
    /// Acceptance criteria bullets in author order. The UI renders
    /// each as a tickable line; `None` ≠ `Some(vec![])` is preserved
    /// so the round-trip can tell "key omitted" from "explicitly
    /// empty list".
    pub acceptance: Option<Vec<String>>,
    /// Layered verify gates, run in order on stage completion. The
    /// bare `verify_cmd: "<shell>"` legacy form parses as a single
    /// step named `"verify"`; the structured form is a list of
    /// `{name, run}` pairs. The vec is empty (not absent) when the
    /// YAML omits both keys, so `verify.is_empty()` is the wire
    /// signal for "no verify gate".
    pub verify: Vec<VerifyStep>,
}

/// One layer of a stage's verify gate. Named so the UI can render a
/// per-step row (e.g. `cargo check`, `cargo test`, `cargo clippy`)
/// with its own pass/fail state, rather than collapsing the whole
/// stage's verify into a single bit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VerifyStep {
    pub name: String,
    pub run: String,
}

impl StageSpec {
    fn from_title(raw: &str) -> Self {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("REVIEW ") {
            Self {
                title: rest.trim().to_string(),
                review: true,
                docs: None,
                goal: None,
                acceptance: None,
                verify: Vec::new(),
            }
        } else if trimmed == "REVIEW" {
            Self {
                title: String::new(),
                review: true,
                docs: None,
                goal: None,
                acceptance: None,
                verify: Vec::new(),
            }
        } else {
            Self {
                title: trimmed.to_string(),
                review: false,
                docs: None,
                goal: None,
                acceptance: None,
                verify: Vec::new(),
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
            #[serde(default)]
            goal: Option<String>,
            #[serde(default)]
            acceptance: Option<Vec<String>>,
            // Sugar: a bare string under `verify_cmd:` wraps into a
            // single-step list named "verify". This keeps the existing
            // `verify_cmd: "cargo test"` shape working unchanged.
            #[serde(default)]
            verify_cmd: Option<String>,
            #[serde(default)]
            verify: Option<Vec<VerifyStep>>,
        }

        match Raw::deserialize(d)? {
            Raw::Bare(s) => Ok(StageSpec::from_title(&s)),
            Raw::Structured(s) => {
                let verify = match (s.verify, s.verify_cmd) {
                    (Some(_), Some(_)) => {
                        return Err(D::Error::custom(
                            "stage may set either `verify:` or `verify_cmd:`, not both",
                        ));
                    }
                    (Some(v), None) => v,
                    (None, Some(cmd)) => vec![VerifyStep {
                        name: "verify".to_string(),
                        run: cmd,
                    }],
                    (None, None) => Vec::new(),
                };
                Ok(StageSpec {
                    title: s.title,
                    review: s.review,
                    docs: s.docs,
                    goal: s.goal,
                    acceptance: s.acceptance,
                    verify,
                })
            }
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

/// Permissive duration parser for YAML: accepts a bare integer
/// (seconds), or a suffixed string with one of `s` / `m` / `h` / `d`.
/// Kept in this module rather than pulled from a crate because the
/// runtime already declines on adding `humantime` and the surface we
/// need is small. Rejects fractional values — the use sites are
/// human-authored caps, not measurement intervals.
fn deserialize_optional_duration<'de, D>(d: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Seconds(u64),
        Text(String),
    }

    match Option::<Raw>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::Seconds(s)) => Ok(Some(Duration::from_secs(s))),
        Some(Raw::Text(s)) => parse_duration_str(s.trim())
            .map(Some)
            .map_err(D::Error::custom),
    }
}

fn parse_duration_str(s: &str) -> Result<Duration, String> {
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_part, unit) = s.split_at(
        s.find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| format!("duration `{s}` missing unit (expected s/m/h/d)"))?,
    );
    let n: u64 = num_part
        .parse()
        .map_err(|e| format!("duration `{s}` has non-numeric magnitude: {e}"))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 60 * 60,
        "d" => n * 60 * 60 * 24,
        other => return Err(format!("duration `{s}` has unknown unit `{other}`")),
    };
    Ok(Duration::from_secs(secs))
}

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
    fn parses_structured_stage_with_goal_and_acceptance() {
        let src = r#"
name: x
goal: y
stages:
  - title: scaffold
    goal: render the empty form
    acceptance:
      - form mounts without runtime errors
      - submit button is disabled while pristine
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.stages[0].goal.as_deref(), Some("render the empty form"));
        assert_eq!(
            t.stages[0].acceptance.as_deref().unwrap(),
            &[
                "form mounts without runtime errors".to_string(),
                "submit button is disabled while pristine".to_string()
            ]
        );
    }

    #[test]
    fn verify_cmd_string_sugar_wraps_into_single_step_list() {
        let src = r#"
name: x
goal: y
stages:
  - title: build
    verify_cmd: cargo test
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.stages[0].verify.len(), 1);
        assert_eq!(t.stages[0].verify[0].name, "verify");
        assert_eq!(t.stages[0].verify[0].run, "cargo test");
    }

    #[test]
    fn structured_verify_list_round_trips() {
        let src = r#"
name: x
goal: y
stages:
  - title: build
    verify:
      - name: check
        run: cargo check
      - name: test
        run: cargo test
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.stages[0].verify.len(), 2);
        assert_eq!(t.stages[0].verify[0].name, "check");
        assert_eq!(t.stages[0].verify[0].run, "cargo check");
        assert_eq!(t.stages[0].verify[1].name, "test");
        assert_eq!(t.stages[0].verify[1].run, "cargo test");
    }

    #[test]
    fn omitted_new_fields_default_to_none_or_empty() {
        let src = r#"
name: x
goal: y
stages:
  - title: bare
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert!(t.stages[0].goal.is_none());
        assert!(t.stages[0].acceptance.is_none());
        assert!(t.stages[0].verify.is_empty());
        assert!(t.session_idle_timeout.is_none());
    }

    #[test]
    fn session_idle_timeout_parses_humantime_strings() {
        let src = r#"
name: x
goal: y
session_idle_timeout: 30m
stages:
  - one
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.session_idle_timeout, Some(Duration::from_secs(30 * 60)));
    }

    #[test]
    fn session_idle_timeout_parses_bare_seconds_integer() {
        let src = r#"
name: x
goal: y
session_idle_timeout: 90
stages:
  - one
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.session_idle_timeout, Some(Duration::from_secs(90)));
    }

    #[test]
    fn verify_cmd_and_verify_together_rejected() {
        let src = r#"
name: x
goal: y
stages:
  - title: build
    verify_cmd: cargo test
    verify:
      - name: check
        run: cargo check
"#;
        assert!(matches!(
            JobTemplate::parse_yaml(src),
            Err(TemplateError::Yaml(_))
        ));
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
