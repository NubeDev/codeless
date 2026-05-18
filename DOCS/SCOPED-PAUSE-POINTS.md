# Scoped pause points — grammar and rejection rules

Pre-declared pause breakpoints in `template.yaml`. The runner halts at
the declared spots on its own; the operator does not click pause at
runtime. This doc is the load-bearing reference for the parser landing
in stage 4 — the in-scope brief lives in
[`.codeless/jobs/scoped-pause-points/SCOPE.md`](../.codeless/jobs/scoped-pause-points/SCOPE.md);
the wire-types land in stage 3 against this grammar.

A scoped pause is **scheduling on top of an existing primitive**. It
calls the existing `pause_job` entry with a new `StopReason` variant
(`ScopedPausePoint { point_id, label }`), emits the existing
`JobPaused` event, and clears on the existing `resume_job` RPC. The
state machine is unchanged; only the trigger is new.

## 1. Grammar

`template.yaml` grows one optional top-level key, `pause_points:`. The
value is a YAML sequence; each entry is a mapping with the fields
below. Both keys and string values are case-sensitive.

```yaml
pause_points:
  - stage: <ordinal | name>           # required
    todo:  <ordinal | trio-kind | "~name-substring">   # optional
    position: before | after          # required
    reason: <free text>               # optional
```

### 1.1 `stage:` — required

One of:

- **Ordinal** — a 1-based positive integer. `1` is the first stage in
  the `stages:` list. The ordinal must be in `1..=N` where `N` is the
  number of stages declared in the same template.
- **Name** — a string. Resolved by **case-sensitive exact match**
  against the stage's name (the first colon-prefixed token of the
  stage string, or for object-form stages the `name:` field). When
  multiple stages share a name the resolver rejects the point as
  ambiguous (see §3) — pick distinct names or fall back to an
  ordinal.

A `REVIEW` stage is a legal stage target. Pausing *before* a REVIEW
is redundant (the REVIEW already pauses for human input) but the
parser accepts it; the runtime treats the scoped pause and the REVIEW
pause as independent triggers, so the resume after a scoped pause
still hits the REVIEW gate as normal. Pausing *after* a REVIEW is
useful: it lets the operator check that the review approval landed
before the next stage spawns.

### 1.2 `todo:` — optional

When omitted, the point fires at the stage boundary (see `position:`).
When present, it narrows the trigger to one todo inside that stage,
and the point fires at the todo boundary instead of the stage
boundary.

One of:

- **Ordinal** — 1-based positive integer. Refers to the todo's
  `ordinal` field as observed at trigger time. Operator's
  responsibility to know the layout; out-of-range numbers reject only
  if the stage's todo count is fixed at submit time (the closing trio
  always reserves three ordinals, so `todo: <last-three>` always
  resolves — see §1.2.3).
- **Trio kind** — the lowercase string form of one of
  `TodoKind::TRIO`: `checks`, `docs`, `git`. These are runtime-injected
  by the recorder and always present, so a trio target is always
  resolvable at submit time.
- **Title substring** — a string prefixed with `~` (tilde). The
  matcher is case-insensitive `contains` against the todo `title`
  field, evaluated at trigger time (not submit time) because the
  runner-authored todos do not exist yet when the template is parsed.
  A match must be **unique** within the stage's todo list at the
  moment the runtime first considers the point; ambiguous matches
  reject with `ScopeError::AmbiguousTitleSubstring`. Empty substring
  (`~`) rejects at parse time.

#### 1.2.3 — trio resolution rule

`checks`, `docs`, `git` are reserved kinds. A todo selector spelled as
one of those bare words always resolves to the trio kind, even if a
runner-authored todo happens to have ordinal 1 with the title
`"checks"`. To target a non-trio todo whose title contains `checks`,
use the `~checks` (title-substring) form, which only matches against
non-trio todos by definition (the trio rows render with their kind,
not their ordinal-zero title).

### 1.3 `position:` — required

Exactly `before` or `after`. The keyword is lowercase and bare (no
quotes required, but accepted). Empty / missing / any other value
rejects with `ScopeError::MissingOrInvalidPosition`.

Semantics:

- `position: before, stage: S` — pause when the state machine has
  selected stage `S` but before the first todo for `S` begins. The
  worktree is provisioned, the runner is not yet spawned.
- `position: after, stage: S` — pause when stage `S` has completed
  its closing trio's `git` todo (the stage-completed transition) but
  before stage `S+1` is selected.
- `position: before, stage: S, todo: T` — pause when the runtime has
  selected todo `T` inside stage `S` but its handler has not yet run.
