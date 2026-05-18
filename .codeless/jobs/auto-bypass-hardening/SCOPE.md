# Scope — auto-bypass-hardening

This job tightens the Surface F (auto-bypass) loop on the back of two
real jobs (`01KRX4ZPF10J3QZ35R5GK8336X` plugin-substrate,
`01KRXH0RYTT6EYGF435WPQS70Q` job-chat) that both completed only
because the policy auto-bypassed a chain of failures the operator
could not see in the overview UI. The recovery worked; the **feedback
loop back to the operator** did not, and one of the recovered
failures was a SQLite infrastructure error (`code 13` = `SQLITE_FULL`)
that should never have been retried in the first place.

Authoritative references:

- [`DOCS/AUTO-BYPASS-DECISIONS.md`](../../../DOCS/AUTO-BYPASS-DECISIONS.md) — current policy + thrashing-guard contract.
- [`DOCS/JOB-WORKFLOW.md`](../../../DOCS/JOB-WORKFLOW.md) §"TODO — precheck rules reference" — the diff-verify rule that fires the false positives.
- [`DOCS/JOB-UI.md`](../../../DOCS/JOB-UI.md) — the stages-overview surface the new glyph lives on.
- [`crates/codeless-runtime/src/template_runner.rs`](../../../crates/codeless-runtime/src/template_runner.rs) — `classify_stage_failure` (line ~1956) and the three `RunnerError` emit sites.
- [`crates/codeless-runtime/src/auto_bypass_guard.rs`](../../../crates/codeless-runtime/src/auto_bypass_guard.rs) — Q1 thrashing-guard; not changed by this job, only referenced.
- [`crates/codeless-runtime/src/auto_bypass_policy.rs`](../../../crates/codeless-runtime/src/auto_bypass_policy.rs) — `policy_comment` is the prompt-thread-through site.

## Goal

After this job:

1. A SQLite infrastructure failure (`SQLITE_FULL` / `CANTOPEN` / `IOERR` /
   `CORRUPT`) halts the job with a structured `stop_reason` instead of
   being auto-bypassed. The policy never silently retries an
   infrastructure error.
2. The runtime SQLite pool runs in WAL mode with a 5s busy timeout,
   shrinking the failure surface that produced the original
   `SQLITE_FULL` (smaller write fanout, no journal rewrite per txn).
3. The pre-check diff-verify tokenizer no longer flags non-path
   tokens. The four real-world false positives (`tool.call`,
   `rest_proxy.path`, `metadata_json.delivery.slack`,
   `/home/user/.codeless/worktrees/ai-runner/Cargo.toml`) stop firing;
   genuine missing paths still do.
4. When auto-bypass advances a job, the next stage's prompt prefix
   carries the **prior stage's `failure_class` + `failure_detail`** so
   the model knows what tripped before and can avoid the same trap.
5. `StagesOverview` distinguishes "failed-and-bypassed" from
   "hard-failed" with a `~` glyph + tooltip carrying the policy and
   the failure detail; the operator sees recovery at a glance instead
   of having to drill into `StageDetail` to find out the job is fine.

## In scope

- New `FailureClass::InfrastructureError` variant in `codeless-types`,
  with the wire name `infrastructure-error`, plus codec round-trip in
  `store/codec.rs`.
- sqlx error -> `FailureClass` mapper at every existing `RunnerError`
  emit site in `template_runner.rs`. The set known today is lines
  `1051`, `1494`, `1579`; a grep at stage 1 confirms whether there are
  others.
- `classify_stage_failure` short-circuit to `FailureAction::Halt` for
  `InfrastructureError`, mirroring the existing `stop_reason.is_some()`
  short-circuit. Halt writes a structured `stop_reason` so the UI can
  label it (rather than the current generic "crash" wording).
- `SqliteStore` pool `after_connect`: `PRAGMA journal_mode=WAL`,
  `PRAGMA synchronous=NORMAL`, `PRAGMA busy_timeout=5000`. In-memory
  test stores stay in default mode (WAL on `:memory:` is a no-op /
  error depending on sqlx version).
