# SCOPE-TELEGRAM-INTEGRATION — locked decisions

Companion to `.codeless/jobs/telegram-integration/SCOPE.md`. The job
SCOPE doc names the *what*; this doc records *which path the agent
picked at each forking stage and why*, so a later session can rebuild
the reasoning without re-deriving it from scratch.

The job's main design rationale doc
(`DOCS/SCOPE-TELEGRAM-INTEGRATION.md`) is referenced from WORKFLOW.md
but does not exist in this tree yet. The decisions below stand on
their own; if/when the design doc is written, fold these in or
cross-reference them rather than duplicating.

## Stage 2 — adapter shape (locked: Approach path 1)

### The fork

The SCOPE doc presents two ways the Telegram transport can share
code with the Slack transport:

1. **Approach 1 — extraction.** A new transport-agnostic crate
   `codeless-bot-core` owns the parser, `BotTransport` trait,
   `CommandBackend` slice of `RpcServer`, reply renderers,
   `ReplyContextMap` (the generalisation of slack's `ThreadMap`),
   and rate-limiters. `codeless-slack` and the new
   `codeless-bot` (Telegram transport) depend on it.
2. **Approach 2 — single bot crate.** One new crate `codeless-bot`
   with `transport/slack.rs` and `transport/telegram.rs` modules
   behind a `BotTransport` trait, replacing `codeless-slack`
   wholesale.

### State of slack-integration when this decision was made

- `codeless/slack-integration` branch is at `ffa11bd` (stage 4 of
  10 completed). Stages 2–4 created `crates/codeless-slack/` with
  nine source files: `command.rs`, `config.rs`, `dispatcher.rs`,
  `lib.rs`, `reply.rs`, `socket_mode.rs`, `thread_map.rs`,
  `web_api.rs`, plus `Cargo.toml`.
- That branch is **not yet merged to `master`**. `origin/HEAD`
  points at `b615115` (auto-bypass-policy merge); slack-
  integration's stage 1 commit `245a228` is not reachable from
  `master`.
- This worktree's branch (`codeless/telegram-integration`) is
  based on `master`, so the RPC-scaffolding fields the gate at
  stage 1 looks for (`ResumeJobArgs.bypass`,
  `ResumeJobArgs.next_stage_comment`, `JobResumed.actor`,
  `JobResumed.comment`) are not visible here. Stage 1 is still
  `[!]` (blocked); this stage-2 decision is paper-only and
  commits no Rust code.

### Decision

**Approach path 1 — extract `codeless-bot-core`.**

### Why

1. **Less churn against the in-flight slack-integration branch.**
   Approach 2 deletes `codeless-slack` (an existing 9-file crate
   with shipped stages 2–4) and rewrites every import as
   `codeless_bot::transport::slack::*`. Every later stage on
   `codeless/slack-integration` (stages 5–10) would have to
   rebase across that rename. Approach 1 leaves the slack crate
   structurally intact: only its `lib.rs` re-export surface
   changes when the shared types move out, and `codeless-slack`'s
   public API stays identical via re-exports from
   `codeless-bot-core`.

2. **The shipped slack code is already extraction-ready.** A read
   of the four files on `codeless/slack-integration` that will
   move out shows them naming Slack only at the *boundary*:
   - `command.rs` — the `Command` enum names RPC methods, not
     Slack verbs. The `ThreadContext` struct carries a `JobId`,
     not a Slack `thread_ts`. The only Slack-named thing in the
     parser is the leading-mention strip (`<@U…>`), and that is
     already isolated as a single helper at the head of `parse`.
   - `dispatcher.rs` — exposes a `CommandBackend` trait that is a
     pure slice of `RpcServer`. The Slack-named bits (`channel`,
     `thread_ts`, envelope decoding) are confined to the
     concrete `Dispatcher` struct; the trait moves cleanly.
   - `thread_map.rs` — keyed on `(channel: String, thread_ts:
     String)`. The shape needs widening to a generic
     `ReplyContextMap<K>` or to a typed-enum key
     `(SlackThread { channel, thread_ts } | TelegramReply {
     chat_id, message_id })`, but this is a small change inside
     one file.
   - `reply.rs` — pure formatters returning `String`. They emit
     ASCII tags (`[ok]`, `[fail]`, `[!]`) per CLAUDE.md R2 — no
     Slack mrkdwn-only syntax. Telegram can wrap the same string
     in MarkdownV2 escaping at post time.

   Approach 2 does not change the difficulty of the underlying
   abstraction; it just renames the file paths. The
   abstraction-readiness above means Approach 1 captures the
   same sharing benefit at lower mechanical cost.

3. **One transport per crate scales better than one crate per
   bot.** If a third transport (Matrix, Discord, SMS, in-app
   webhook) ever lands, Approach 1 adds a sibling crate next to
   `codeless-slack` and `codeless-bot`. Approach 2 keeps growing
   one crate's `transport/` module; the feature flags compound,
   the deps in `Cargo.toml` get conditional, and the
   "host-only" predicate has to grep inside a single crate for
   transport-specific subprocess imports. The per-crate model
   keeps R1's enforcement (`no-process-spawn-outside-adapters-
   host` is a path predicate) trivially correct.

### Planned crate / module layout (what stages 4+ should produce)

```
crates/
├── codeless-bot-core/           # NEW. Host-only. No transport.
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── command.rs           # moved from codeless-slack
│       ├── reply.rs             # moved from codeless-slack
│       ├── context_map.rs       # generalised from codeless-slack/thread_map.rs
│       ├── backend.rs           # CommandBackend trait + RpcServerBackend (moved from dispatcher.rs)
│       ├── transport.rs         # BotTransport trait
│       └── rate_limit.rs        # per-job inbound + per-chat outbound token buckets
│
├── codeless-slack/              # EXISTING. Depends on codeless-bot-core.
│   ├── Cargo.toml               # add: codeless-bot-core = { path = "../codeless-bot-core" }
│   └── src/
│       ├── lib.rs               # re-exports parse/Command/ThreadContext from core
│       ├── config.rs            # Slack-specific tokens + channel
│       ├── socket_mode.rs       # Slack-specific WebSocket pump
│       ├── web_api.rs           # Slack-specific chat.postMessage poster
│       └── dispatcher.rs        # Slack envelope decode + impl BotTransport
│
└── codeless-bot/                # NEW. The Telegram transport.
    ├── Cargo.toml               # codeless-bot-core + teloxide (or equivalent)
    └── src/
        ├── lib.rs
        ├── config.rs            # Telegram bot token, chat_id, allowed_user_ids
        ├── long_poll.rs         # getUpdates loop with backoff
        ├── post.rs              # sendMessage poster, MarkdownV2 escaping, 30 msg/sec/chat bucket
        └── dispatcher.rs        # Telegram update decode + impl BotTransport
```

### Crate name note

The SCOPE doc uses the phrase "`codeless-bot` crate with
`transport/slack.rs` and `transport/telegram.rs`" to describe
Approach 2. With Approach 1 picked, the name `codeless-bot` is
re-used for the **Telegram transport crate**, not for an umbrella
crate. The shared layer is `codeless-bot-core`. This is the same
naming relationship Rust ecosystem crates use (`tracing-core` +
`tracing`, `tower-service` + `tower`, etc.) so it should not be
confusing.

If a later reader prefers a different name for the Telegram crate
(`codeless-telegram` was ruled out by SCOPE), the rename is
strictly cosmetic — only one crate's `name` field, the workspace
member list, and the `Cargo.toml` consumer in
`codeless-cli`/`codeless-server` need updating.

### Sequencing constraint that falls out of this decision

The extraction stage MUST land *before* any Telegram transport
code. Otherwise the Telegram crate copies-and-pastes the parser /
ThreadMap / reply formatters out of `codeless-slack`, and the
"shared via codeless-bot-core" promise becomes a refactor debt
the project will never pay back.

That extraction stage cannot run inside this worktree until
slack-integration is merged to `master`. The branch-merge ban in
WORKFLOW.md (lines 27–29) is exactly the reason: doing the
extraction here against the slack-integration branch's view of
`codeless-slack` would tie the two jobs' branches together at
merge time, defeating the whole gate.

So the practical sequence is:

1. slack-integration finishes stages 5–10 on its branch and
   merges to `master`.
2. The gate at telegram-integration stage 1 re-runs and passes.
3. A new telegram-integration stage (3 in the current template?
   probably a re-numbered insertion) performs the extraction:
   creates `codeless-bot-core`, moves `command.rs` / `reply.rs`
   / `thread_map.rs` / `CommandBackend` / `RpcServerBackend`
   out of `codeless-slack`, adds re-exports, runs
   `cargo test --workspace`.
4. Stages 4–10 of the current template (scaffold telegram
   transport, parser, dispatch, event subscription, REVIEW
   context, polish) then proceed against the post-extraction
   layout.

The current `template.yaml` does not have an explicit extraction
stage between stage 2 (this one) and stage 3 ("Scaffold Telegram
transport"). The next REVIEW gate is the natural place to flag
that — either insert an extraction stage in template.yaml, or
fold the extraction into the same commit as stage 3's scaffold.
Either is fine; the constraint is that no `codeless-bot/`
directory exists in the tree before `codeless-bot-core/` does.

## Stage 3 — REVIEW gate verdict (PASS)

Stage 3 is a blocking review of the diff produced by stages 1 and 2,
not a human pause. The check is the Layer-1 invariant set named in
the stage instructions: R1 crate dependency direction, R2 single
transport per crate, R4/R5 trust boundary, wire formats untouched.

Diff under review: `e442d61..c0c59b1` (scaffold through stage 2).
Files touched, all docs:

- `.codeless/jobs/telegram-integration/SCOPE.md`
- `.codeless/jobs/telegram-integration/WORKFLOW.md`
- `DOCS/SCOPE-TELEGRAM-INTEGRATION-DECISIONS.md`
- `runs/.../handover.md`

Zero files under `crates/` were modified. `codeless-bot`,
`codeless-bot-core`, and `codeless-slack` do not exist in this
worktree (slack-integration is unmerged); `codeless-types` and
`codeless-rpc` are unchanged.

Invariant-by-invariant:

- **R1 — crate dependency direction.** No new crates, no new deps,
  no `tokio::process` / `std::process::Command` imports introduced.
  The planned layout puts `codeless-bot-core` host-only alongside
  the existing host-only crates with both transport crates
  depending on it, which is consistent with the R1 enforcement
  predicate.
- **R2 — single transport per crate.** Approach 1 was picked
  *specifically* to preserve this: one transport per crate
  (`codeless-slack` for Slack, `codeless-bot` for Telegram).
  Approach 2 (multi-transport in one crate) was rejected on the
  grounds that it compounds feature flags and weakens the
  per-crate predicate.
- **R4/R5 — operator trust boundary.** SCOPE.md preserves the
  single-operator boundary: bot token via `SecretStore` only,
  `allowed_user_ids` is defence-in-depth not authorisation,
  fail-closed if empty, `JobResumed.actor` is audit-only. No code
  exists yet that could violate it.
- **Wire formats untouched.** `crates/codeless-types/` and
  `crates/codeless-rpc/` are not in the diff. SCOPE explicitly
  states "No new RPC fields. Everything Telegram needs already
  lands in slack-integration stage 1."

Verdict: **PASS**. The job may proceed to stage 4 (scaffold
Telegram transport) once its own preconditions — slack-integration
merged to `master`, then the codeless-bot-core extraction —
clear. The closed stage-1 gate is unaffected by this verdict;
stage 4 must re-verify it before writing code.

## Open follow-ups for the REVIEW after stage 2

- Confirm with the operator that the crate name `codeless-bot`
  (rather than `codeless-telegram`) is acceptable for the
  Telegram transport. SCOPE.md is consistent with this; calling
  it out at the gate is cheap.
- Confirm the sequencing: extraction-then-telegram, with the
  extraction performed *here* on `codeless/telegram-integration`
  after slack-integration merges, rather than as a third
  parallel job.
- Confirm whether to amend `template.yaml` to insert an explicit
  extraction stage between current stages 2 and 3, or fold it
  into stage 3's commit. The former is a clearer paper trail;
  the latter avoids re-running the REVIEW machinery for a
  mechanical refactor.
- Confirm whether `JobResumed.comment` is something
  slack-integration is expected to add (WORKFLOW.md's stage-1
  gate requires it; the slack stage-1 commit `245a228`'s message
  mentions only `actor`). If `comment` is not coming from
  slack-integration, that field needs an owner before
  `codeless-bot-core` can render operator-comment audit lines
  in reply formatters.

## Stage 4 — blocked, marked `[!]`

Stage 4's outcome from `template.yaml` is:

> Scaffold Telegram transport (teloxide); bot token + chat_id +
> allowed_user_ids from secrets store; CLI
> `--enable-telegram-bot` flag plumbing; long-polling event loop.

This stage cannot write code in this worktree. Two
independent preconditions are unmet, and both are documented as
hard sequencing rules earlier in this file and in
`WORKFLOW.md`:

### Precondition A — the stage-1 RPC-scaffolding gate is still closed

`WORKFLOW.md` defines the gate as four greps that must all match
on the working tree. Re-running them against this worktree's
`HEAD` (`90d932e`, branched from `master @ b615115`):

| Grep                                                                          | Required        | Found at this `HEAD`                                                                    |
| ----------------------------------------------------------------------------- | --------------- | --------------------------------------------------------------------------------------- |
| `grep -n 'pub bypass' crates/codeless-rpc/src/methods.rs`                     | `ResumeJobArgs` | `pub bypass_failing_stage: bool` (line 142) — a *different* field, from auto-bypass-policy |
| `grep -n 'pub next_stage_comment' crates/codeless-rpc/src/methods.rs`         | `ResumeJobArgs` | (no matches)                                                                            |
| `grep -nE 'actor:.*Option<String>' crates/codeless-types/src/event.rs`        | `JobResumed`    | (no matches; `JobResumed` is lines 85–93 and has only `job_id` + `previous_reason`)     |
| `grep -nE 'comment:.*Option<String>' crates/codeless-types/src/event.rs`      | `JobResumed`    | (no matches; line 263 is `ReviewCommented.comment: String`, unrelated)                  |

Three of the four fields are absent. The fourth (`bypass`) is
present only by coincidence — `bypass_failing_stage` was added by
the merged auto-bypass-policy job, not by slack-integration, and
its semantics are different from the `bypass` field this job
expects. The gate fails.

`origin/codeless/slack-integration` is not present on origin in
this worktree's view (only `origin/codeless/telegram-integration`,
`origin/master`, and a handful of unrelated branches are
reachable). The local `codeless/slack-integration` branch named
in earlier handovers is not visible from this worktree either.
That is consistent with the WORKFLOW.md rule that slack-
integration's stage 1 must land on `master` (not on the slack
branch) before this gate opens.

### Precondition B — `codeless-bot-core` extraction has not happened

The Stage-2 decision in this same file states:

> The extraction stage MUST land *before* any Telegram transport
> code. Otherwise the Telegram crate copies-and-pastes the parser
> / ThreadMap / reply formatters out of `codeless-slack`, and the
> "shared via codeless-bot-core" promise becomes a refactor debt
> the project will never pay back. […] no `codeless-bot/`
> directory exists in the tree before `codeless-bot-core/` does.

`ls crates/` on this worktree shows neither `codeless-bot/` nor
`codeless-bot-core/` nor `codeless-slack/`. The extraction cannot
be performed here because `codeless-slack` itself is not present
on `master` — it lives only on the unmerged `codeless/slack-
integration` branch. Performing the extraction here against a
branch this worktree cannot see would either (i) require
branch-merging slack-integration in, which `WORKFLOW.md` and
`CLAUDE.md` R4 forbid, or (ii) re-create `codeless-slack` from
scratch inside this branch, which would fork the slack code into
two divergent copies.

### What would have to happen for stage 4 to unblock

The sequence laid out under the Stage-2 "Sequencing constraint"
section still applies, with no shortcuts available from this
worktree:

1. slack-integration finishes its remaining stages on its own
   branch and merges to `master`. That ships
   `ResumeJobArgs.bypass`, `ResumeJobArgs.next_stage_comment`,
   `JobResumed.actor`, and `JobResumed.comment`, plus the
   `crates/codeless-slack/` directory with the nine files named
   in the Stage-2 inventory.
2. This worktree rebases (or a fresh worktree is opened) on the
   updated `master`. The four greps in `WORKFLOW.md` then pass.
3. A `codeless-bot-core` extraction stage runs in this job. It
   creates `crates/codeless-bot-core/`, moves `command.rs`,
   `reply.rs`, `thread_map.rs` (renamed `context_map.rs`),
   `CommandBackend`, and `RpcServerBackend` out of
   `codeless-slack`, adds re-exports, runs the full verify trio
   (`cargo test --workspace`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo fmt --check`), commits.
   This is the stage the Stage-2 open follow-ups asked the
   operator to confirm — insert as a new stage in
   `template.yaml`, or fold into stage 4's commit.
