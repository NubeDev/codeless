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