- Precheck path tokenizer in the diff-verify pre-check — stage 1
  locates it, stage 6 hardens it. New rule: a token is path-shaped
  only if it (a) starts with a repo-relative prefix derived from the
  diff's file list (e.g. `crates/`, `ui/`, `DOCS/`), OR (b) resolves
  to an existing file under the worktree at check-time.
- `auto_bypass_policy::policy_comment` — append the prior stage's
  `failure_class` + `failure_detail` (when present) to the comment
  threaded into the next stage's prompt. The existing canned guidance
  remains the lead paragraph.
- `StagesOverview.tsx` — new `~` glyph + tooltip when
  `rollup.stage.bypassed_at !== null`. The stage row type gains a
  derived `bypassed: boolean` field; no schema change.
- `JobTimeline.tsx` — confirm the existing `stage-auto-bypassed` chip
  carries `failure_class` + `failure_detail`; extend if it doesn't.
- Unit + integration tests for every change above. Specifically:
  classification round-trip for each new error code, halt-vs-bypass
  decision under each policy, the four real-world tokenizer false
  positives, the threaded comment for `pre-check-failed` and
  `review-fail`, the UI snapshot for `~`-glyph rendering.
- Doc updates: `AUTO-BYPASS-DECISIONS.md` gains a new Q for the infra
  branch; `JOB-WORKFLOW.md` precheck-rules TODO flips to a real
  documented rule; `CODELESS.md` gains the "what works" line.

## Out of scope

- Rewriting the diff-verify precheck. We are tightening the
  tokenizer, not replacing it. A fuller overhaul (e.g. parsing the
  handover's `Done` section as Markdown and only checking links /
  code spans) is a follow-up job.
- Adding new `FailureClass` variants beyond `InfrastructureError`.
  The existing set (`PreCheckFailed`, `RunnerError`, `ReviewFail`,
  `ReviewUnparseable`, `ReviewPatchInvalid`) is unchanged.
- Touching the thrashing guard (`auto_bypass_guard.rs`). Two-strikes
  rule is unchanged. `InfrastructureError` halts upstream of the
  guard, same as cap-breach.
- Surfacing infrastructure failures over Slack/Telegram differently
  from other halts. Bot adapters already render `JobFailed` with a
  reason; the new `Infrastructure` reason flows through unchanged.
- The fuller PR / commit-message-prefix work tracked in
  `JOB-WORKFLOW.md` §"TODO — commit message conventions".
- Server-side disk-pressure alerting / preemptive halt before
  `SQLITE_FULL` fires. That belongs in a monitoring job.
- Mobile-shell rendering of the new glyph (mobile shell is Phase 6).

## Constraints

- **R1** — `InfrastructureError` and the new pool pragmas live in
  host-only crates (`codeless-runtime`). `FailureClass` is in
  `codeless-types` (iOS / Android safe) — a wire variant is fine,
  process-spawning code is not.
- **R2** — UI changes are inside existing components
  (`StagesOverview.tsx`, `JobTimeline.tsx`). No new `@tauri-apps/*`
  imports. The wire type lookup goes through the existing generated
  `wire.ts`.
- **R3** — no per-shell `.tsx`. The glyph is a single component
  branch.
- **R4** — `bypassed_at` and `failure_class` already live in SQLite;
  this job adds no new columns. The UI subscribes through the
  existing event surface.
- **R5** — single trust boundary unchanged.
- Comment rule from `codeless/CLAUDE.md` R2 — every new comment
  explains *why*, never *what*. The new InfrastructureError variant
  in particular must carry a one-line doc comment explaining why it
  exists (the operator-visible fact that we never silently retry
  infra failures), not what it stores.

## Deliverables (what "done" looks like)

1. `codeless/auto-bypass-hardening` branch with one commit per stage,
   pushed via mani; commit messages follow `stage N: <title>`.