- `position: after, stage: S, todo: T` — pause when todo `T` has
  transitioned to a resolved status (`Done` or `Skipped`) but the next
  todo selection has not yet happened.

`after` on the final stage of the template means "pause once the job
is otherwise complete." The job stays in `Paused`; `resume_job`
advances it to `Completed` after the trio-gate is satisfied.

### 1.4 `reason:` — optional

Free-text. Surfaced verbatim in the chat divider label after a colon:
"before stage 3 todo checks: confirm migration didn't drop a column".
No interpolation. Length cap 512 chars; longer rejects with
`ScopeError::ReasonTooLong`.

## 2. Examples

### 2.1 Stage-only pause

```yaml
stages:
  - "design: extend template.yaml schema (S)"
  - REVIEW the schema
  - "wire types in codeless-types (S)"
  - "template parser (M)"

pause_points:
  - { stage: 3, position: before, reason: "spot-check wire types before parser code" }
```

Resolved: one point, `PausePoint { target: Stage { ordinal: 3 },
position: Before, reason: Some("spot-check...") }`. Fires when stage 3
is selected and before its first todo begins.

### 2.2 Stage + trio pause

```yaml
stages:
  - "design (S)"
  - REVIEW
  - "wire types (S)"
  - "parser (M)"
  - "persistence (M)"

pause_points:
  - stage: persistence
    todo: docs
    position: after
    reason: "verify SCOPED-PAUSE-POINTS.md picked up the table shape"
```

Resolved: the name `persistence` is matched against the stage names;
it maps to ordinal 5. The trio kind `docs` is reserved, so the
selector is `TodoSelector::Trio(TodoKind::Docs)`. Fires once the docs
todo for stage 5 resolves, before the git todo runs.

### 2.3 Stage + title-substring pause (deferred resolution)

```yaml
stages:
  - "wire types (S)"
  - "parser (M)"
  - "persistence: add scheduled_pause_points table (M)"

pause_points:
  - { stage: 3, todo: "~migrate", position: before }
```

The substring `migrate` does not match any submit-time-known todo
(the trio is `checks`/`docs`/`git`; runner-authored todos don't exist
yet). The parser accepts the point as `TodoSelector::TitleSubstring
("migrate")` and defers resolution. At runtime, when stage 3's todo
list first contains a todo whose title matches `migrate` (case-
insensitive `contains`), the point binds to that todo and fires
`before` it. If two such todos appear in the same scan tick the
runtime rejects with `ScopeError::AmbiguousTitleSubstring` and
*halts the job into `Paused` with the error in `stop_reason_message`*
rather than picking one arbitrarily — same fail-loud posture as the
parser.

## 3. Rejection rules — the parser refuses to submit a job whose `pause_points:` violates any of these.

Each rejection emits one variant of `ScopeError` (added in stage 4).
The job stays in `draft`; nothing is written to SQLite.

| `ScopeError` variant                  | Trigger                                                                                                                              |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `UnknownStageName { name }`           | `stage: <name>` did not match any stage's name.                                                                                      |
| `AmbiguousStageName { name, count }`  | `stage: <name>` matched more than one stage.                                                                                         |
| `StageOrdinalOutOfRange { ordinal, n }` | `stage: <ordinal>` with `ordinal < 1` or `ordinal > n` (n = declared stage count).                                                  |
| `UnknownTrioKind { kind }`            | `todo: <bare-word>` where the word is not `checks`, `docs`, or `git`.                                                                |
| `EmptyTitleSubstring`                 | `todo: "~"` (or `todo: ~` after YAML unquoting reduces to an empty string).                                                          |
| `AmbiguousTitleSubstring { stage, pattern, matches }` | (Runtime, not parser.) Title-substring matched multiple todos at bind time.                                          |
| `TodoOrdinalOutOfRange { stage, ordinal }` | `todo: <ordinal>` with `ordinal < 1`. Out-of-range upper bounds are deferred to runtime because runner-authored todos grow.       |
| `MissingOrInvalidPosition { found }`  | `position:` absent, or value not `before` / `after`.                                                                                 |
| `DuplicatePausePoint { existing, duplicate }` | Two entries resolve to the same `(stage_ordinal, todo_selector, position)`. The first wins is *not* the rule — the parser rejects so the duplicate is surfaced. |
| `ReasonTooLong { len }`               | `reason:` longer than 512 bytes.                                                                                                     |
| `PausePointOnEmptyStageList`          | `pause_points:` non-empty with `stages:` empty or missing.                                                                           |

Parser pass order (deterministic; tests pin this order):

