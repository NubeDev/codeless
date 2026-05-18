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

use std::collections::HashMap;
use std::time::Duration;

use codeless_types::pause_point::{
    PausePoint, PausePointId, PausePointPosition, PausePointTarget, TodoSelector,
};
use codeless_types::todo::TodoKind;
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
    /// Operator-declared breakpoints, held in their raw, *unvalidated*
    /// shape after `parse_yaml`. Structural YAML errors (wrong scalar
    /// type, malformed map) still surface as `TemplateError::Yaml`;
    /// semantic resolution — symbolic name → ordinal, trio-kind
    /// whitelist, duplicate detection — is deferred to
    /// [`JobTemplate::resolve_pause_points`] so the submit path
    /// collects every violation in one pass rather than
    /// short-circuiting on the first. See `DOCS/SCOPED-PAUSE-POINTS.md`
    /// §3 for the rejection table this resolver enforces.
    #[serde(default)]
    pub pause_points: Vec<RawPausePoint>,
}

/// One scoped pause entry as it sits in `template.yaml` before name
/// resolution. Distinct from `codeless_types::PausePoint` so the YAML
/// layer can be permissive (string-or-integer `stage`, optional
/// `todo`, free-form `position`) while the wire type stays strict
/// (resolved ordinals, `TodoSelector` enum, kebab-case `before` /
/// `after`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawPausePoint {
    pub stage: StageRef,
    #[serde(default)]
    pub todo: Option<TodoRef>,
    /// Required at the grammar level (SCOPED-PAUSE-POINTS §1.3). Held
    /// as `Option<String>` so a missing or non-`before`/`after` value
    /// surfaces as `ScopeError::MissingOrInvalidPosition` from the
    /// resolver rather than killing the whole YAML parse, which would
    /// hide every other schedule error in the same file.
    #[serde(default)]
    pub position: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// YAML scalar for the `stage:` key — an integer ordinal (1-based) or
/// a string name. Resolution to a canonical ordinal happens in
/// `resolve_pause_points`; this type only captures the surface shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageRef {
    Ordinal(i64),
    Name(String),
}

impl<'de> Deserialize<'de> for StageRef {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(i64),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Int(n) => Ok(StageRef::Ordinal(n)),
            Raw::Text(s) => Ok(StageRef::Name(s)),
        }
    }
}

/// YAML scalar for the optional `todo:` key. A bare integer is an
/// ordinal; a string prefixed with `~` is a title substring (the
/// tilde marks it explicitly so a runner-authored todo titled
/// `"checks"` cannot accidentally collide with the trio kind); any
/// other string is a bare word matched against the reserved trio
/// kinds (`checks` / `docs` / `git`) by the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoRef {
    Ordinal(i64),
    /// Tilde-prefixed substring with the leading `~` stripped. Empty
    /// ⇒ `ScopeError::EmptyTitleSubstring` at resolve time.
    Substring(String),
    /// Any other bare string. Held verbatim so the error message can
    /// echo the operator's exact spelling on a trio-whitelist miss.
    Word(String),
}

impl<'de> Deserialize<'de> for TodoRef {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(i64),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Int(n) => Ok(TodoRef::Ordinal(n)),
            Raw::Text(s) => match s.strip_prefix('~') {
                Some(rest) => Ok(TodoRef::Substring(rest.to_string())),
                None => Ok(TodoRef::Word(s)),
            },
        }
    }
}