2. `cargo test --workspace` green; the new tests for classification,
   halt-vs-bypass, tokenizer, and bypass-comment thread-through are
   all in the suite.
3. `cargo clippy --workspace --all-targets -- -D warnings` green.
4. `cargo fmt --check` green.
5. `pnpm -C ui/codeless-ui lint` green; `pnpm -C ui/codeless-ui test`
   green with updated snapshots for the new glyph + tooltip.
6. Manual smoke: with the server running, force a SQLITE_FULL by
   pointing the runtime at a DB with `PRAGMA max_page_count=1`,
   submit a one-stage job, observe the job halts with
   `stop_reason=Infrastructure` and no `stage-auto-bypassed` event
   fires. (The fixture for this lives in the stage-4 unit test; the
   manual smoke is the integration confidence check.)
7. `DOCS/AUTO-BYPASS-DECISIONS.md` and `DOCS/JOB-WORKFLOW.md` updated
   per stage 12; `CODELESS.md` "What works today" gains the
   landing-line.

## Open questions (resolved in stage 1)

Stage 1 closed every question below. Each entry keeps the original
prompt + bias, then states the **Decision** the rest of the job
builds on. A later stage that contradicts a decision here without
amending it first is a workflow failure (per `WORKFLOW.md` §"When to
halt").

### Q1 — Which SQLite error codes belong to `InfrastructureError`?

**Bias:** the four codes seen as recoverable-by-the-host —
`13 SQLITE_FULL`, `14 SQLITE_CANTOPEN`, `10 SQLITE_IOERR`,
`11 SQLITE_CORRUPT`. Codes that point at a runtime bug
(`1 SQLITE_ERROR`, `21 SQLITE_MISUSE`) stay as `RunnerError` because
they are the runner's fault, not the host's.

**Decision: accept the bias verbatim, with one widening.** The set
that maps to `InfrastructureError` at the `sqlx::Error` ->
`FailureClass` boundary is exactly the four primary codes plus their
extended siblings, matched on the *primary* code only so the mapper
does not have to enumerate every extended variant:

| Primary code | Name              | Why it is infrastructure          |
| -----------: | ----------------- | --------------------------------- |
|         `8`  | `SQLITE_READONLY` | filesystem flipped to read-only — host disk/permission, not runner logic |
|        `10`  | `SQLITE_IOERR`    | any low-level I/O failure (covers all 30+ extended `IOERR_*` variants) |
|        `11`  | `SQLITE_CORRUPT`  | file header / page invariant violated — disk or fsync truth broken |
|        `13`  | `SQLITE_FULL`     | disk or `max_page_count` reached — host has no room to advance |
|        `14`  | `SQLITE_CANTOPEN` | cannot open the DB file — perms / path / FS state, never a query bug |

Every other primary code stays in `RunnerError`. The explicit
exclusion list — the codes the mapper sees in the wild that must
**not** flip to `InfrastructureError` because they signal a runner /
query bug we want the operator to fix, not silently retry — is:

- `1 SQLITE_ERROR` (generic SQL parse / logic error — our query is wrong)
- `5 SQLITE_BUSY` (lock contention — covered by `busy_timeout`; if it still surfaces, it is a missing-await bug, not a disk issue)
- `6 SQLITE_LOCKED` (within-connection lock conflict — runner reentered itself)
- `19 SQLITE_CONSTRAINT` (unique / FK / check violation — our data is wrong)
- `20 SQLITE_MISMATCH` (type mismatch — codec bug)
- `21 SQLITE_MISUSE` (API contract violated — runner bug)
- `25 SQLITE_RANGE` (bind index out of range — runner bug)
- `26 SQLITE_NOTADB` is deliberately **not** in the infra set despite
  the surface similarity to `CORRUPT`: a non-DB file at the configured
  path is a deployment-config error the operator must see and fix,
  not a transient disk condition the policy could plausibly retry
  through. Halting on it is fine; it just lands via `RunnerError`'s
  existing halt path when the pool fails to open at startup, not via
  the new `InfrastructureError` mid-stage branch.

**Why this split:** the five infra codes are the ones where retrying
the same SQL on the same host with the same load is guaranteed not
to help — they signal host environment failure (disk, fsync, FS
perms, file invariants). Auto-bypass exists for *runner* flakes that
a fresh subprocess can plausibly survive; an infra failure that
auto-bypass retries past will just fail the next stage too, burn
tokens, and bury the original disk-full message under five more
identical ones. `SQLITE_READONLY` widens the bias because the
real-world trigger (filesystem remount to RO under disk pressure,
or container quota tripping a CoW barrier) is the same operator
intervention as `SQLITE_FULL` and should land in the same halt
branch.

**Matcher shape (for stage 3).** The classifier looks at the
primary `code()` on `sqlx::error::Error::Database(_)` (or
`SqliteError` after `try_downcast_ref`). Extended codes are
ignored. A non-`Database` `sqlx::Error` (e.g. `Io`, `Tls`,
`PoolClosed`, `PoolTimedOut`) stays in `RunnerError` for now — the
pool timeout is policy-relevant retry territory and a separate Q
would have to add it.

### Q2 — When does the WAL pragma fire?

**Bias:** in `SqlitePoolOptions::after_connect` on every connection
acquire, so a reconnect after pool churn re-applies the pragma.
`:memory:` databases opt out (WAL is not meaningful there). Note:
WAL is a per-database setting persisted in the file header;
applying it on every connect is idempotent.

**Decision: accept the bias.** The pragma set fires from
`SqlitePoolOptions::after_connect(...)` on every connection acquire,
in this exact order:

```text
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
```

`journal_mode` is the only one that is per-database-file (WAL state
lives in the header and is sticky); the other two are per-connection
and **must** re-apply on every acquire because sqlx pools recycle
connections across requests and connection-scoped pragmas reset on
the next reopen. Putting all three in `after_connect` is the
simplest contract — one hook, runs every time, no special case for
"is this the first connection."

**Opt-out rule for `:memory:`.** The hook inspects the connect URL
(`SqliteConnectOptions::filename()` plus the in-memory flag) and
skips the `journal_mode = WAL` statement when the database is in
memory. The other two pragmas still apply — `synchronous=NORMAL` is
the in-memory default and a no-op, `busy_timeout=5000` is harmless
on a single-connection in-memory DB. The skip is required because
sqlx-on-SQLite either errors or no-ops on `journal_mode=WAL`
against `:memory:` depending on the linked SQLite minor version,
and the existing test suite opens `:memory:` everywhere
(`MockRunner` harness, `state_machine` unit tests, store
round-trip tests). A blanket `WAL` would tank the suite on the
SQLite versions that error.

**Why `after_connect` and not a one-shot at pool construction:** the
sqlx pool may discard and re-establish connections under fault
(connection drop, timeout, lifetime expiry); a one-shot at
construction only configures the first connection in the pool and
silently regresses the next one. `after_connect` is the API sqlx
documents for exactly this contract, and the pragmas are cheap
enough (three round-trips on a local SQLite file) that the
per-acquire cost is invisible next to the WAL latency win on the
write path.

**Why `synchronous=NORMAL`, not `FULL`:** WAL + NORMAL is the
sqlx-documented "fast and safe enough" pairing — fsync at WAL
checkpoint boundaries instead of on every commit. We lose
durability only for transactions in the *current* WAL window if
the host hard-crashes, and the runtime already tolerates that by
crash-recovery reaping orphans (`OrphanReap` is a first-class
`FailureClass` for the same reason). The faster commit path
materially reduces the write-storm that produced the original
`SQLITE_FULL` (one fsync per checkpoint, not one per stage event).

**Why `busy_timeout=5000`:** matches the existing operator-visible
retry budgets elsewhere in the runtime and is long enough to ride
out the WAL checkpoint pause on a contended writer without making
deadlocks invisible. Anything longer hides bugs; anything shorter
re-introduces the `SQLITE_BUSY` flakes WAL is meant to suppress.

### Q3 — What is the precise tokenizer rule for path-shaped tokens?

**Bias:** a token in `Done` is "path-shaped" only if EITHER
(a) it starts with a repo-relative prefix derived from the diff's
file list (the set `{crates/, ui/, DOCS/, plugins/, bin/, setup/,
tests/, migrations/, mani.yaml, Cargo.toml, ...}` extracted from
the diff at check-time), OR
(b) it resolves to an existing file under the worktree. Anything
else — dotted RPC method names, JSON key paths, config keys — is
not path-shaped and not flagged.

**Decision: accept the bias, with the following clarifications
that lock down corner cases the current
`diff_verify::looks_path_like` already half-implements.**

A `Done`-section token (extracted by `extract_paths_from_done` per
the existing two-tier rule: backticked first, then bare in
modification-verb bullets) is considered **path-shaped, and
therefore subject to the diff-presence check**, iff it survives
the existing `looks_path_like` shape filter (whitespace-free,
leading char is alnum/`.`/`/`/`_`, plus the slash-and-extension or
filename rule) **AND** at least one of the following holds:

- **(a) Prefix match.** The token starts with one of the
  repo-relative path prefixes derived **from the current diff's
  file list**, computed once per pre-check and used as a closed
  set. The prefix set is the union of:
  - every distinct **first** path segment in the diff
    (`crates`, `ui`, `DOCS`, `plugins`, `bin`, `setup`, `tests`,
    `migrations`, `.codeless`, ...) appended with `/`;
  - every distinct **two-segment** prefix in the diff (e.g.
    `crates/codeless-runtime/`) so a `Done` bullet that names a
    sub-crate by its full prefix still matches even when only one
    file deep into it was changed;
  - the literal repo-root file names that appear in the diff
    (`Cargo.toml`, `Cargo.lock`, `mani.yaml`, `README.md`,
    `CODELESS.md`, ...) — bare repo-root filenames only count when
    they literally appear in the diff for this run, so a `Done`
    bullet that names `Cargo.toml` only flags as missing if no
    `Cargo.toml` was actually edited.
- **(b) Worktree resolves.** The token, joined onto the worktree
  root, names a path that exists on disk at pre-check time. This
  catches the case where the agent claims a file under a *new*
  top-level directory the diff already introduces (so prefix
  derivation does pick it up) **and** the case where the agent
  references a file that exists in the tree but did not change
  (legitimate context, should not flag as missing — the
  diff-presence check still runs and fails it correctly).

A token that satisfies neither (a) nor (b) is **not path-shaped**
and is dropped silently — it never reaches the diff-presence check
and therefore cannot cause a false-positive failure. The four
real-world false positives from the motivating jobs are dropped by
this rule:

| Token                              | Why it drops                                                                                   |
| ---------------------------------- | ---------------------------------------------------------------------------------------------- |
| `tool.call`                        | no `/`, no repo-prefix match, no file at `<worktree>/tool.call` — dropped at (a)+(b)            |
| `rest_proxy.path`                  | same shape as `tool.call`; bare `name.ext` with `path` as a 4-char "ext" passes `looks_path_like` today but fails (a) and (b) |
| `metadata_json.delivery.slack`     | bare dotted identifier; `looks_path_like`'s filename rule rejects it already (multi-dot bare) but the safety net of (a)+(b) keeps the rule total |
| `/home/user/.codeless/worktrees/ai-runner/Cargo.toml` | absolute path outside the worktree; (a) fails because no diff prefix starts with `/home`, (b) fails because the path is outside the worktree root — dropped |

A token that **does** belong to the diff — say a bullet that names
`crates/codeless-runtime/src/diff_verify.rs` when the diff touches
that file — passes (a) (prefix `crates/codeless-runtime/` is in
the derived set) and the diff-presence check then succeeds. A
token that names a *missing* path — say `crates/codeless-types/src/never-added.rs` —
passes (a) on the prefix and (b) fails (file does not exist) but
that is the **point**: the diff-presence check is what fires the
miss, not the tokenizer. The tokenizer's job is only to reject
non-path-shaped strings before they reach that check.

**Why per-diff prefix derivation, not a hard-coded prefix list:**
the hard-coded list bias listed `{crates/, ui/, DOCS/, plugins/,
bin/, setup/, tests/, migrations/, mani.yaml, Cargo.toml, ...}`
which is fine today but degrades the moment the repo grows a new
top-level (e.g. when the workspace gains `ai-runner/` or a future
`schemas/`). Deriving the prefix set from the diff makes the
tokenizer self-updating: any directory the agent could plausibly
have touched is by definition represented in the diff's first-
segment set, and the rule keeps working without a follow-up patch.

**Why also keep the worktree-resolves rule:** the diff is a *change*
set; an agent that legitimately references an unchanged but
existing file (e.g. "Wrote test against `crates/codeless-types/src/job.rs`")
would otherwise drop the token and the bullet would never get to
the diff-presence check at all. With (b), the tokenizer extracts
the token, the diff-presence check fires, and the bullet is
correctly flagged as a no-op claim if the file is unchanged. The
existing diff-presence-check semantics are unaffected; only the
tokenizer's intake widens to include real existing files.

**Bounds (so this stays cheap):** prefix derivation is one pass
over the diff file list (typically <100 entries). The worktree
resolve is a single `try_exists` per surviving candidate token
(typically <20 per `Done` block). No globbing, no recursion, no
new filesystem traversal beyond what the pre-check already does
to load the diff itself.

### Q4 — What exactly do we thread into the next stage's prompt on auto-bypass?

**Bias:** append a fenced block after the existing canned guidance:

```text
Previous-stage failure: <failure_class>
Detail: <failure_detail>
```

The `failure_class` is the wire name (`pre-check-failed`,
`review-fail`, etc.). `failure_detail` is the raw string already
stored on the row, truncated to 400 chars at the prompt boundary
to keep the per-stage prefix short. The threading uses the same
prompt-prefix assembler the existing operator comment uses; we are
extending the comment, not adding a new section.

**Decision: accept the bias, with the wording, ordering, and
fallback rules locked down below.**

When auto-bypass advances a job and the *prior* stage row has a
`failure_class` set, `auto_bypass_policy::policy_comment` returns
the canned policy paragraph followed by a single blank line
followed by a fenced block. The assembled string the operator-
comment plumbing receives is (illustrated below with `~~~` standing
in for the literal triple-backtick fence so this example is itself
renderable inside SCOPE.md):

~~~text
<canned policy paragraph for the selected preset, unchanged>

```
Previous-stage failure: <wire-name>
Detail: <failure_detail truncated to 400 chars; trailing "…" iff truncated>
```
~~~

The literal characters used in the assembled comment are:

- The fence is a triple-backtick fence with **no language tag** so
  it renders as plain preformatted text in every chat UI and is
  not mistakenly highlighted as code.
- `<wire-name>` is the kebab-case serde rename of the variant —
  exactly the string `serde_json::to_value(&failure_class)?`
  produces, e.g. `pre-check-failed`, `review-fail`, `runner-error`,
  `infrastructure-error`, `review-patch-invalid`,
  `review-unparseable`, `orphan-reap`. Using the wire name (not the
  Rust variant name) means the operator and the model see the same
  string the events stream carries and `grep`-the-log forensics
  match.
- `<failure_detail>` is the raw stored string, with the following
  normalisations applied **only on the prompt boundary** (the
  stored value on the stage row is unchanged):
  1. Trailing whitespace and trailing newlines stripped.
  2. Truncated to 400 Unicode scalar values (not bytes — `chars()`
     count, so a multibyte run never splits mid-codepoint). When
     truncation fires, the trailing ellipsis is the single
     character `…` (U+2026), not `...`, so the truncation marker is
     one char and the boundary calculation does not have to
     account for it.
  3. When the raw detail is empty or whitespace-only, the `Detail:`
     line is omitted entirely and only the `Previous-stage
     failure:` line is emitted inside the fence. This is the
     fallback for `OrphanReap` (whose detail is often the empty
     string) and any future class that does not always carry a
     detail.
- When `failure_class` itself is `None` (no prior failure on
  record — the previous stage `Passed`, or this is stage 0 of the
  job), the entire fenced block is omitted and `policy_comment`
  returns the canned paragraph unchanged. The thread-through never
  emits an empty fence or a fence with a placeholder.

The threading is **append**, not prepend or replace. The canned
policy guidance stays the lead paragraph because the model reads
it first and the policy's intent (Quick vs Long-term vs Cheap
etc.) is the framing for *how* to interpret the failure detail.
The fenced block sits below so the model encounters the policy
first, then the concrete failure, then the stage's own goal.