1. Structural — keys present, types match, scalar shapes valid.
2. Stage resolution — ordinal range, name match, ambiguity.
3. Todo resolution — trio kind, ordinal floor, substring non-empty.
4. Cross-point — duplicate detection across the resolved set.

The parser surfaces *all* errors found in pass 1 before moving to
pass 2, so the operator sees the full list per pass rather than
fixing one and re-submitting four times. Within a pass, errors are
ordered by the entry's appearance in the YAML.

## 4. Source-of-truth and re-resolution

The parser writes one row per declared point into
`scheduled_pause_points` (schema lands in stage 5) keyed on
`(job_id, ordinal)` where `ordinal` is the index in the YAML
sequence. The row stores the *resolved* target (ordinals after
name → ordinal lookup) plus the original textual selector so a
`resync_template_from_disk` can re-diff against the YAML without
re-parsing the world.

A chat-driven on-disk edit (operator rewrites `template.yaml` from
the chat side) triggers `resync_template_from_disk` which:

1. Re-parses `pause_points:`, applying the rules above. If parsing
   fails, the job stays at its current status, the resync event
   carries the `ScopeError`, and the existing schedule is preserved.
2. Diffs the new resolved set against `scheduled_pause_points`.
   - Added rows insert.
   - Removed rows delete (only ones not yet `fired_at`).
   - Modified rows update in place.
3. Emits `pause_points_updated` on the bus so the UI re-reads.

Points whose target stage is **already past** at the moment of
resync are silent no-ops: the row inserts with `fired_at = NULL,
superseded_at = <now>` and is never considered by the runtime hook.
The resync event payload carries a one-line note listing the
silenced point ids. This matches the bias in §"Open questions" Q3.

## 5. Open-question resolutions (stage-1 decisions)

The four questions in
[`.codeless/jobs/scoped-pause-points/SCOPE.md`](../.codeless/jobs/scoped-pause-points/SCOPE.md)
resolve as follows. Per-file rationale lives in the job SCOPE; the
short form is captured here so the grammar in §1 has no holes.

1. **`position:` is required.** `pause stage 3` is ambiguous between
   "halt before the runner spawns" and "halt after the trio closes";
   forcing the keyword removes a foot-gun and keeps the YAML
   self-explaining.
2. **Title-substring selector — kept.** It is the only way to address
   runner-authored todos that don't exist at submit time. Ambiguity
   rejects loudly (parser-side for empty patterns, runtime-side for
   multi-match) so the cost is local to the unhappy path.
3. **Resync does not retroactively fire `JobPaused`** for points whose
   target already passed. They land as `superseded_at = <now>` rows;
   the resync event lists them so the operator sees the no-op.
4. **`StopReason::ScopedPausePoint` resets cost caps the same way as
   manual `pause_job`.** A scoped pause is operator intent expressed
   ahead of time; the cap-reset path stays identical to the existing
   pause primitive.

## 6. Worked example — fully resolved schedule

Given:

```yaml
stages:
  - "design (S)"
  - REVIEW
  - "wire types (S)"
  - "parser (M)"
  - "persistence (M)"
  - "runtime hook (L)"
  - REVIEW
  - "UI (M)"
  - REVIEW

pause_points:
  - { stage: 3,             position: before, reason: "spot-check wire types" }
  - { stage: parser,        todo: docs,       position: after }
  - { stage: 5,             todo: "~migrate", position: before }
  - { stage: "runtime hook", position: after, reason: "snapshot before UI" }
```

Resolved (in declaration order, the ordinal column is the YAML index
that becomes the `scheduled_pause_points.ordinal`):

| ord | stage | todo                          | pos    | reason                |
| --- | ----- | ----------------------------- | ------ | --------------------- |
| 1   | 3     | (none)                        | before | spot-check wire types |
| 2   | 4     | trio:docs                     | after  | (none)                |
| 3   | 5     | title-substring:`migrate`     | before | (none)                |
| 4   | 6     | (none)                        | after  | snapshot before UI    |

`StopReason::ScopedPausePoint` label format at trigger time:
`"<position> stage <ordinal>[ todo <selector>]: <reason>"`. Trailing
`": <reason>"` is omitted when `reason` is absent. Examples for the
table above:

- Point 1 fires as `"before stage 3: spot-check wire types"`.
- Point 2 fires as `"after stage 4 todo docs"`.
- Point 3 fires as `"before stage 5 todo migrations"` (the
  substring binds to the first matching runner-authored todo;
  the label uses the *bound* title, not the original `~migrate`
  pattern).
- Point 4 fires as `"after stage 6: snapshot before UI"`.
