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