**Why a fenced block, not a `> blockquote` or a free-form
paragraph:** the fence makes the thread-through unambiguously
machine-grep-able. A later session reviewing the prompt history
(or a debugging Slack chip) can pattern-match
`^Previous-stage failure: ` inside a code fence and pull the
detail without disambiguating prose. Blockquotes get eaten by
some rendering paths; free-form paragraphs blur into the canned
guidance and the model loses the signal that this is
machine-injected context, not part of the prose.

**Why the 400-char ceiling on the prompt boundary, not the stored
value:** the stored `failure_detail` on the row is an audit trail
and may need to carry the full stack tail for post-mortem; we do
not want to lose information at write time. The prompt-side
truncation only protects the per-stage prefix budget. The
`stage_recorder` already enforces a ~200-char ceiling on the
stored value for most paths; the 400-char prompt ceiling is the
ceiling on what we **inject** and is sized larger than the stored
ceiling so the typical case is untruncated on the prompt side.
A future code path that stores a longer detail gets the prompt-
side truncation as the safety net.

**Why the wire name and not a human-readable label:** the wire
name is the canonical identifier the events stream, the chat
chip, and the timeline badge all use. Picking a different
rendering here (`Pre-check failed` vs `pre-check-failed`)
introduces a parallel vocabulary that the operator has to learn.
The wire name is ugly but consistent, and the policy paragraph
above the fence is where any human-readable framing belongs.