/// Typed schedule-resolution failure. Surfaces every way a
/// `pause_points:` entry can violate the grammar in
/// `DOCS/SCOPED-PAUSE-POINTS.md` §3. Each variant carries the
/// operator's original spelling (not a normalised form) so the error
/// quotes the YAML verbatim and the operator can ctrl-F the file.
///
/// Returned in batches: `resolve_pause_points` collects every
/// violation it sees within a resolution pass before bailing, so the
/// operator fixes the whole schedule once rather than re-submitting
/// four times. Pass order is pinned by tests:
///
/// 1. Empty-stage-list guard (`PausePointOnEmptyStageList`).
/// 2. Stage resolution — ordinal range, name match, ambiguity.
/// 3. Position validation — `before` / `after`.
/// 4. Todo resolution — trio whitelist, ordinal floor, substring empty.
/// 5. Reason length.
/// 6. Cross-point duplicate detection over the resolved set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    UnknownStageName {
        name: String,
    },
    AmbiguousStageName {
        name: String,
        count: usize,
    },
    StageOrdinalOutOfRange {
        ordinal: i64,
        n: usize,
    },
    UnknownTrioKind {
        kind: String,
    },
    EmptyTitleSubstring,
    TodoOrdinalOutOfRange {
        stage: u32,
        ordinal: i64,
    },
    /// `found = None` ⇒ key absent; `found = Some(s)` ⇒ key present
    /// but not `before` / `after`. The two cases share a variant
    /// because they share a fix.
    MissingOrInvalidPosition {
        found: Option<String>,
    },
    /// Indices are 1-based YAML positions in `pause_points:`.
    DuplicatePausePoint {
        existing: usize,
        duplicate: usize,
    },
    ReasonTooLong {
        len: usize,
    },
    PausePointOnEmptyStageList,
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeError::UnknownStageName { name } => {
                write!(f, "pause_points: unknown stage name `{name}`")
            }
            ScopeError::AmbiguousStageName { name, count } => write!(
                f,
                "pause_points: stage name `{name}` matches {count} stages; use the ordinal"
            ),
            ScopeError::StageOrdinalOutOfRange { ordinal, n } => write!(
                f,
                "pause_points: stage ordinal {ordinal} out of range (have {n} stages, 1-based)"
            ),
            ScopeError::UnknownTrioKind { kind } => write!(
                f,
                "pause_points: todo `{kind}` is not a trio kind (expected checks, docs, or git; use `~{kind}` for a title substring)"
            ),
            ScopeError::EmptyTitleSubstring => {
                write!(f, "pause_points: empty title substring (`todo: ~`)")
            }
            ScopeError::TodoOrdinalOutOfRange { stage, ordinal } => write!(
                f,
                "pause_points: todo ordinal {ordinal} on stage {stage} must be >= 1"
            ),
            ScopeError::MissingOrInvalidPosition { found } => match found {
                Some(v) => write!(
                    f,
                    "pause_points: invalid position `{v}` (expected `before` or `after`)"
                ),
                None => write!(
                    f,
                    "pause_points: missing position (expected `before` or `after`)"
                ),
            },
            ScopeError::DuplicatePausePoint {
                existing,
                duplicate,
            } => write!(
                f,
                "pause_points: entry #{duplicate} duplicates entry #{existing} (same stage, todo, and position)"
            ),
            ScopeError::ReasonTooLong { len } => write!(
                f,
                "pause_points: reason is {len} bytes; cap is {REASON_BYTE_CAP}"
            ),
            ScopeError::PausePointOnEmptyStageList => write!(
                f,
                "pause_points: cannot schedule a pause when the template has no stages"
            ),
        }
    }
}

impl std::error::Error for ScopeError {}