4. Only *then* does this stage 4 ("scaffold Telegram transport")
   actually create `crates/codeless-bot/` with `Cargo.toml`,
   `lib.rs`, `config.rs`, `long_poll.rs`, `post.rs`,
   `dispatcher.rs`, the `teloxide` dependency, the
   `--enable-telegram-bot` CLI flag in `codeless-cli`, and the
   secrets-store reads for the bot token + chat ID +
   allowed-user-IDs.

### What this stage produces

This commit. The decisions doc gains this Stage-4 section so the
audit trail records *why* stage 4 was halted (not just that it
was). No `crates/` files are touched; no `Cargo.toml` is edited;
no Rust is written. `cargo test --workspace` was not run for the
same reason — there is no Rust change in this stage to verify.

Stage 4 is marked `[!]` (blocked) per `CLAUDE.md` R4 ("Do not
commit a partial implementation with a TODO."). The job halts
here and waits for the two preconditions above. The next session
that picks this stage up must re-run the four-grep gate before
writing any code; if it still fails, the next session also
halts.

## Stage 5 — blocked, marked `[!]`

Stage 5's outcome from `template.yaml` is:

> Implement Telegram command parser for Surface 1 grammar:
> `/status`, `/start`, `/stop`, `/resume` with bypass/comment;
> `reply_to_message_id` job-ID resolution mirroring Slack's
> `thread_ts` logic.

This stage cannot write code in this worktree. It inherits every
blocker that halted stage 4, plus a stage-5-specific reason
called out as an anti-pattern in `WORKFLOW.md`.

### Precondition A (carried from stage 4) — gate still closed

The four `WORKFLOW.md` greps were re-run against this worktree's
`HEAD` (`acc8dd0`, branched from `master @ b615115`). The result
is identical to the table recorded under "Stage 4 — blocked" in
this file:

- `grep -n 'pub bypass' crates/codeless-rpc/src/methods.rs` →
  only `pub bypass_failing_stage: bool` (line 142), the unrelated
  auto-bypass-policy field.
- `grep -n 'pub next_stage_comment' crates/codeless-rpc/src/methods.rs`
  → no matches.
- `grep -nE 'actor:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.
- `grep -nE 'comment:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.

Three of four required fields absent. The gate is still closed.
Slack-integration has not landed on `master` between the previous
session (`acc8dd0`) and this one (`fetch origin` reports
`origin/master` unchanged at `b615115`; `origin/codeless/slack-
integration` is still not visible). The `Command` enum stage 5
would emit needs `ResumeJobArgs.bypass` and
`ResumeJobArgs.next_stage_comment` as its destination types in
stage 6, so building the parser against fields that do not exist
on `master` would either invent placeholder field names (which
the next session would have to rewrite) or be untestable end-
to-end against the RPC server.

### Precondition B (carried from stage 4) — no `codeless-bot-core`

`ls crates/` still returns no `codeless-bot/`, `codeless-bot-core/`,
or `codeless-slack/`. The Stage-2 sequencing constraint stands
verbatim: no `codeless-bot/` directory may exist before
`codeless-bot-core/` does, and the extraction cannot run here
until `codeless-slack` is on `master`.

### Precondition C (new at stage 5) — the parser cannot land outside `codeless-bot-core`

Stage 5 is the first stage whose output is *the shared
abstraction itself* — the parser, `Command` enum, and reply-
context map. `WORKFLOW.md` calls this out under "Anti-patterns
specific to this job":

> **Re-implementing the parser inside `transport/telegram.rs`.**
> The whole point of the shared adapter is that the parser is
> written once. If the Telegram-side grammar diverges from
> Slack's, fix the parser, not by duplicating it.

The Stage-2 decision recorded in this file picked Approach 1
specifically so the parser lives in `codeless-bot-core` and both
transports re-export it. Implementing a standalone Telegram
parser in `crates/codeless-bot/src/command.rs` *now*, before the
extraction stage, would:

1. Fork the parser. The Slack version (`codeless-slack/src/command.rs`,
   visible on the unmerged `codeless/slack-integration` branch at
   commit `129054a`, 766 lines including 20 unit tests covering
   mention-stripping, case folding, both context branches of
   every verb, keyword-vs-comment disambiguation, escape
   handling, unicode comments, and every error variant) and the
   Telegram version would drift the moment either side fixed a
   bug. The "shared via codeless-bot-core" promise becomes
   refactor debt that the project has explicitly committed to
   avoid.

2. Misalign the `Command` enum with the RPC argument types it is
   supposed to feed. The Slack parser emits
   `Command::ResumeJob { job_id, bypass, comment }` mapping
   directly to `ResumeJobArgs.bypass` and
   `ResumeJobArgs.next_stage_comment` (precondition A above).
   With neither field present, the Telegram-only `Command` enum
   would have to invent its own field names, then either rewrite
   them to match the canonical ones once stage 1 lands, or feed
   the dispatcher fake intermediate types — neither is
   acceptable under CLAUDE.md R4.

3. Foreclose the only generalisation that makes the Slack and
   Telegram parsers a single piece of code: `ThreadContext`
   widening to `ReplyContext` keyed on a typed-enum
   (`SlackThread { channel, thread_ts }` or `TelegramReply {
   chat_id, message_id }`). The stage-5 task explicitly names
   `reply_to_message_id` as the Telegram-side analogue of
   `thread_ts`; that mapping belongs inside
   `codeless-bot-core::context_map`, not in two parallel
   per-transport files.

The stage-5 task also names a small but load-bearing grammar
delta from Slack: Telegram messages arrive with `/`-prefixed
verbs (BotFather convention) instead of bare verbs, and Telegram
has no analogue of Slack's `<@U…>` leading-mention envelope.
Both deltas are one-line changes inside a shared parser
(strip-leading-slash vs. strip-leading-mention helper, swapped
at the entry point); they are not justifications for a separate
parser file.

### What would have to happen for stage 5 to unblock

Same sequence as stage 4, with stage 4 added at the front:

1. slack-integration finishes its remaining stages on its own
   branch and merges to `master`. Ships
   `ResumeJobArgs.bypass`, `ResumeJobArgs.next_stage_comment`,
   `JobResumed.actor`, `JobResumed.comment`, and the
   `crates/codeless-slack/` directory (nine source files,
   including the 766-line `command.rs` with its 20 unit tests).
2. This worktree rebases (or a fresh worktree opens) on the
   updated `master`. The four-grep gate then passes.
3. The `codeless-bot-core` extraction runs: creates
   `crates/codeless-bot-core/`, moves `command.rs`, `reply.rs`,
   `thread_map.rs` (renamed `context_map.rs`), `CommandBackend`,
   and `RpcServerBackend` out of `codeless-slack`, adds
   re-exports, runs the verify trio, commits.
4. Stage 4 (scaffold Telegram transport) creates
   `crates/codeless-bot/` with `Cargo.toml`, `lib.rs`,
   `config.rs`, `long_poll.rs`, `post.rs`, `dispatcher.rs`, the
   `teloxide` dep, and the `--enable-telegram-bot` CLI flag.
5. *Then* this stage 5 ("Telegram command parser for Surface 1")
   makes its three concrete additions inside `codeless-bot-core`:

   a. Generalise `ThreadContext` to `ReplyContext` keyed on a
      transport-tagged enum, with `SlackThread { channel,
      thread_ts }` and `TelegramReply { chat_id, message_id }`
      variants both resolving to the same optional `JobId`.
      Existing `ThreadContext` is kept as a type alias so the
      Slack call sites don't churn.
   b. Extend the `parse` entry point to accept an optional
      leading-`/` strip (Telegram's BotFather convention) in
      addition to the existing leading-`<@U…>` strip (Slack's
      mention envelope). The cleanest shape is a `ParseOptions`
      struct with a `strip_leading_slash: bool` field passed
      from the per-transport dispatcher; each transport flips
      one flag at its call site.
   c. Add Telegram-specific unit tests in
      `codeless-bot-core/src/command.rs` that cover the slash-
      prefixed verbs, the `reply_to_message_id`-based short
      forms (`/stop`, `/resume`, `/resume bypass`,
      `/resume "<comment>"`), and the cold-context
      `MissingJobId` cases. The Slack tests on the same module
      stay green unchanged.

   No new file is created under `crates/codeless-bot/src/` for
   the parser. The Telegram dispatcher imports the parser
   verbatim from `codeless_bot_core::command::parse` and only
   supplies the Telegram-shaped `ReplyContext` it constructed
   from the inbound update.

### What this stage produces

This commit. The decisions doc gains this Stage-5 section so the
audit trail records *why* stage 5 was halted in the same form as
stage 4's record. No `crates/` files are touched; no `Cargo.toml`
is edited; no Rust is written. `cargo test --workspace` was not
run for the same reason — there is no Rust change in this stage
to verify.

Stage 5 is marked `[!]` (blocked) per `CLAUDE.md` R4. The job
halts here and waits for the four preconditions above (A, B,
plus stage 4's own completion). The next session that picks this
stage up must re-run the four-grep gate before writing any code,
confirm `crates/codeless-bot-core/src/command.rs` exists (from
the post-extraction stage), and only then add the three concrete
parser extensions listed under "What would have to happen for
stage 5 to unblock" above; if any precondition still fails, that
session also halts.

## Stage 6 — blocked, marked `[!]`

Stage 6's outcome from `template.yaml` is:

> Wire parsed commands through the shared `BotTransport` trait to
> `RpcClient` calls; format and post synchronous command replies.

This stage cannot write code in this worktree. It inherits every
blocker that halted stages 4 and 5, plus two stage-6-specific
constraints rooted in the Stage-2 decision and `WORKFLOW.md`.

### Precondition A (carried) — gate still closed

The four `WORKFLOW.md` greps were re-run against this worktree's
`HEAD` (`c2c4efb`, branched from `master @ b615115`). The result
is identical to the tables recorded under "Stage 4 — blocked"
and "Stage 5 — blocked" in this file:

- `grep -n 'pub bypass' crates/codeless-rpc/src/methods.rs` →
  only `pub bypass_failing_stage: bool` (line 142), the unrelated
  auto-bypass-policy field.
- `grep -n 'pub next_stage_comment' crates/codeless-rpc/src/methods.rs`
  → no matches.
- `grep -nE 'actor:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.
- `grep -nE 'comment:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.

`fetch origin` reports `origin/master` unchanged at `b615115`;
`origin/codeless/slack-integration` is still not visible.
Slack-integration's stage 1 commit `245a228` has not landed on
`master` between the previous session (`c2c4efb`) and this one.
The `RpcServer` methods stage 6 would call still do not accept
the `bypass` and `next_stage_comment` arguments the parser is
expected to emit, so the dispatcher → RPC seam has no contract
to wire against.

### Precondition B (carried) — no `codeless-bot-core`, no `codeless-bot`

`ls crates/` still returns no `codeless-bot/`, `codeless-bot-core/`,
or `codeless-slack/`. The Stage-2 sequencing constraint stands
verbatim: no `codeless-bot/` directory may exist before
`codeless-bot-core/` does, and the extraction cannot run here
until `codeless-slack` is on `master`. Stage 6's deliverable
is the *Telegram-side dispatcher inside* `crates/codeless-bot/`,
so there is no directory to add files to.

### Precondition C (carried) — no Telegram parser

Stage 5 is blocked, so the Telegram-flavoured extensions of the
`codeless-bot-core` parser (slash-stripping `ParseOptions`,
`ReplyContext::TelegramReply { chat_id, message_id }`) do not
exist. Stage 6 consumes those types at the dispatcher entry point
when it decodes the inbound Telegram update into a `ReplyContext`
and calls `parse(text, ParseOptions { strip_leading_slash: true,
… }, reply_ctx)`. Without them the dispatcher cannot be written
in a form that survives stage 5 landing later.

### Precondition D (new at stage 6) — no `BotTransport` trait

The stage-6 task names *the shared `BotTransport` trait* as the
seam the parsed commands flow through to `RpcClient` and back out
as posted replies. That trait is one of the six items the
Stage-2 decision lists for the `codeless-bot-core` extraction:

> ```
> crates/codeless-bot-core/src/
> ├── transport.rs       # BotTransport trait
> ├── backend.rs         # CommandBackend trait + RpcServerBackend
> ├── command.rs         # parser
> ├── reply.rs           # renderers
> ├── context_map.rs     # generalised ThreadMap
> └── rate_limit.rs
> ```

Neither `transport.rs` nor `backend.rs` exists in this worktree
(no `codeless-bot-core/` at all). The slack-side analogues that
will move out (`codeless-slack/src/dispatcher.rs` defines
`CommandBackend` + `RpcServerBackend`; the trait the dispatcher
posts replies through is still inlined as the `ChatPoster` struct
in `codeless-slack/src/web_api.rs`, not yet a trait) live only on
the unmerged `codeless/slack-integration` branch at commit
`ffa11bd`. The extraction stage is what generalises `ChatPoster`'s
shape into `BotTransport`, with Slack providing one impl
(`chat.postMessage` over `https://slack.com/api`) and Telegram
providing the other (`sendMessage` over
`https://api.telegram.org/bot<token>/`).

Writing a Telegram-only `BotTransport` definition inside
`crates/codeless-bot/` now would commit the project to a trait
shape that the extraction is then forced to either ratify or
rewrite — either outcome wastes work and risks the two halves
drifting in the same way the parser duplication risks.

### Precondition E (new at stage 6) — no `CommandBackend`, no `RpcServer` in this worktree's bot path

The dispatcher's other input is `CommandBackend` — the
5-method slice of `RpcServer` (`list_jobs`, `get_job`,
`start_job`, `stop_job`, `resume_job`) the bot actually calls.
The Slack stage-4 commit (`ffa11bd`) defines this trait at
`codeless-slack/src/dispatcher.rs:46-56` with a blanket
`RpcServerBackend` that wraps `Arc<dyn RpcServer>` so the
in-process runtime drives the bot in production while tests fake
five methods instead of ~80. The extraction moves this trait
into `codeless-bot-core::backend`; stage 6 imports it from there.
Both the trait and the blanket impl are absent from this
worktree because their slack-side originals are not on `master`.

### What stage 6 will actually produce when it unblocks

The Slack equivalent (commit `ffa11bd`, +1560/-61 lines across
9 files) is the working template. The Telegram stage 6 maps to
it file-for-file via the post-extraction layout. The expected
shape:

```
crates/codeless-bot/src/
├── dispatcher.rs   # NEW — Telegram update decode → reply_to_message_id
│                   #   lookup → parse() → CommandBackend call → reply
│                   #   render → BotTransport::send_message
├── post.rs         # NEW — sendMessage poster impl of BotTransport;
│                   #   MarkdownV2 escaping; per-chat 30 msg/sec bucket
│                   #   (the rate-limit bucket itself lives in
│                   #   codeless-bot-core::rate_limit, this file is the
│                   #   Telegram-side consumer)
├── long_poll.rs    # CHANGED — handle_update spawns dispatcher into a
│                   #   detached task per update so a slow reply post
│                   #   does not stall the next getUpdates batch
├── lib.rs          # CHANGED — TelegramBot::spawn grows an
│                   #   Arc<dyn RpcServer> parameter; exposes
│                   #   reply_context_map() clone for stage 8's outbound
│                   #   notification poster
└── config.rs       # unchanged at this stage
```

And, on the consumer side:

```
crates/codeless-cli/src/serve.rs
  # CHANGED — when --enable-telegram-bot is set, pass
  # state.rpc.clone() into TelegramBot::spawn so Telegram
  # commands hit the same RpcServer code path the web UI uses.
```

The dispatcher path mirrors the Slack one exactly, with the
transport-specific bits parameterised through `BotTransport`:

1. Decode the inbound Telegram `Update`, extract `chat.id`,
   `from.id`, `text`, and `reply_to_message.message_id`.
2. Reject if `from.id` is not in the configured
   `allowed_user_ids` set (defence-in-depth, fail-closed per
   `SCOPE.md`).
3. Resolve the reply-context job id via the
   `ReplyContextMap::get(ReplyContext::TelegramReply {
   chat_id, message_id: reply_to_message_id })` lookup.
4. Call `codeless_bot_core::command::parse(text,
   ParseOptions { strip_leading_slash: true, … }, reply_ctx)`.
5. Dispatch on the resulting `Command` to one of
   `CommandBackend`'s five methods.
6. Render the result via `codeless_bot_core::reply::*`,
   wrapping the returned `String` in MarkdownV2 escaping before
   handing it to `BotTransport::send_message(chat_id,
   reply_to_message_id, text)`.
7. Posting failures are logged and dropped — same rationale as
   the Slack stage-4 dispatcher: the runtime has already
   advanced, so refusing to ack the update would just produce a
   duplicate dispatch on Telegram's retry of the next
   `getUpdates`, which is the wrong recovery for a transient
   post failure against state the runtime has already mutated.

Tests live with the code (R5):

- `dispatcher.rs` gets per-`Command` unit tests using a fake
  `CommandBackend` (the five-method seam) and a recording
  `BotTransport` (capturing `send_message` calls in-memory).
  No HTTP and no Tokio runtime spawn the dispatcher itself
  needs; the long-poll integration is tested separately.
- `post.rs` gets a `wiremock`-backed test pointing at the same
  base-URL pattern the production `TelegramPoster` uses,
  asserting that the JSON body contains the correct
  `chat_id`, `reply_to_message_id`, `parse_mode: "MarkdownV2"`,
  and that MarkdownV2-reserved characters in the rendered text
  are backslash-escaped per
  <https://core.telegram.org/bots/api#markdownv2-style>.
- Telegram-specific reply tests are *not* added in
  `crates/codeless-bot/`. The renderers live in
  `codeless_bot_core::reply` and are transport-agnostic; the
  Slack tests added at slack stage 4 stay the only renderer
  tests, and Telegram inherits the rendering correctness
  guarantee through the shared crate (see SCOPE-MUTABLE-UI R2:
  the reply body emits ASCII tags `[ok]` / `[fail]` / `[!]`,
  not Slack-mrkdwn-only syntax, so the same string is correct
  in both transports once Telegram's MarkdownV2 escape pass
  wraps the metadata characters).

### What would have to happen for stage 6 to unblock

Same sequence as stage 5, with stage 5 added at the front:

1. slack-integration finishes its remaining stages on its own
   branch and merges to `master`. Ships
   `ResumeJobArgs.bypass`, `ResumeJobArgs.next_stage_comment`,
   `JobResumed.actor`, `JobResumed.comment`, and the
   `crates/codeless-slack/` directory (nine source files
   including stage-4's dispatcher + reply + thread_map +
   web_api).
2. This worktree rebases (or a fresh worktree opens) on the
   updated `master`. The four-grep gate then passes.
3. The `codeless-bot-core` extraction runs: creates
   `crates/codeless-bot-core/`, moves `command.rs`, `reply.rs`,
   `thread_map.rs` (renamed `context_map.rs`), `CommandBackend`,
   `RpcServerBackend`, and generalises `ChatPoster` into the
   `BotTransport` trait. Slack's `ChatPoster` becomes the
   `impl BotTransport for SlackChatPoster` in
   `codeless-slack/src/web_api.rs`. Verify trio, commit.
4. Stage 4 (scaffold Telegram transport) creates
   `crates/codeless-bot/` with `Cargo.toml`, `lib.rs`,
   `config.rs`, `long_poll.rs`, `post.rs`, and the `teloxide`
   (or hand-rolled `reqwest`) dep, plus the
   `--enable-telegram-bot` CLI flag.
5. Stage 5 (Telegram parser extensions) adds the three concrete
   widenings inside `codeless-bot-core` enumerated in that
   stage's blocked-doc section (ReplyContext enum,
   ParseOptions, Telegram-flavoured unit tests).
6. *Then* this stage 6 makes its two concrete additions:

   a. `crates/codeless-bot/src/dispatcher.rs` with the
      6-step pipeline above, plus the per-`Command` unit tests
      against the fake `CommandBackend` + recording
      `BotTransport`.
   b. `crates/codeless-bot/src/post.rs` with the Telegram
      `BotTransport` impl, MarkdownV2 escaping, and the
      `wiremock`-backed JSON-body test.
   c. Touch `crates/codeless-bot/src/lib.rs` to grow
      `TelegramBot::spawn` an `Arc<dyn RpcServer>` parameter
      and expose `reply_context_map()` for stage 8's outbound
      notification poster (the writer-side; stage 6 only reads).
   d. Touch `crates/codeless-cli/src/serve.rs` to pass
      `state.rpc.clone()` into `TelegramBot::spawn` when
      `--enable-telegram-bot` is set.

   The verify trio (`cargo test --workspace`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo fmt
   --check`) must be green before commit.

### What this stage produces

This commit. The decisions doc gains this Stage-6 section so the
audit trail records *why* stage 6 was halted in the same form as
stages 4 and 5's records. No `crates/` files are touched; no
`Cargo.toml` is edited; no Rust is written. `cargo test
--workspace` was not run for the same reason — there is no Rust
change in this stage to verify.

Stage 6 is marked `[!]` (blocked) per `CLAUDE.md` R4. The job
halts here and waits for the five preconditions above (A through
E, including stages 4 and 5's own completion). The next session
that picks this stage up must re-run the four-grep gate before
writing any code, confirm `crates/codeless-bot-core/src/{backend,
transport,command,reply,context_map}.rs` all exist (from the
post-extraction and stage-5 commits), confirm
`crates/codeless-bot/src/{lib,config,long_poll,post}.rs` all
exist (from stage 4) — and only then add the two new dispatcher /
post files listed under "What would have to happen for stage 6
to unblock" above; if any precondition still fails, that session
also halts.

## Stage 8 — blocked, marked `[!]`

Stage 8's outcome from `template.yaml` is:

> Subscribe to event bus; post outbound failure notifications on
> `JobFailed` and `JobStopped` via Telegram with structured format;
> per-job 5-minute debounce; respect Telegram's 30 msg/sec/chat
> outbound rate limit.

This stage cannot write code in this worktree. It inherits every
blocker that halted stages 4, 5 and 6, plus three stage-8-specific
constraints that turn directly on what slack-integration's analogue
commit (`ec766c5`, stage 6 on the unmerged `codeless/slack-
integration` branch) introduced.

(Stage 7 between stage 6 and this one is a REVIEW gate from
`template.yaml`: "REVIEW after command surface is working
end-to-end from a real Telegram client". It has no code deliverable.
The end-to-end Telegram client run it would gate is impossible to
record while stages 4–6 are `[!]` and no `crates/codeless-bot/`
exists; the gate's verdict is the same one stages 4–6 record —
preconditions unmet — and is implicit in the per-stage blocked
sections rather than re-stated here.)

### Precondition A (carried) — gate still closed

The four `WORKFLOW.md` greps were re-run against this worktree's
`HEAD` (`378ebf6`, branched from `master @ b615115`). The result
is identical to the tables under "Stage 4 — blocked", "Stage 5 —
blocked", and "Stage 6 — blocked":

- `grep -n 'pub bypass' crates/codeless-rpc/src/methods.rs` →
  only `pub bypass_failing_stage: bool` (line 142), the unrelated
  auto-bypass-policy field.
- `grep -n 'pub next_stage_comment' crates/codeless-rpc/src/methods.rs`
  → no matches.
- `grep -nE 'actor:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.
- `grep -nE 'comment:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.

`git fetch origin` between sessions reports `origin/master`
unchanged at `b615115`; `origin/codeless/slack-integration` is
still not visible. Slack-integration's stage 1 commit `245a228`
has not landed on `master` between the previous session
(`378ebf6`) and this one. Three of the four required fields stay
absent. The gate is still closed.

The publisher this stage would write subscribes to the event bus
and emits messages on `Event::JobFailed` and `Event::JobStopped`;
those two variants already exist on `master` (`codeless-types`
ships them), so the *subscription* side of stage 8 does not need
the four scaffolding fields. But the publisher's commit also
re-registers the posted message ts (Telegram: returned `message_id`)
in the shared `ReplyContextMap` so a bare-verb reply
(`resume bypass`, `stop`) inside the notification thread resolves
to the failing job id without the operator retyping it. The bare-
verb branch is exactly what `ResumeJobArgs.bypass` and
`ResumeJobArgs.next_stage_comment` parameterise on the inbound
side; landing the outbound half without the inbound half would
break the load-bearing loop the publisher's `ReplyContextMap`
registration exists for. The gate stays load-bearing for stage 8.

### Precondition B (carried) — no `codeless-bot-core`, no `codeless-bot`, no `codeless-slack`

`ls crates/` on this worktree:

```
codeless-adapters-host  codeless-mcp         codeless-runtime         codeless-tools
codeless-cli            codeless-predicates  codeless-server          codeless-types
codeless-client         codeless-rpc         codeless-tauri-desktop
```

No `codeless-bot/`, no `codeless-bot-core/`, no `codeless-slack/`.
The Stage-2 sequencing constraint stands verbatim: no
`codeless-bot/` directory may exist before `codeless-bot-core/`
does, and the extraction cannot run here until `codeless-slack`
is on `master`. Stage 8's deliverable is the outbound publisher
*plus its renderers* — the renderers must live in
`codeless-bot-core::reply::notify` (mirror of
`codeless-slack/src/notify.rs` at slack stage 6) and the publisher
core in `codeless-bot-core::outbound`, with only the Telegram-side
post call swapped through `BotTransport`. There is no
`codeless-bot-core/` directory to add either file to.

### Precondition C (carried) — no Telegram parser, no `ReplyContext::TelegramReply`

Stage 5 is blocked, so `codeless-bot-core::context_map::ReplyContext`
with its `TelegramReply { chat_id, message_id }` variant does not
exist. The publisher's `ReplyContextMap` registration step
(`publisher.register(ReplyContext::TelegramReply { chat_id,
message_id: posted_message_id }, job_id)` on every successful
top-level post) is the same map the dispatcher reads in stage 6 to
resolve a bare-verb reply. Without `ReplyContext` widened from
`ThreadContext`, the publisher and dispatcher cannot share a single
keyspace and the bare-verb-reply loop the publisher's whole reason
for existing closes silently.

### Precondition D (carried) — no `BotTransport` trait

The stage-6 record names `BotTransport` as the seam parsed commands
flow through; stage 8 names it as the seam the publisher posts
through. Slack's `ChatPoster::post` grew a `PostedMessage` return
value at slack stage 6 (`ec766c5`, `web_api.rs` +88 lines) so the
publisher could capture the new top-level `ts` for `ThreadMap`
registration. The extraction stage's job is to generalise that
return type to a transport-tagged enum:

```
pub enum PostedReply {
    Slack { channel: String, thread_ts: String },
    Telegram { chat_id: i64, message_id: i64 },
}
```

so the publisher takes one `Arc<dyn BotTransport>` and registers
the right `ReplyContext` variant in the right `ReplyContextMap`
keyspace without branching on the transport. Until that trait
exists in `codeless-bot-core`, the publisher cannot be written in
a form that survives the extraction landing later.

### Precondition E (carried) — no `CommandBackend`, no `RpcServer` in this worktree's bot path

The publisher's *enrichment* call path uses two `RpcServer`
methods the slack stage 6 commit names explicitly:
`get_job(job_id)` for the job row + cost + cost-cap, and
`list_stages(job_id)` for the failing stage's ordinal and title.
The slack publisher captures these behind a new `EventSource`
trait so its tests don't drag in the full ~80-method `RpcServer`
surface. The extraction stage moves `EventSource` (and its
production blanket impl `RpcServerEventSource`) into
`codeless-bot-core::backend` alongside `CommandBackend` and
`RpcServerBackend`. Both seams are needed for stage 8's tests to
fake five methods (subscribe + get_job + list_stages on the
publisher; start_job + stop_job + resume_job on the dispatcher)
instead of stubbing the whole RPC trait. Neither exists in this
worktree.

### Precondition F (new at stage 8) — no outbound publisher pattern in `codeless-bot-core`

Stage 8 is the first stage whose output is the *publisher* itself
— the per-bot background task that subscribes once, filters to
two event variants, debounces per job, enriches via two RPC calls,
renders, posts, registers the returned reply context, and never
forwards `StageStarted` / `AiToken` / `JobCompleted`. Per Stage 2,
the publisher and its `DEBOUNCE_WINDOW` constant belong in
`codeless-bot-core::outbound`, with the Telegram crate doing only
the transport wiring (long-poll task plus the `BotTransport` impl
that wraps `sendMessage`). The slack-integration analogue at
`ec766c5` is the working template: `codeless-slack/src/outbound.rs`
is 782 lines (new file at slack stage 6); `codeless-slack/src/
notify.rs` is 254 lines (also new at slack stage 6). Both files
live only on the unmerged `codeless/slack-integration` branch.

Implementing the publisher inside `crates/codeless-bot/src/` now
— rather than waiting for it to be extracted into
`codeless-bot-core` — would fork it for the same reason stages 5
and 6 cite for the parser and dispatcher: the Slack and Telegram
publishers would diverge the first time either side fixed a
debounce / enrichment / replay-policy bug, and the
"shared via codeless-bot-core" promise becomes refactor debt.

### Precondition G (new at stage 8) — no `EventSource` seam, no notify renderers

Two additional `codeless-bot-core` items that ship in the slack
stage-6 commit and must be extracted before stage 8 can land:

1. **`codeless-bot-core::backend::EventSource`** — the trait
   `subscribe(filter: EventFilter, since: Option<EventCursor>) ->
   EventStream`. Slack stage 6 introduces this exactly to keep the
   publisher's unit tests off the full `RpcServer` surface; both
   transports' tests need it for the same reason. Currently lives
   in `codeless-slack/src/outbound.rs` on the unmerged branch.

2. **`codeless-bot-core::reply::notify::{format_job_failed,
   format_job_stopped}`** — pure renderers that take a resolved
   `Job` row plus an optional `StageRollup` and return a `String`
   matching the Surface-1 mockup block:

   ```
   [!] Job "scope-mutable-ui" - Failed at stage 8/13
       Stage:  "REVIEW after per-job action loop"
       Reason: <stop_reason or "failed">
       Cost:   $52.64 / $150.00 cap
       Reply in this thread: resume bypass | resume "<comment>" | stop
   ```

   These renderers emit pure ASCII (`[!]` per CLAUDE.md R2; no
   emojis, no Slack mrkdwn-only syntax). The Telegram crate wraps
   the returned `String` in MarkdownV2 escaping at post time
   (`post.rs` from stage 4), so the same renderer output is
   correct in both transports. Currently lives in
   `codeless-slack/src/notify.rs` on the unmerged branch.

Stage 8 must not duplicate either of these inside
`crates/codeless-bot/`. Both belong in `codeless-bot-core`.

### Precondition H (new at stage 8) — no per-chat 30 msg/sec outbound bucket

Telegram's documented outbound limit (30 messages / second / chat,
per <https://core.telegram.org/bots/faq#my-bot-is-hitting-limits-
how-do-i-avoid-this>) is the stage-8 task's named rate constraint.
The Stage-2 plan put this under
`codeless-bot-core::rate_limit` alongside the inbound per-job
debounce ("per-job inbound + per-chat outbound token buckets").
Neither bucket exists yet. The slack-side analogue is unbounded:
Slack's Web-API rate budget for `chat.postMessage` is per-app, not
per-channel, and slack stage 6 does not introduce a token bucket
on the outbound path. Telegram's 30 msg/sec/chat policy is the
first place a bucket actually matters, so this stage *adds* it
to `codeless-bot-core::rate_limit` rather than extracts it — but
the receiving crate still has to exist before the bucket can land.

The bucket sits between the publisher's "I have a message to
post" call and the `BotTransport::send_message` HTTP call, keyed
by `chat_id`. Implementation shape (for the session that picks
this up post-extraction): a `tokio::sync::Mutex<HashMap<i64,
TokenBucket>>` inside the Telegram poster (`crates/codeless-bot/
src/post.rs` from stage 4), with the bucket itself defined in
`codeless-bot-core::rate_limit::TokenBucket { capacity: 30,
refill_per_second: 30 }`. The publisher does not see the bucket;
it calls `send_message` and the poster yields the calling task
until a token is available. This keeps the publisher transport-
agnostic (no `chat_id`-keyed state in `codeless-bot-core::
outbound`) and contains the Telegram-specific quota inside the
Telegram crate.

### What stage 8 will actually produce when it unblocks

The Slack equivalent (`ec766c5`, +1227/-65 lines across 6 files)
is the working template. The Telegram stage 8 maps to it with the
generalisation already chosen at Stage 2:

```
crates/codeless-bot-core/src/
├── outbound.rs       # MOVED from codeless-slack/src/outbound.rs;
│                     #   transport-agnostic publisher. Subscribes via
│                     #   EventSource, filters to JobFailed / JobStopped,
│                     #   debounces per job (5 min), enriches via
│                     #   get_job + list_stages, renders via
│                     #   reply::notify, posts via BotTransport,
│                     #   registers returned ReplyContext in
│                     #   ReplyContextMap. No transport branches.
├── reply/
│   └── notify.rs     # MOVED from codeless-slack/src/notify.rs;
│                     #   format_job_failed + format_job_stopped pure
│                     #   string renderers, ASCII-tag output, no
│                     #   transport-specific markup.
├── backend.rs        # ADDS EventSource trait + RpcServerEventSource
│                     #   blanket impl alongside CommandBackend /
│                     #   RpcServerBackend (from stage-6's extraction).
├── transport.rs      # ADDS BotTransport::send_message return value
│                     #   PostedReply enum (Slack { channel, thread_ts }
│                     #   | Telegram { chat_id, message_id }) so the
│                     #   publisher can register the right
│                     #   ReplyContext variant transport-agnostically.
├── context_map.rs    # ADDS register/get for the
│                     #   ReplyContext::TelegramReply variant; the Slack
│                     #   variant continues to work via the existing
│                     #   ThreadMap-shaped storage.
└── rate_limit.rs     # ADDS per-chat 30 msg/sec/chat TokenBucket type
                      #   used by the Telegram crate's post.rs.

crates/codeless-bot/src/
├── lib.rs            # CHANGED — TelegramBot::spawn grows the
│                     #   subscription side: spawns
│                     #   codeless_bot_core::outbound::Publisher with
│                     #   the same Arc<dyn RpcServer> that drives the
│                     #   dispatcher, the BotTransport poster from
│                     #   post.rs, and the shared ReplyContextMap from
│                     #   stage 6.
├── post.rs           # CHANGED — TelegramPoster::send_message returns
│                     #   PostedReply::Telegram { chat_id, message_id };
│                     #   wraps each call in a per-chat TokenBucket
│                     #   (capacity 30, refill 30/sec); MarkdownV2
│                     #   escape pass unchanged from stage 6.
└── outbound.rs       # NEW — Telegram-specific thin shim, if any
                      #   chat-side glue is needed beyond the BotTransport
                      #   impl. Likely empty: the slack equivalent's
                      #   transport-specific bits all sit in
                      #   web_api.rs (post + post return shape), so
                      #   the Telegram-side equivalent is post.rs and
                      #   no new file is needed.

crates/codeless-slack/src/
├── lib.rs            # CHANGED — re-exports outbound::Publisher,
│                     #   reply::notify::*, EventSource from
│                     #   codeless-bot-core so existing Slack call sites
│                     #   don't churn.
├── outbound.rs       # DELETED — moved to codeless-bot-core.
├── notify.rs         # DELETED — moved to codeless-bot-core.
└── web_api.rs        # CHANGED — ChatPoster impl BotTransport returns
                      #   PostedReply::Slack { channel, thread_ts: ts }
                      #   from each post (the slack stage-6 return value
                      #   widened to the transport-tagged enum).
```

Tests live with the code (R5):

- `codeless-bot-core/src/outbound.rs` keeps slack stage 6's full
  test surface verbatim — the publisher's tests use the
  `EventSource` + `CommandBackend` + recording `BotTransport`
  fakes, no HTTP. The tests do not change shape; only the imports
  and the `Arc<dyn BotTransport>` parameter generalise.
- `codeless-bot-core/src/reply/notify.rs` keeps the renderer
  string-match tests verbatim (header lines, cost / cap formatting,
  reply-options trailer, missing-rollup degradation). The output is
  ASCII; both transports inherit correctness.
- `crates/codeless-bot/src/post.rs` adds a `wiremock`-backed test
  asserting that posting two messages to the same `chat_id`
  within < 1/30s blocks the second call until a token is
  available (drive the test clock with a `tokio::time::pause()`
  block and an explicit `advance` to assert the bucket releases).
- `crates/codeless-bot/src/lib.rs` adds a `Publisher` spawn test
  with a fake event source emitting one `JobFailed`, asserting
  the recording `BotTransport` saw one `send_message` and the
  `ReplyContextMap` got one `TelegramReply` registration.

The verify trio (`cargo test --workspace`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo fmt --check`)
must be green before commit.

### What would have to happen for stage 8 to unblock

Same sequence as stage 6, with two new items at the tail:

1. slack-integration finishes its remaining stages on its own
   branch and merges to `master`. Ships `ResumeJobArgs.bypass`,
   `ResumeJobArgs.next_stage_comment`, `JobResumed.actor`,
   `JobResumed.comment`, and the `crates/codeless-slack/` directory
   (now 11 files post-slack-stage-6: the original 9 plus
   `outbound.rs` and `notify.rs`).
2. This worktree rebases (or a fresh worktree opens) on the
   updated `master`. The four-grep gate then passes.
3. The `codeless-bot-core` extraction runs — and at stage 8 the
   extraction set widens by two files: it moves `outbound.rs` and
   `notify.rs` out of `codeless-slack` alongside `command.rs`,
   `reply.rs`, `thread_map.rs`, `CommandBackend`,
   `RpcServerBackend`, and the new `EventSource` trait. The
   `ChatPoster`-to-`BotTransport` generalisation grows the
   `PostedReply` enum return value.
4. Stage 4 (scaffold Telegram transport) creates
   `crates/codeless-bot/` with `Cargo.toml`, `lib.rs`,
   `config.rs`, `long_poll.rs`, `post.rs`, plus the `teloxide`
   (or hand-rolled `reqwest`) dep and the `--enable-telegram-bot`
   CLI flag.
5. Stage 5 (Telegram parser extensions) adds the three concrete
   widenings inside `codeless-bot-core` enumerated in that stage's
   blocked-doc section.
6. Stage 6 (dispatcher + Telegram poster) adds the inbound
   command pipeline and the `BotTransport` impl, with
   `TelegramBot::spawn` exposing `reply_context_map()` for stage 8
   to share.
7. Stage 7's REVIEW gate runs with a real end-to-end Telegram
   client trace and either passes or sends the job back to a
   prior stage.
8. *Then* this stage 8 makes its concrete additions:

   a. Move `outbound.rs` and `notify.rs` into `codeless-bot-core`
      as part of the same commit that adds the Telegram-side
      wiring (or — if the operator prefers an explicit extraction
      stage per the Stage-2 open follow-up — as the tail of the
      extraction commit, leaving this stage to do only the
      Telegram-side wiring).
   b. Generalise `BotTransport::send_message` return value to
      `PostedReply` enum.
   c. Add `codeless-bot-core::rate_limit::TokenBucket` for the
      per-chat 30 msg/sec/chat outbound limit (the inbound per-job
      bucket from Stage 2's `rate_limit.rs` plan may also land
      here or stay deferred — slack stage 6 has no inbound
      bucket either, since the slack dispatcher is naturally
      paced by the operator's typing).
   d. Spawn the publisher from `TelegramBot::spawn` (in
      `crates/codeless-bot/src/lib.rs`) with the
      Telegram-shaped `BotTransport`, the shared `ReplyContextMap`
      from stage 6, and the same `Arc<dyn RpcServer>` the
      dispatcher uses.
   e. Wrap each `TelegramPoster::send_message` call in the
      per-chat `TokenBucket` (in `crates/codeless-bot/src/
      post.rs`).
   f. Tests per the section above.

   The verify trio must be green before commit.

### What this stage produces

This commit. The decisions doc gains this Stage-8 section so the
audit trail records *why* stage 8 was halted in the same form as
stages 4, 5 and 6's records. No `crates/` files are touched; no
`Cargo.toml` is edited; no Rust is written. `cargo test
--workspace` was not run for the same reason — there is no Rust
change in this stage to verify.

Stage 8 is marked `[!]` (blocked) per `CLAUDE.md` R4. The job halts
here and waits for the eight preconditions above (A through H,
including stages 4, 5, 6 and the stage-7 REVIEW gate's own
completion). The next session that picks this stage up must:

1. Re-run the four-grep gate before writing any code.
2. Confirm `crates/codeless-bot-core/src/{backend,transport,
   command,reply,context_map,rate_limit,outbound}.rs` and
   `crates/codeless-bot-core/src/reply/notify.rs` all exist
   (from the extended extraction).
3. Confirm `crates/codeless-bot/src/{lib,config,long_poll,post,
   dispatcher}.rs` all exist (from stages 4 and 6).
4. Only then add the publisher spawn in `crates/codeless-bot/
   src/lib.rs`, the per-chat `TokenBucket` wrap in
   `crates/codeless-bot/src/post.rs`, and the matching tests.

If any precondition still fails, that session also halts.

## Stage 9 — blocked, marked `[!]`

Stage 9's outcome from `template.yaml` is:

> Surface 2: enrich failure notifications with REVIEW gate context
> (`ReviewPreCheck` / `ReviewVerdict` event data) when available;
> render structured block as Markdown V2 preformatted text.

This stage cannot write code in this worktree. It inherits every
blocker that halted stages 4, 5, 6 and 8, plus three stage-9-
specific constraints that turn on what slack-integration's analogue
commit (`68799e0`, stage 7 on the unmerged `codeless/slack-
integration` branch, +798 / -21 lines across `notify.rs` +
`outbound.rs` + `lib.rs`) introduced and where the Telegram-side
work diverges from it.

### Precondition A (carried) — gate still closed

The four `WORKFLOW.md` greps were re-run against this worktree's
`HEAD` (`43eb338`, branched from `master @ b615115`). The result
is identical to the tables under "Stage 4 — blocked" through
"Stage 8 — blocked":

- `grep -n 'pub bypass' crates/codeless-rpc/src/methods.rs` →
  only `pub bypass_failing_stage: bool` (line 142), the unrelated
  auto-bypass-policy field.
- `grep -n 'pub next_stage_comment' crates/codeless-rpc/src/methods.rs`
  → no matches.
- `grep -nE 'actor:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.
- `grep -nE 'comment:.*Option<String>' crates/codeless-types/src/event.rs`
  → no matches.

`git fetch origin` between sessions reports `origin/master`
unchanged at `b615115`; `origin/codeless/slack-integration` is
still not visible. The branch-base merge-base check
(`git merge-base HEAD origin/master`) returns `b615115`, the same
commit the prior six stages' "blocked" sections recorded. Three
of the four required fields stay absent. The gate is still
closed.

The renderer this stage extends takes its `ReviewContext` argument
through the same path the publisher's enrichment branch (stage 8)
adds. Without stages 1–8's scaffolding the renderer signature and
its call site both fail to compile, and the new Markdown V2
preformatted-block pass in the Telegram poster has nothing to
wrap — there is no `notify::format_job_failed` in this worktree
to call.

### Precondition B (carried) — no `codeless-bot-core`, no `codeless-bot`, no `codeless-slack`

`ls crates/` on this worktree:

```
codeless-adapters-host  codeless-mcp         codeless-runtime         codeless-tools
codeless-cli            codeless-predicates  codeless-server          codeless-types
codeless-client         codeless-rpc         codeless-tauri-desktop
```

No `codeless-bot/`, no `codeless-bot-core/`, no `codeless-slack/`.
The Stage-2 sequencing constraint stands verbatim. Stage 9's
renderer changes must land inside
`codeless-bot-core::reply::notify` (mirror of
`codeless-slack/src/notify.rs` at slack stage 7, file is 585 lines
post-stage-7); the cache + capture branches must land inside
`codeless-bot-core::outbound` (mirror of
`codeless-slack/src/outbound.rs` at slack stage 7, file is 1226
lines post-stage-7); the Markdown V2 preformatted-block wrap must
land inside `crates/codeless-bot/src/post.rs` (created at stage 4,
which is itself `[!]`). None of those files exist here.

### Precondition C (carried) — no `ReviewContext` arg on the renderer signature

Stage 8 names `codeless-bot-core::reply::notify::{format_job_failed,
format_job_stopped}` as the two pure renderers it moves out of
`codeless-slack`. Stage 9's first concrete change is to widen each
renderer's signature with a trailing `review: Option<&ReviewContext>`
argument (this is exactly the diff `68799e0` makes on `notify.rs`:
the `Option<&ReviewContext>` parameter is appended to both
renderers, and every call site at the publisher's
`post_notification` path is updated in the same commit). Without
the file existing in `codeless-bot-core/`, the signature cannot be
widened; without the publisher (stage 8 `[!]`), there is no call
site to update.

### Precondition D (carried) — no enrichment-event capture branch in the publisher

The publisher's `handle_envelope` function in
`codeless-bot-core::outbound` is the seam where stage 9 inserts
the two enrichment-event match arms. Slack's stage-7 diff against
`outbound.rs` (`@@ async fn handle_envelope`) adds:

```rust
match &event {
    Event::ReviewPreCheck { stage_id, outcome } => {
        reviews.lock().await
            .record_pre_check(*stage_id, outcome.clone());
        return;
    }
    Event::ReviewVerdict { stage_id, verdict } => {
        reviews.lock().await
            .record_verdict(*stage_id, verdict.clone());
        return;
    }
    _ => {}
}
```

Both arms `return` early — Surface 2 events are pure enrichment
and never produce a top-level post on their own (the firehose
policy from Surface 1 is preserved verbatim). Until `outbound.rs`
lives in `codeless-bot-core` with its `handle_envelope` body, the
two arms have nowhere to land.

### Precondition E (carried) — no `EventSource` + `CommandBackend` test fakes

Stage 8 cites both seams as needed for the publisher's tests to
fake five methods (subscribe + get_job + list_stages on the
publisher; start_job + stop_job + resume_job on the dispatcher)
without stubbing the whole `RpcServer`. Stage 9's tests build on
the same fakes — `TestSource` from slack stage 7's `outbound.rs`
implements both `EventSource` and `CommandBackend` and exposes
`seed_jobs` / `seed_stages` / `events: Mutex<Vec<Envelope>>` so
the `wiremock`-backed integration tests can replay a
`ReviewPreCheck` + `ReviewVerdict` + `JobFailed` triple and assert
the captured POST body. Without these fakes in
`codeless-bot-core`, the tests have nothing to drive.

### Precondition I (new at stage 9) — no `ReviewContext` type, no `ReviewCache` type

Two new types ship in `68799e0` that stage 9 must add to
`codeless-bot-core` (and that have no equivalent on `master`):

1. **`codeless_bot_core::reply::notify::ReviewContext`** — a small
   pair-of-options struct:

   ```rust
   #[derive(Debug, Clone, Default, PartialEq, Eq)]
   pub struct ReviewContext {
       pub pre_check: Option<PreCheckOutcome>,
       pub verdict: Option<ReviewVerdict>,
   }

   impl ReviewContext {
       pub fn is_empty(&self) -> bool {
           self.pre_check.is_none() && self.verdict.is_none()
       }
   }
   ```

   Both fields are independently optional: a model-driven `Fail`
   arrives with only `verdict`; a pre-check auto-fail arrives with
   both; a `Skipped` / `NothingToVerify` pre-check arrives with
   only `pre_check`. `is_empty()` is the renderer's short-circuit
   for the non-REVIEW failure case. The type is `pub` so the
   publisher (in `outbound.rs`) and the renderers (in
   `reply/notify.rs`) can name it across the module boundary; it
   also re-exports through `codeless-bot-core::lib.rs` so the
   downstream Slack and Telegram crates can name it without
   reaching into private paths.

2. **`codeless_bot_core::outbound::ReviewCache`** — a bounded FIFO
   keyed by `StageId`:

   ```rust
   struct ReviewCache {
       capacity: usize,
       entries: HashMap<StageId, ReviewContext>,
       order: Vec<StageId>,
   }
   ```

   With `record_pre_check`, `record_verdict`, `upsert<F>`, and
   `take(stage_id) -> Option<ReviewContext>`. The capacity is
   `pub const REVIEW_CACHE_CAPACITY: usize = 1024` (re-exported
   through `codeless-bot-core::lib.rs`). `take` is destructive on
   hit so a future retry of the same stage starts from a clean
   cache; non-REVIEW failures miss the cache and the renderer
   collapses the Surface 2 block (the SCOPE doc names
   notifications as additive — the events table is the durable
   record, an evicted entry that the renderer never used is not a
   data loss). Neither type exists in this worktree.

### Precondition J (new at stage 9) — no gate-type label table

Stage 9's renderer derives a single `Type:` line from the
`(pre_check, verdict)` pair so the operator reads
"diff-verify pre-check auto-fail" vs "model rejected handover" vs
"runtime auto-fail (sentinel)" at a glance. The mapping table
lives inside `notify::review_type_label` in `68799e0`:

| `pre_check`                              | `verdict`                | label                                       |
| ---------------------------------------- | ------------------------ | ------------------------------------------- |
| `Some(Fail { .. })`                      | `Some(AutoFail { .. })`  | `"diff-verify pre-check auto-fail"`         |
| `Some(Fail { .. })`                      | other                    | `"diff-verify pre-check failed"`            |
| any                                      | `Some(Fail { .. })`      | `"model rejected handover"`                 |
| any                                      | `Some(AutoFail { .. })`  | `"runtime auto-fail"`                       |
| any                                      | `Some(Pass { .. })`      | `"review passed (failure elsewhere)"`       |
| `Some(Skipped)`                          | `None`                   | `"diff-verify pre-check skipped"`           |
| `Some(NothingToVerify)`                  | `None`                   | `"nothing to verify"`                       |
| `Some(Pass { .. })`                      | `None`                   | `None` (no Type line)                       |
| `None`                                   | `None`                   | `None` (caller should not have rendered)    |

The table is identical to slack's because the renderer is shared.
Telegram does not customise the labels; only the wrapping at post
time differs. Without `notify.rs` in `codeless-bot-core`, the
table has nowhere to land.

### Precondition K (new at stage 9) — no MarkdownV2 preformatted-block wrap in the Telegram poster

This is the one place Telegram's stage 9 actually diverges from
slack's stage 7. Slack's transport posts plain text and the
Surface 2 block renders as ASCII lines inline. Telegram's
transport sends Markdown V2 (the parse mode the stage-4 poster
sets so `[!]` does not trip on the literal `!` and so the
`reply_to_message_id → job_id` map's bare-verb hint hides nicely
in italic). Markdown V2 has 18 reserved characters that must be
escaped outside a code block (per
<https://core.telegram.org/bots/api#markdownv2-style>):
``_ * [ ] ( ) ~ ` > # + - = | { } . !``. Inside a preformatted
(triple-backtick) block the only escapes needed are backtick and
backslash. So the Surface 2 structured lines, which contain `:`,
`-`, and `.`, naturally belong in a preformatted block — the
operator sees fixed-width text and the escape rule drops from
"escape 18 chars" to "escape 2 chars".

The renderer cannot do this wrapping itself: it is shared with
the Slack transport, which must continue to emit plain ASCII. So
the renderer's signature must widen further than slack's stage 7
needed:

```rust
pub struct RenderedNotification {
    pub prelude: String,      // header + Stage: line + (no Surface 2)
    pub block: Option<String>,// Type / Missing paths / Verdict lines
    pub postlude: String,     // Reason / Cost / Reply hint
}

pub fn format_job_failed(
    job: &Job,
    stage: Option<&StageRollup>,
    total_stages: Option<u32>,
    review: Option<&ReviewContext>,
) -> RenderedNotification;
```

with the matching shape for `format_job_stopped`. The Slack
poster (`web_api::ChatPoster::post`) concatenates
`prelude + block_or_empty + postlude` verbatim, recovering its
prior behaviour. The Telegram poster (`crates/codeless-bot/src/
post.rs`) does:

1. MarkdownV2-escape `prelude` (18-char escape pass).
2. If `block.is_some()`, MarkdownV2-escape it with the *code-
   block* rules (only `` ` `` and `\`), then wrap in triple
   backticks on their own lines, with a leading blank line and
   no language tag.
3. MarkdownV2-escape `postlude`.
4. Concatenate.

The reason for refactoring the renderer's return type rather than
sentinel-parsing the slack-stage-7-shape `String` in the
Telegram poster: sentinel parsing couples the poster to the
renderer's exact output text, and any future format tweak to the
Surface 2 block in `codeless-bot-core::reply::notify` would
silently break the Telegram wrap. The struct return type makes
the seam explicit and survives renderer evolution.

This refactor — by changing `notify`'s return type — is the part
of stage 9 that fans out into the slack crate as well: slack's
stage-7-shaped `format_job_failed(...) -> String` must move to
`format_job_failed(...) -> RenderedNotification` and the slack
poster must learn to concatenate. The slack-side change is
mechanical and lands in the same commit as the Telegram-side
addition; the assertions in slack's stage-7 wiremock tests
(`assert!(text.contains("Type:   ..."))` and the bare-Surface-1
negative tests) are preserved verbatim because the concatenated
output is byte-identical to what slack stage 7 produced.

The MarkdownV2 escape passes themselves do not yet exist in this
worktree: stage 4 (creating `crates/codeless-bot/src/post.rs`) is
blocked, so neither the 18-char nor the code-block escape pass
has a host file. The escape passes are short — `String::with_
capacity` + character-by-character — and slack does not need
them, so they live only in the Telegram crate.

### What stage 9 will actually produce when it unblocks

The slack equivalent (`68799e0`, +798 / -21 lines across 3 files)
is the working template. The Telegram stage 9 maps to it plus the
renderer-return-type refactor plus the Telegram-poster wrap:

```
crates/codeless-bot-core/src/
├── reply/
│   └── notify.rs     # CHANGED — adds pub struct ReviewContext
│                     #   (+impl is_empty), pub struct
│                     #   RenderedNotification (prelude / block /
│                     #   postlude), widens format_job_failed /
│                     #   format_job_stopped to take
│                     #   Option<&ReviewContext> and return
│                     #   RenderedNotification, adds private
│                     #   render_review_block + review_type_label
│                     #   helpers, +15 unit tests pinning every
│                     #   row of the gate-type label table plus
│                     #   the is_empty short-circuit plus the
│                     #   bare-block (non-REVIEW) shape.
└── outbound.rs       # CHANGED — adds pub const
                      #   REVIEW_CACHE_CAPACITY: usize = 1024;
                      #   adds struct ReviewCache with
                      #   record_pre_check / record_verdict /
                      #   upsert / take; threads
                      #   Arc<Mutex<ReviewCache>> through run_loop
                      #   + handle_envelope + post_notification;
                      #   matches Event::ReviewPreCheck /
                      #   Event::ReviewVerdict before the terminal-
                      #   variant check and returns early; takes
                      #   the cache entry on the terminal envelope
                      #   and passes Option<&ReviewContext> to the
                      #   renderer; +3 unit tests on ReviewCache
                      #   (record + take, FIFO eviction at
                      #   capacity, take-on-miss) and +3 wiremock-
                      #   backed integration tests (
                      #   ReviewPreCheck+ReviewVerdict+JobFailed
                      #   renders Surface 2 block, bare JobFailed
                      #   renders Surface 1 only, two review-only
                      #   envelopes produce zero posts).

crates/codeless-bot-core/src/
└── lib.rs            # CHANGED — pub use reply::notify::{
                      #   ReviewContext, RenderedNotification};
                      #   pub use outbound::REVIEW_CACHE_CAPACITY;
                      #   so the Slack and Telegram crates can
                      #   name them without reaching into private
                      #   paths.

crates/codeless-bot/src/
├── post.rs           # CHANGED — TelegramPoster::send_message
│                     #   accepts the RenderedNotification return
│                     #   value rather than a flat String;
│                     #   escapes prelude / postlude with the
│                     #   18-char MarkdownV2 pass; wraps block (if
│                     #   present) in triple backticks with the
│                     #   code-block 2-char escape pass; +2 unit
│                     #   tests asserting the wrapped output
│                     #   roundtrips through Telegram's reference
│                     #   parser (one Surface 2 case, one Surface
│                     #   1 only) and that the code-block escape
│                     #   leaves the structured lines unchanged
│                     #   (no `:` / `-` / `.` escaping inside the
│                     #   block).
└── lib.rs            # UNCHANGED on Surface 2 — the publisher
                      #   spawn from stage 8 already wires the
                      #   shared ReplyContextMap and the Arc<dyn
                      #   RpcServer>; the only change visible at
                      #   the spawn site is the renderer's return
                      #   type, which propagates through the
                      #   BotTransport seam without touching this
                      #   file.

crates/codeless-slack/src/
├── notify.rs         # CHANGED (after stage 8's move to
                      #   codeless-bot-core) — re-exports the
                      #   widened renderer and the new types from
                      #   codeless-bot-core so existing call sites
                      #   don't churn.
├── outbound.rs       # CHANGED (after stage 8's move) — slack-
                      #   side concatenation of prelude / block /
                      #   postlude inside ChatPoster::post; the
                      #   wiremock tests' string assertions stay
                      #   byte-identical because the concatenation
                      #   reproduces slack stage 7's output verbatim.
└── web_api.rs        # CHANGED — ChatPoster::post takes
                      #   RenderedNotification, concatenates plain.
```

Tests live with the code (R5):

- `codeless-bot-core/src/reply/notify.rs` keeps slack stage 7's
  full renderer-string-shape test surface verbatim, adapted to
  assert on `RenderedNotification` fields rather than the
  flattened `String`. The 15 unit tests slack stage 7 added (label
  table row by row, `is_empty` short-circuit, missing-paths-only,
  pre-check-pass collapses block, JobStopped path, empty-missing-
  list edge case) stay; assertions like
  `assert!(body.contains("Type:   diff-verify pre-check auto-fail"))`
  become
  `assert_eq!(rendered.block.as_deref().unwrap_or(""), "...")`
  with the explicit expected block text.
- `codeless-bot-core/src/outbound.rs` keeps the 3 `ReviewCache`
  unit tests plus the 3 integration tests slack stage 7 added,
  with the wiremock POST-body capture updated to assert on the
  concatenated `prelude + block + postlude` rather than the
  flattened slack-shape `String`. The "no post on review-only"
  invariant test stays unchanged.
- `crates/codeless-bot/src/post.rs` adds two tests asserting the
  Markdown V2 wrap: one with a Surface 2 block (round-trips
  through `teloxide::utils::markdown::escape` for the 18-char
  pass and asserts the triple-backtick fence frames the
  structured lines unchanged), one with a `None` block (asserts
  no fence emitted and the full text passes the 18-char escape).
  The wiremock binding from stage 4 captures the Telegram-side
  `sendMessage` body and pulls `text` + `parse_mode` for the
  assertions.
- `crates/codeless-bot/src/lib.rs` reuses stage 8's `Publisher`
  spawn test; it gains a single negative assertion that the
  wrapped Telegram body does NOT contain an unescaped `[` from
  the `[!]` glyph (a regression guard for the renderer / poster
  boundary).

The verify trio (`cargo test --workspace`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo fmt --check`)
must be green before commit.

### What would have to happen for stage 9 to unblock

Same sequence as stage 8, with stage 8 itself at the tail:

1. slack-integration finishes its remaining stages (4 through 10)
   on its own branch and merges to `master`. Ships
   `ResumeJobArgs.bypass`, `ResumeJobArgs.next_stage_comment`,
   `JobResumed.actor`, `JobResumed.comment`, and the
   `crates/codeless-slack/` directory (now 11 files post-slack-
   stage-7: the original 9 plus `outbound.rs` (1226 lines) and
   `notify.rs` (585 lines)).
2. This worktree rebases (or a fresh worktree opens) on the
   updated `master`. The four-grep gate then passes.
3. The `codeless-bot-core` extraction runs and moves
   `outbound.rs` + `notify.rs` (including their slack-stage-7
   contents — `ReviewContext`, `ReviewCache`, the enrichment-
   event capture arms, the gate-type label table) out of
   `codeless-slack` alongside the stage-4/5/6 shared modules.
4. Stage 4 (scaffold Telegram transport) creates
   `crates/codeless-bot/` with `Cargo.toml`, `lib.rs`,
   `config.rs`, `long_poll.rs`, `post.rs` (with the 18-char
   MarkdownV2 escape pass already present from the Surface 1
   posting work), plus the `teloxide` (or hand-rolled `reqwest`)
   dep and the `--enable-telegram-bot` CLI flag.
5. Stage 5 (Telegram parser extensions) adds the three concrete
   widenings inside `codeless-bot-core` enumerated in that stage's
   blocked-doc section.
6. Stage 6 (dispatcher + Telegram poster) adds the inbound
   command pipeline and the `BotTransport` impl, with
   `TelegramBot::spawn` exposing `reply_context_map()` for stage
   8 to share.
7. Stage 7's REVIEW gate runs with a real end-to-end Telegram
   client trace and either passes or sends the job back to a
   prior stage.
8. Stage 8 adds the publisher spawn, the per-chat `TokenBucket`
   wrap, and the matching tests.
9. *Then* this stage 9 makes its concrete additions:

   a. Refactor `codeless-bot-core::reply::notify::format_job_failed`
      and `format_job_stopped` to return `RenderedNotification`
      (prelude / block / postlude). Update slack's `web_api.rs`
      to concatenate the three fields. The slack-side wiremock
      tests stay byte-identical because the concatenation
      reproduces slack stage 7's output verbatim.
   b. Add the Markdown V2 wrap to `crates/codeless-bot/src/
      post.rs`: escape prelude / postlude with the 18-char pass,
      wrap block in triple backticks with the code-block 2-char
      escape pass. Skip the fence entirely when block is None.
   c. (Already present from stage 8's move and slack stage 7's
      contents: `ReviewContext`, `ReviewCache`, the enrichment-
      event capture arms in `handle_envelope`, the gate-type
      label table in `notify`, the cache lookup in
      `post_notification`.)
   d. Tests per the section above.

   The verify trio must be green before commit.

### What this stage produces

This commit. The decisions doc gains this Stage-9 section so the
audit trail records *why* stage 9 was halted in the same form as
stages 4, 5, 6 and 8's records. No `crates/` files are touched;
no `Cargo.toml` is edited; no Rust is written. `cargo test
--workspace` was not run for the same reason — there is no Rust
change in this stage to verify.

Stage 9 is marked `[!]` (blocked) per `CLAUDE.md` R4. The job
halts here and waits for the eleven preconditions above (A, B, C,
D, E carried from stages 4–8; I, J, K new at stage 9; plus
stages 4, 5, 6, 7's REVIEW gate, and 8's own completion). The
next session that picks this stage up must:

1. Re-run the four-grep gate before writing any code.
2. Confirm `crates/codeless-bot-core/src/reply/notify.rs` already
   contains `pub struct ReviewContext`, `pub fn review_type_label`,
   and the widened `format_job_failed` / `format_job_stopped`
   signatures (from stage 8's move plus slack stage 7's contents).
3. Confirm `crates/codeless-bot-core/src/outbound.rs` already
   contains `pub const REVIEW_CACHE_CAPACITY`, `struct
   ReviewCache`, and the `Event::ReviewPreCheck` /
   `Event::ReviewVerdict` arms in `handle_envelope`.
4. Confirm `crates/codeless-bot/src/post.rs` exists (from stage 4)
   with the 18-char MarkdownV2 escape pass already in place for
   Surface 1 posting.
5. Only then refactor the renderer return type to
   `RenderedNotification`, propagate through the slack and
   Telegram posters, and add the Telegram-side preformatted-block
   wrap and its tests.

If any precondition still fails, that session also halts.