**Plumbing for stage 8:** the `bypass`-comment-build path that
calls `policy_comment` must load the *prior* stage's row
(`failure_class`, `failure_detail`) at bypass time and pass both
into `policy_comment` by value. The current signature
`policy_comment(policy: &AutoBypassPolicy) -> String` widens to
`policy_comment(policy: &AutoBypassPolicy, prior:
Option<&PriorFailure>) -> String` where `PriorFailure` is a small
`{ class: FailureClass, detail: String }` struct local to the
crate. `None` reproduces today's behaviour byte-for-byte so the
two integration tests that pin the bare canned strings keep
passing without edits.

## References

- Two failed-and-recovered jobs that motivated this:
  `01KRX4ZPF10J3QZ35R5GK8336X` and `01KRXH0RYTT6EYGF435WPQS70Q` in
  the events table — read the `stage-completed{failed}` +
  `stage-auto-bypassed` pairs to see what the policy did and what
  the operator could not see.
- Wire types: `codeless-types::FailureClass`,
  `codeless-types::Event::StageAutoBypassed`,
  `codeless-types::StageRow`.
- The thrashing guard contract (unchanged):
  [`auto_bypass_guard.rs`](../../../crates/codeless-runtime/src/auto_bypass_guard.rs).
- The Surface F decisions doc (this job adds a Q to it):
  [`DOCS/AUTO-BYPASS-DECISIONS.md`](../../../DOCS/AUTO-BYPASS-DECISIONS.md).