/// 512 bytes per SCOPED-PAUSE-POINTS §1.4. Byte length, not character
/// count, because that's the cheap upper bound on the persisted column
/// width — a graphemes budget would invite encoding mismatches between
/// the parser and the column.
const REASON_BYTE_CAP: usize = 512;

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
    /// Per-stage persona override (D1). `Some("builtin:reviewer")`
    /// or `Some("persona_01J...")` resolves to a row in the
    /// `personas` table at job-submit time; `None` inherits the
    /// job-level persona (and, lacking that, the runner default).
    /// Bare-string stage entries cannot carry this — only the
    /// structured map form does — by design: the field is opt-in
    /// metadata and the bare form is the shorthand.
    pub persona: Option<String>,
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
                persona: None,
            }
        } else if trimmed == "REVIEW" {
            Self {
                title: String::new(),
                review: true,
                docs: None,
                goal: None,
                acceptance: None,
                verify: Vec::new(),
                persona: None,
            }
        } else {
            Self {
                title: trimmed.to_string(),
                review: false,
                docs: None,
                goal: None,
                acceptance: None,
                verify: Vec::new(),
                persona: None,
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
            // Per-stage persona override. Empty string collapses to
            // `None` so a YAML author who left the field blank gets
            // the same inheritance path as one who omitted the key.
            #[serde(default)]
            persona: Option<String>,
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
                    persona: s.persona.filter(|p| !p.trim().is_empty()),
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

    /// Resolve `pause_points:` against the parsed `stages:` list,
    /// collecting every violation in a single sweep so the operator
    /// sees the full punch list. Returns the resolved schedule keyed
    /// by YAML position so the persistence layer can write
    /// `(job_id, ordinal)` rows directly without re-walking the source.
    ///
    /// Resolution happens at submit time (the runtime calls this on
    /// the parsed template before the job leaves `draft`); a fresh id
    /// per `PausePoint` is therefore safe — the row hasn't been
    /// persisted yet, so the ULID generated here is the canonical one
    /// for the schedule. The runtime contract:
    ///
    /// - `Err(es)` ⇒ refuse the submit; the job never reaches `draft`
    ///   with a broken schedule.
    /// - `Ok(ps)` ⇒ caller writes the rows verbatim, in order.
    ///
    /// See `DOCS/SCOPED-PAUSE-POINTS.md` §3 for the rejection table
    /// and the pinned pass order.
    pub fn resolve_pause_points(&self) -> Result<Vec<PausePoint>, Vec<ScopeError>> {
        if self.pause_points.is_empty() {
            return Ok(Vec::new());
        }
        let mut errors: Vec<ScopeError> = Vec::new();
        if self.stages.is_empty() {
            errors.push(ScopeError::PausePointOnEmptyStageList);
            return Err(errors);
        }

        // Precompute case-sensitive name → ordinal index for the stage
        // name lookup. Stored as `Vec<u32>` so duplicates surface as a
        // count rather than collapsing silently.
        let n = self.stages.len();
        let mut by_name: HashMap<String, Vec<u32>> = HashMap::with_capacity(n);
        for (i, s) in self.stages.iter().enumerate() {
            let name = stage_short_name(&s.title);
            if !name.is_empty() {
                by_name.entry(name).or_default().push((i + 1) as u32);
            }
        }

        let mut resolved: Vec<PausePoint> = Vec::with_capacity(self.pause_points.len());
        let mut keys: Vec<(ResolvedKey, usize)> = Vec::with_capacity(self.pause_points.len());

        for (idx, raw) in self.pause_points.iter().enumerate() {
            let entry_no = idx + 1;
            // Pass 2: stage resolution.
            let stage_ordinal = match &raw.stage {
                StageRef::Ordinal(o) => {
                    if *o < 1 || (*o as usize) > n {
                        errors.push(ScopeError::StageOrdinalOutOfRange { ordinal: *o, n });
                        None
                    } else {
                        Some(*o as u32)
                    }
                }
                StageRef::Name(name) => match by_name.get(name).map(Vec::as_slice) {
                    Some([only]) => Some(*only),
                    Some(many) => {
                        errors.push(ScopeError::AmbiguousStageName {
                            name: name.clone(),
                            count: many.len(),
                        });
                        None
                    }
                    None => {
                        errors.push(ScopeError::UnknownStageName { name: name.clone() });
                        None
                    }
                },
            };

            // Pass 3: position validation. Parsed independently of the
            // stage so a typo in `position:` still reports against this
            // entry even when the stage name is wrong.
            let position = match raw.position.as_deref().map(str::trim) {
                Some("before") => Some(PausePointPosition::Before),
                Some("after") => Some(PausePointPosition::After),
                Some(other) => {
                    errors.push(ScopeError::MissingOrInvalidPosition {
                        found: Some(other.to_string()),
                    });
                    None
                }
                None => {
                    errors.push(ScopeError::MissingOrInvalidPosition { found: None });
                    None
                }
            };

            // Pass 4: todo resolution. Title-substring failures that
            // are *runtime-deferred* (multi-match at bind time) are
            // out of scope here — only parse-time failures land in
            // this batch.
            let selector_result: Option<Option<TodoSelector>> = match &raw.todo {
                None => Some(None),
                Some(TodoRef::Ordinal(o)) => {
                    if *o < 1 {
                        errors.push(ScopeError::TodoOrdinalOutOfRange {
                            stage: stage_ordinal.unwrap_or(0),
                            ordinal: *o,
                        });
                        None
                    } else {
                        Some(Some(TodoSelector::Ordinal { ordinal: *o as u32 }))
                    }
                }
                Some(TodoRef::Substring(pat)) => {
                    if pat.is_empty() {
                        errors.push(ScopeError::EmptyTitleSubstring);
                        None
                    } else {
                        Some(Some(TodoSelector::TitleSubstring {
                            pattern: pat.clone(),
                        }))
                    }
                }
                Some(TodoRef::Word(w)) => match trio_kind_from_word(w) {
                    Some(kind) => Some(Some(TodoSelector::Trio { kind })),
                    None => {
                        errors.push(ScopeError::UnknownTrioKind { kind: w.clone() });
                        None
                    }
                },
            };

            // Pass 5: reason length. Cheap, runs even when other
            // fields failed — same rationale as position.
            if let Some(r) = raw.reason.as_ref() {
                if r.len() > REASON_BYTE_CAP {
                    errors.push(ScopeError::ReasonTooLong { len: r.len() });
                }
            }

            // Only assemble the resolved point when every field
            // contributing to its identity is well-formed; otherwise
            // skip duplicate-detection for this entry rather than
            // emit spurious follow-on errors.
            if let (Some(stage_ord), Some(pos), Some(selector_opt)) =
                (stage_ordinal, position, selector_result)
            {
                let target = match selector_opt.clone() {
                    None => PausePointTarget::Stage { ordinal: stage_ord },
                    Some(sel) => PausePointTarget::StageTodo {
                        stage_ordinal: stage_ord,
                        selector: sel,
                    },
                };
                let key = ResolvedKey {
                    stage: stage_ord,
                    selector: selector_key(&selector_opt),
                    position: position_key(pos),
                };
                keys.push((key, entry_no));
                resolved.push(PausePoint {
                    id: PausePointId::new(),
                    target,
                    position: pos,
                    reason: raw.reason.clone(),
                });
            }
        }

        // Pass 6: cross-point duplicates. Detection runs over the
        // resolved set so two entries that *resolve* to the same
        // (stage_ordinal, selector, position) collide even when their
        // YAML spellings differ ("stage: 3" vs "stage: parser").
        let mut seen: HashMap<ResolvedKey, usize> = HashMap::with_capacity(keys.len());
        for (key, entry_no) in &keys {
            if let Some(prev) = seen.get(key) {
                errors.push(ScopeError::DuplicatePausePoint {
                    existing: *prev,
                    duplicate: *entry_no,
                });
            } else {
                seen.insert(key.clone(), *entry_no);
            }
        }

        if errors.is_empty() {
            Ok(resolved)
        } else {
            Err(errors)
        }
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
                persona: s.persona.as_deref(),
                verify: s.verify.as_slice(),
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
    /// Per-stage persona id (D1) — `builtin:<slug>` or a user
    /// persona row id. `None` inherits the job-level persona.
    /// Resolution to the actual `Persona` row happens at job-submit;
    /// the runner reads the row again at stage-run time.
    pub persona: Option<&'a str>,
    /// Verify-gate steps in declaration order. Borrowed from the
    /// stage's `verify` list; empty when the YAML omitted both
    /// `verify:` and `verify_cmd:`. The template runner uses
    /// `is_empty()` as the wire signal for "no verify gate, skip
    /// the `Checks` trio rail".
    pub verify: &'a [VerifyStep],
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

/// Derive the "name" used to address a stage from its title. Stage
/// titles in practice carry two trailing bits of decoration the
/// pause-point grammar should not require the operator to repeat:
///
/// - a size suffix `(S)` / `(M)` / `(L)` describing rough effort, and
/// - a colon-prefixed long-form sentence (`design: extend …`) where
///   the first token is the actual handle.
///
/// `stage_short_name` strips the size suffix first, then takes the
/// portion before the first colon. The result is the string the YAML
/// `stage: <name>` lookup matches against — case-sensitive, per
/// SCOPED-PAUSE-POINTS §1.1.
fn stage_short_name(title: &str) -> String {
    let trimmed = title.trim();
    let without_size = strip_size_suffix(trimmed);
    let head = match without_size.find(':') {
        Some(idx) => &without_size[..idx],
        None => without_size,
    };
    head.trim().to_string()
}

fn strip_size_suffix(s: &str) -> &str {
    let trimmed = s.trim_end();
    for suffix in [" (S)", " (M)", " (L)"] {
        if let Some(stripped) = trimmed.strip_suffix(suffix) {
            return stripped.trim_end();
        }
    }
    trimmed
}

/// Map the bare `todo:` word forms onto the reserved trio kinds.
/// Anything else returns `None` and surfaces as `UnknownTrioKind` so
/// the operator sees the misspelling instead of getting a silent
/// title-substring binding. Per SCOPED-PAUSE-POINTS §1.2.3 these
/// words are reserved at the YAML layer.
fn trio_kind_from_word(w: &str) -> Option<TodoKind> {
    match w {
        "checks" => Some(TodoKind::Checks),
        "docs" => Some(TodoKind::Docs),
        "git" => Some(TodoKind::Git),
        _ => None,
    }
}

/// Identity key for duplicate detection after resolution. The
/// `PausePointId` does *not* participate — every new entry gets a
/// fresh ULID so identity would defeat the check. Selector and
/// position are flattened to owned strings so `TodoSelector` and
/// `PausePointPosition` (which live in `codeless-types` and don't
/// derive `Hash`) don't need to grow traits just for this sweep.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResolvedKey {
    stage: u32,
    selector: Option<String>,
    position: &'static str,
}

fn selector_key(s: &Option<TodoSelector>) -> Option<String> {
    s.as_ref().map(|sel| match sel {
        TodoSelector::Ordinal { ordinal } => format!("ord:{ordinal}"),
        TodoSelector::Trio { kind } => format!("trio:{kind:?}"),
        TodoSelector::TitleSubstring { pattern } => format!("sub:{pattern}"),
    })
}

fn position_key(p: PausePointPosition) -> &'static str {
    match p {
        PausePointPosition::Before => "before",
        PausePointPosition::After => "after",
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
    fn parses_per_stage_persona_override_on_structured_form() {
        let src = r#"
name: x
goal: y
stages:
  - title: implement
    persona: "builtin:coder"
  - title: review
    persona: "builtin:reviewer"
  - title: ship
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert_eq!(t.stages[0].persona.as_deref(), Some("builtin:coder"));
        assert_eq!(t.stages[1].persona.as_deref(), Some("builtin:reviewer"));
        assert!(t.stages[2].persona.is_none());
        let planned = t.planned_stages();
        assert_eq!(planned[0].persona, Some("builtin:coder"));
        assert_eq!(planned[1].persona, Some("builtin:reviewer"));
        assert_eq!(planned[2].persona, None);
    }

    #[test]
    fn empty_persona_string_collapses_to_none() {
        let src = r#"
name: x
goal: y
stages:
  - title: implement
    persona: ""
  - title: also
    persona: "   "
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert!(t.stages[0].persona.is_none());
        assert!(t.stages[1].persona.is_none());
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

    // ---- pause_points ------------------------------------------------

    fn resolve(src: &str) -> Result<Vec<PausePoint>, Vec<ScopeError>> {
        JobTemplate::parse_yaml(src).unwrap().resolve_pause_points()
    }

    #[test]
    fn pause_points_default_to_empty_and_resolve_to_empty() {
        let src = r#"
name: x
goal: y
stages:
  - one
"#;
        let t = JobTemplate::parse_yaml(src).unwrap();
        assert!(t.pause_points.is_empty());
        assert_eq!(t.resolve_pause_points().unwrap(), Vec::new());
    }

    #[test]
    fn stage_only_ordinal_resolves_to_stage_target() {
        let src = r#"
name: x
goal: y
stages:
  - a
  - b
  - c
pause_points:
  - { stage: 2, position: before, reason: "look here" }
"#;
        let ps = resolve(src).unwrap();
        assert_eq!(ps.len(), 1);
        assert!(matches!(
            ps[0].target,
            PausePointTarget::Stage { ordinal: 2 }
        ));
        assert_eq!(ps[0].position, PausePointPosition::Before);
        assert_eq!(ps[0].reason.as_deref(), Some("look here"));
    }

    #[test]
    fn stage_name_resolves_after_stripping_size_suffix_and_colon_prefix() {
        let src = r#"
name: x
goal: y
stages:
  - "design: extend template.yaml (S)"
  - "parser (M)"
  - "persistence (M)"
pause_points:
  - { stage: parser, position: after }
  - { stage: design, position: before }
"#;
        let ps = resolve(src).unwrap();
        assert!(matches!(
            ps[0].target,
            PausePointTarget::Stage { ordinal: 2 }
        ));
        assert!(matches!(
            ps[1].target,
            PausePointTarget::Stage { ordinal: 1 }
        ));
    }

    #[test]
    fn trio_word_resolves_to_trio_selector() {
        let src = r#"
name: x
goal: y
stages:
  - one
  - two
pause_points:
  - { stage: 2, todo: docs, position: after }
  - { stage: 2, todo: checks, position: before }
  - { stage: 2, todo: git, position: after }
"#;
        let ps = resolve(src).unwrap();
        assert!(matches!(
            &ps[0].target,
            PausePointTarget::StageTodo {
                stage_ordinal: 2,
                selector: TodoSelector::Trio {
                    kind: TodoKind::Docs
                }
            }
        ));
        assert!(matches!(
            &ps[1].target,
            PausePointTarget::StageTodo {
                selector: TodoSelector::Trio {
                    kind: TodoKind::Checks
                },
                ..
            }
        ));
        assert!(matches!(
            &ps[2].target,
            PausePointTarget::StageTodo {
                selector: TodoSelector::Trio {
                    kind: TodoKind::Git
                },
                ..
            }
        ));
    }

    #[test]
    fn tilde_prefix_is_title_substring_with_prefix_stripped() {
        let src = r#"
name: x
goal: y
stages:
  - one
  - two
pause_points:
  - { stage: 2, todo: "~migrate", position: before }
"#;
        let ps = resolve(src).unwrap();
        match &ps[0].target {
            PausePointTarget::StageTodo {
                selector: TodoSelector::TitleSubstring { pattern },
                ..
            } => assert_eq!(pattern, "migrate"),
            other => panic!("expected title substring target, got {other:?}"),
        }
    }

    #[test]
    fn todo_ordinal_resolves_to_ordinal_selector() {
        let src = r#"
name: x
goal: y
stages:
  - one
pause_points:
  - { stage: 1, todo: 3, position: after }
"#;
        let ps = resolve(src).unwrap();
        assert!(matches!(
            &ps[0].target,
            PausePointTarget::StageTodo {
                stage_ordinal: 1,
                selector: TodoSelector::Ordinal { ordinal: 3 }
            }
        ));
    }

    #[test]
    fn position_keyword_after_resolves_correctly() {
        let src = r#"
name: x
goal: y
stages:
  - one
pause_points:
  - { stage: 1, position: after }
"#;
        let ps = resolve(src).unwrap();
        assert_eq!(ps[0].position, PausePointPosition::After);
    }

    #[test]
    fn unknown_stage_name_rejects() {
        let src = r#"
name: x
goal: y
stages:
  - alpha
  - beta
pause_points:
  - { stage: gamma, position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(
            errs,
            vec![ScopeError::UnknownStageName {
                name: "gamma".into()
            }]
        );
    }

    #[test]
    fn ambiguous_stage_name_rejects_with_match_count() {
        let src = r#"
name: x
goal: y
stages:
  - "core (S)"
  - "core (M)"
  - "tail"
pause_points:
  - { stage: core, position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(
            errs,
            vec![ScopeError::AmbiguousStageName {
                name: "core".into(),
                count: 2
            }]
        );
    }

    #[test]
    fn stage_ordinal_below_one_or_past_end_rejects() {
        let src = r#"
name: x
goal: y
stages:
  - a
  - b
pause_points:
  - { stage: 0, position: before }
  - { stage: 9, position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert!(errs.contains(&ScopeError::StageOrdinalOutOfRange { ordinal: 0, n: 2 }));
        assert!(errs.contains(&ScopeError::StageOrdinalOutOfRange { ordinal: 9, n: 2 }));
    }

    #[test]
    fn unknown_trio_word_rejects_and_suggests_substring_form() {
        let src = r#"
name: x
goal: y
stages:
  - a
pause_points:
  - { stage: 1, todo: typos, position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(
            errs,
            vec![ScopeError::UnknownTrioKind {
                kind: "typos".into()
            }]
        );
        // Error message names the substring escape hatch so the operator
        // sees the fix in the message rather than going hunting in DOCS.
        assert!(errs[0].to_string().contains("~typos"));
    }

    #[test]
    fn empty_tilde_substring_rejects() {
        let src = r#"
name: x
goal: y
stages:
  - a
pause_points:
  - { stage: 1, todo: "~", position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(errs, vec![ScopeError::EmptyTitleSubstring]);
    }

    #[test]
    fn todo_ordinal_below_one_rejects() {
        let src = r#"
name: x
goal: y
stages:
  - a
pause_points:
  - { stage: 1, todo: 0, position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(
            errs,
            vec![ScopeError::TodoOrdinalOutOfRange {
                stage: 1,
                ordinal: 0
            }]
        );
    }

    #[test]
    fn missing_position_rejects() {
        let src = r#"
name: x
goal: y
stages:
  - a
pause_points:
  - { stage: 1 }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(
            errs,
            vec![ScopeError::MissingOrInvalidPosition { found: None }]
        );
    }

    #[test]
    fn invalid_position_value_rejects_with_operator_spelling() {
        let src = r#"
name: x
goal: y
stages:
  - a
pause_points:
  - { stage: 1, position: midway }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(
            errs,
            vec![ScopeError::MissingOrInvalidPosition {
                found: Some("midway".into())
            }]
        );
    }

    #[test]
    fn duplicate_resolved_points_reject_with_yaml_indices() {
        let src = r#"
name: x
goal: y
stages:
  - "parser (M)"
  - other
pause_points:
  - { stage: 1, position: before }
  - { stage: parser, position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert_eq!(
            errs,
            vec![ScopeError::DuplicatePausePoint {
                existing: 1,
                duplicate: 2
            }]
        );
    }

    #[test]
    fn duplicate_detection_distinguishes_different_selectors() {
        // Same stage and position but different selectors must not
        // collide — the trio/docs and trio/git pauses are separate
        // operator intents, even though they share an ordinal.
        let src = r#"
name: x
goal: y
stages:
  - a
pause_points:
  - { stage: 1, todo: docs, position: after }
  - { stage: 1, todo: git, position: after }
"#;
        let ps = resolve(src).unwrap();
        assert_eq!(ps.len(), 2);
    }

    #[test]
    fn reason_over_byte_cap_rejects() {
        let long = "x".repeat(513);
        let src = format!(
            r#"
name: x
goal: y
stages:
  - a
pause_points:
  - {{ stage: 1, position: before, reason: "{long}" }}
"#
        );
        let errs = resolve(&src).unwrap_err();
        assert_eq!(errs, vec![ScopeError::ReasonTooLong { len: 513 }]);
    }

    #[test]
    fn reason_at_byte_cap_accepts() {
        let exact = "x".repeat(512);
        let src = format!(
            r#"
name: x
goal: y
stages:
  - a
pause_points:
  - {{ stage: 1, position: before, reason: "{exact}" }}
"#
        );
        assert!(resolve(&src).is_ok());
    }

    #[test]
    fn multiple_independent_errors_are_collected_in_one_pass() {
        // The operator should see the full punch list, not just the
        // first failure — pinned because the doc promises this exact
        // posture in SCOPED-PAUSE-POINTS §3.
        let src = r#"
name: x
goal: y
stages:
  - alpha
pause_points:
  - { stage: 9, position: before }
  - { stage: unknown, position: sideways }
  - { stage: 1, todo: nope, position: before }
"#;
        let errs = resolve(src).unwrap_err();
        assert!(errs.contains(&ScopeError::StageOrdinalOutOfRange { ordinal: 9, n: 1 }));
        assert!(errs.contains(&ScopeError::UnknownStageName {
            name: "unknown".into()
        }));
        assert!(errs.contains(&ScopeError::MissingOrInvalidPosition {
            found: Some("sideways".into())
        }));
        assert!(errs.contains(&ScopeError::UnknownTrioKind {
            kind: "nope".into()
        }));
        assert!(errs.len() >= 4);
    }

    #[test]
    fn pause_point_id_is_freshly_minted_per_resolved_entry() {
        let src = r#"
name: x
goal: y
stages:
  - a
pause_points:
  - { stage: 1, position: before }
  - { stage: 1, position: after }
"#;
        let ps = resolve(src).unwrap();
        assert_ne!(ps[0].id, ps[1].id);
    }

    #[test]
    fn stage_short_name_helper_strips_size_suffix_and_colon_prefix() {
        assert_eq!(stage_short_name("parser (M)"), "parser");
        assert_eq!(stage_short_name("design: extend foo (S)"), "design");
        assert_eq!(stage_short_name("runtime hook (L)"), "runtime hook");
        assert_eq!(stage_short_name("plain title"), "plain title");
        assert_eq!(stage_short_name("  trim me  "), "trim me");
    }
}
