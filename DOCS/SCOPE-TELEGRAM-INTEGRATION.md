# SCOPE-TELEGRAM-INTEGRATION

A design proposal for driving Codeless jobs from Telegram. Not a
spec — a thesis to argue with before any integration code lands.

Read [`SESSION-MUTABLE-SCOPE.md`](./SESSION-MUTABLE-SCOPE.md) and
[`SCOPE-MUTABLE-UI.md`](./SCOPE-MUTABLE-UI.md) first if you have
not already. The first names the runtime contract this integration
sits on; the second names the editor surfaces (REVIEW gate panel,
patch inbox, escape hatch) that Telegram is being asked to mirror
in text.

This doc is the **Telegram counterpart** to
[`SCOPE-SLACK-INTEGRATION.md`](./SCOPE-SLACK-INTEGRATION.md). Same
operator workflow, same thesis, same surface choices — different
transport. The two integrations exist for the same reason
(operator-on-phone, away from a keyboard, needs to keep a job
moving forward) and ship as siblings, not as alternatives. If both
ever ship, they share the same `codeless-bot` adapter crate with
two transport backends; see "What lands where in the codebase"
below.

The reason this doc exists at all: the Slack integration assumed
the operator's workspace was on a paid plan with the necessary
admin headroom to configure bot scopes, channels, and Socket Mode.
For a single-tenant self-hosted operator (R5) running on a personal
or hobbyist workspace, Slack's app-creation flow is a
disproportionate amount of overhead — multiple OAuth scopes, a
reinstall cycle for every scope addition, paid-plan gates on
features the integration needs. Telegram has none of that: one
message to `@BotFather`, one token, done. For the same operator-
on-phone use case, Telegram is the lower-friction choice on every
axis that matters to a self-hosted Codeless deploy.

## Instructions to the reader

Before you read further: **think operator, not feature list.** The
Telegram surface is not "the UI ported to Telegram." Most of the
UI's surfaces (patch inbox, rule maturity badge, cross-job
worklist) are denser than a chat message can carry. Specifically:

- **Reject "everything the UI does, in Telegram."** The Telegram
  surface is narrow on purpose. Status, resume, bypass, comment.
  Patch approval and rulebook editing stay in the UI.
- **Challenge the load-bearing premise.** This doc rests on one
  claim: that the keep-it-running workflow is text-message-shaped
  and the *editor* workflow is not. If patch approvals from
  Telegram turn out to be the real ask, the doc is wrong and the
  right shape is a different one (inline-keyboard cards, separate
  bot per kind of decision, etc.). Attack this first.
- **One bot, one trust boundary.** Codeless is R5 single-tenant;
  the Telegram integration must not introduce a multi-tenant
  trust model. The bot has the same bearer token the UI does. If
  you find yourself reaching for per-Telegram-user permissions,
  you have crossed a line.
- **Name what you are willing to throw away.** Inline keyboards,
  callback queries, multi-step ConversationHandler flows — all
  of it is tempting and most of it is wrong for this scope.
  Plain text commands and one reply-to-message reply are enough
  to start.

If your reaction to any of this is "we could ship a smaller
version of this," you have read it right.

## Out of scope: Telegram-as-an-agent-tool

There are two plausible Telegram integrations and this doc only
covers one of them. Naming the other explicitly so a future
reader does not assume this doc subsumes it:

1. **Telegram as operator control plane** — what this doc is
   about. The human operator drives jobs from Telegram; the
   runtime posts notifications back. The bot acts *as* the
   operator, with the operator's bearer token. Lives next to
   `codeless-server`, subscribes to the event bus.
2. **Telegram as an agent tool** — *not this doc.* The LLM
   inside a running job calls `telegram.send_message` or
   `telegram.read_chat` the same way it calls `browser.fetch`
   or `github_issue`. Would live in
   [`codeless-tools`](../codeless/crates/codeless-tools/src/lib.rs)
   as an MCP tool, gated by tool policy.

They share an SDK and a bot token and nothing else: different
caller (human vs. LLM), different auth model (operator bearer
token vs. tool policy), different trigger (inbound message vs.
agent decision), different failure mode (wrong-job mistake vs.
**prompt injection from Telegram message content**). The tool
variant in particular is a real risk surface — any Telegram
message the agent reads is untrusted input that can attempt to
jailbreak the prompt — and deserves its own thesis. If/when it
ships, it ships as a separate doc.

If you find yourself adding `telegram.read_chat` to this
integration, stop: you are writing the wrong doc.

## The thesis (one paragraph)

A Codeless job is a sequence of stages, each of which is either
"running and healthy" or "failed and waiting for the operator to
decide what to do." The first state needs status; the second state
needs a binary decision (retry, bypass, stop). Both are
text-message-shaped: a status line is a short string, a decision
is one word. The operator on a phone away from their desk wants
to **see the failure**, **understand the reason in one line**, and
**make one decision** without leaving Telegram. Anything beyond
that — inspecting diffs, approving rulebook patches, walking the
rule stratification — belongs in the web UI. The first scope is
the narrowest possible surface that covers the keep-it-running
loop: status, start, stop, resume, resume-and-bypass-failing-
stage, resume-with-comment, and a failure notification with
enough context to act on. Everything else is a follow-up.

## Why Telegram (vs. Slack)

The two integrations cover the same use case. The reasons to pick
Telegram first for a self-hosted single-operator Codeless deploy:

| | Slack | Telegram |
|---|---|---|
| Bot creation | App manifest, OAuth client, multiple scopes, reinstall on scope change | Message `@BotFather`, name the bot, copy token |
| Receive events | Socket Mode (paid-plan dependency for some features) or public HTTPS webhook + signing-secret verification | Long-polling (no public endpoint) or simple HTTPS webhook |
| Permission model | 15+ named scopes; channels:join, groups:write, channels:manage, app_mentions:read, etc. | Bot is a user; can read messages in chats it is added to. No scope system. |
| Channel selection | Bot must be invited to each channel; channel IDs are opaque | Bot can chat 1:1 (DM equivalent) or be added to a group; chat IDs are simple integers |
| Threading model | Top-level message + thread replies; `thread_ts` is the thread anchor | `reply_to_message_id` on each reply; same semantics, simpler shape |
| Cost to test | Slack workspace + admin access | One mobile message to BotFather |
| Cost to operate | Paid plan for some features (workflows, Socket Mode reliability tiers) | Free, no tier system |

For an R5 single-tenant operator running Codeless on their own
hardware, every row of that table is a strict win for Telegram.
The Slack doc still exists because some operators are already in a
Slack-shaped org; for those, the Slack surface is the right fit.
The two integrations are siblings, not alternatives.

## What the operator can do today

- Web UI on a desktop browser.
- `codeless` CLI over SSH.
- Direct RPC against `http://127.0.0.1:7777` if they know the URL.
- Nothing from a phone unless they SSH from it.

## What the operator gets from Surface 1 of this integration

The bot posts failure notifications as top-level messages in the
configured chat (a 1:1 DM with the operator, or a private group
the operator and the bot share). The operator replies *to that
notification* (Telegram's native reply-to-message feature) — no
job ID required; the reply target implies the job. Cold commands
(not a reply to a notification) require an explicit job ID.

```
/status                                       → list of jobs, status, cost
/status <job-id>                              → one-job detail
/start <job-id>                               → transition Draft → Running

# Cold (requires job ID):
/stop <job-id>
/resume <job-id>
/resume <job-id> bypass
/resume <job-id> "<comment>"
/resume <job-id> bypass "<comment>"

# Reply-to (job ID implied by which notification you replied to):
/stop
/resume
/resume bypass
/resume "<comment>"
/resume bypass "<comment>"
```

Telegram commands are prefixed with `/` by convention — this is the
native command syntax the Telegram client renders as a tappable
list. The Slack doc uses `@codeless` because that is the Slack
convention; the meaning and parser are otherwise identical.

Plus an **outbound failure notification** posted by Codeless when
any job transitions to `Failed`, with the failure reason, the
failing stage's title, and the reply commands the operator can
type as a reply to the message:

```
🚨 Job "scope-mutable-ui" — Failed at stage 8/13
   Stage: "REVIEW after per-job action loop"
   Reason: diff-verify pre-check failed; handover claims paths
           not in the diff: DOCS/SCOPE-MUTABLE-UI.md
   Cost: $52.64 / $150.00 cap
   Reply to this message: /resume bypass | /resume "<comment>" | /stop
```

That is the whole first surface.

## The six surfaces (smallest to broadest)

Same shape as the Slack doc — the surfaces are the *workflows*,
not the transport — so this section is a cross-reference rather
than a re-write. Where a surface's behaviour is identical between
Slack and Telegram, the Slack doc is the source of truth and this
doc adds only the Telegram-specific transport notes.

The boundary between surfaces is what each one assumes about the
operator's intent. Surface 1 is "keep the job moving"; the rest
are progressively richer engagements.

| #  | Surface                                  | Operator stance     |
|----|------------------------------------------|---------------------|
| 1  | Keep-it-running commands                 | Present, deciding   |
| 2  | REVIEW gate failure context              | Present, debugging  |
| 3  | Job submission from Telegram             | Present, creating   |
| 4  | Job policy commands (hands-off mode)     | Walking away        |
| 5  | Patch approvals (deliberately not v1)    | Editor — not chat   |
| 6  | Cross-job inbox                          | Triage              |

### Surface 1 — Keep-it-running (first scope)

The minimum-viable Telegram surface. Five commands, one
notification. Identical workflow to Slack Surface 1; only the
transport differs.

**Commands** (each maps directly to an existing RPC):

When replying to a notification message the job ID is implied by
which notification was replied to; the parser maps
`reply_to_message_id → job_id` from the table the bot kept when it
posted the notification. When issuing a cold command (not a reply)
the job ID is required.

| Telegram input                            | RPC                                  | Notes |
|-------------------------------------------|--------------------------------------|-------|
| `/status`                                 | `list_jobs`                          | Bot filters to non-terminal + last 3 terminal per repo. |
| `/status <job-id>`                        | `get_job`                            | Includes status, cost, current stage, stop_reason. |
| `/start <job-id>`                         | `start_job`                          | Only valid when status is `Draft`. |
| `/stop` *(reply-to)*                      | `stop_job`                           | Valid when status is `Running` or `Queued`. |
| `/stop <job-id>` *(cold)*                 | `stop_job`                           | Same, explicit job ID. |
| `/resume` *(reply-to)*                    | `resume_job`                         | Valid when status is `Stopped`, `Failed`, or `Paused`. |
| `/resume <job-id>` *(cold)*               | `resume_job`                         | Same, explicit job ID. |
| `/resume bypass` *(reply-to)*             | `resume_job` with `bypass: true`     | Requires Dependency #1 below. |
| `/resume bypass "<comment>"` *(reply-to)* | Both.                                |  |
| `/resume "<comment>"` *(reply-to)*        | `resume_job` with `comment: <str>`   | Comment threaded into the prompt of the next-run stage. |
| `/resume <job-id> bypass "<comment>"` *(cold)* | Both.                           |  |

**Grammar — identical to Slack Surface 1.** `bypass` is a
positional keyword that, if present, appears immediately after
`<job-id>` and before the optional quoted comment. The comment, if
present, is the *last* token and is always double-quoted. A
comment that happens to contain the word `bypass` is unambiguous
because the parser only treats the literal keyword in the keyword
slot:

```
/resume 01KRP... bypass "this also bypasses linting"   # bypass=true, comment set
/resume 01KRP... "please bypass the linter manually"   # bypass=false, comment set
```

Embedded double-quotes in the comment are escaped with `\"`. The
bot's help text restates this; do not invent a second quoting
convention.

**Outbound notification** fires on `JobFailed` and `JobStopped`
events (subscribed via the existing event bus). Format above. One
message per terminal transition, no flapping.

**Why this surface first:** the keep-it-running loop is the
operator-on-phone use case. Five commands cover the entire
workflow the user described in their ask. Each command is a thin
wrapper around an existing RPC; the integration is mostly
transport.

**Status:**
- `/start` / `/stop` / `/status` / standard `/resume` — unblocked,
  RPCs exist today.
- `/resume bypass` — **blocked** on Dependency #1 below (shared
  with the Slack integration and with SCOPE-MUTABLE-UI Surface E,
  where it is numbered #6a).
- `/resume "<comment>"` — **partially blocked** on Dependency #2
  (the `next_stage_comment` field on `ResumeJobArgs`). Shared
  with the Slack integration.
- Failure notification — unblocked. The Codeless event bus
  already emits `JobFailed`; the Telegram adapter subscribes via
  the existing `subscribe` RPC.

**Anti-patterns to avoid:**

- **Inline keyboards as confirmation.** Telegram supports
  `InlineKeyboardButton` with `callback_data`; resist for v1.
  Tappable buttons are tempting but a two-tap exchange ("are you
  sure?" → "yes I'm sure") is exactly the friction the
  operator-on-phone scenario does not have time for. One
  command, one result. (Buttons may show up in a later surface
  as a quality-of-life addition — `[Resume bypass] [Resume] [Stop]`
  rendered as inline keyboard buttons below the failure
  notification, each sending the equivalent command. They are not
  in v1.)
- **Mirroring the entire JobsDashboard in Telegram.** Telegram is
  a control plane, not a dashboard. Keep `/status` output to
  ~10 lines max; if the operator needs more, they open the web
  UI.
- **Per-chat state.** The bot does not remember "the last job ID
  you typed in this chat" so a follow-up `/resume` can omit the
  ID. That kind of state is what the UI is for. Every cold
  command takes the job ID explicitly; reply-to commands carry
  the implication explicitly via Telegram's native
  `reply_to_message_id`.
- **Message reactions as confirmation.** Telegram message
  reactions exist; same anti-pattern as Slack. If a confirmation
  is truly needed, use an explicit follow-up command, never a
  reaction.

### Surfaces 2–6

Identical *workflow* to the Slack doc's Surfaces 2–6. The transport
swaps (`@codeless submit` → `/submit`, "in-thread reply" →
"reply-to-message"). The dependencies, anti-patterns, and
stopping points carry over.

Rather than restate them, the Slack doc is the source of truth
for the workflow semantics of Surfaces 2–6. Where this doc adds
content it is only the Telegram-specific transport fact.

**Surface 2 — REVIEW gate failure context.** Same structured
context block as Slack Surface 2. Telegram supports Markdown V2
and HTML message formatting; render the block as monospaced
preformatted text (triple-backtick) so the path list and the
quoted prior bullet remain readable on a phone.

**Surface 3 — Job submission from Telegram.** Command becomes
`/submit <repo> <template-name>`. The 5-second cooldown described
in the Slack doc applies identically; an inline-keyboard "Cancel"
button is *out of v1* (use the `/cancel <token>` text command per
the Slack doc).

**Surface 4 — Job policy commands (hands-off mode).** Command
becomes `/policy <job-id> ...`. The muted "auto-bypassed"
notification and the louder "thrashing halt" notification are
posted to the same configured chat. No Telegram-specific change.

**Surface 5 — Patch approvals (deliberately out of scope of v1).**
Same reasoning as Slack Surface 5. If patch approval from Telegram
ever becomes a real ask, it ships as a separate doc with its own
thesis. Don't fold it into this one.

**Surface 6 — Cross-job worklist.** Command becomes `/inbox`.
Output format identical to Slack Surface 6.

## The user journeys

Same three journeys as the Slack doc — the journeys are the
*workflow* and don't change with the transport. The Slack doc is
the source of truth; the only difference is that "reply in this
thread" becomes "reply to this message" and `@codeless status`
becomes `/status`.

## Dependencies — what has to land before each surface

Numbering below is local to this doc. The Slack doc uses its own
numbering for the *same* set of dependencies; this is deliberate so
each doc reads standalone. Cross-references to the Slack doc are
called out by surface number (e.g. "Slack Dep #2").

| Surface | Backend | Telegram-side |
|---------|---------|---------------|
| 1 (keep-it-running) | Dep #1 (`resume_job.bypass`), Dep #2 (`resume_job.next_stage_comment`) | Dep #3 (Telegram adapter crate); bot user setup |
| 2 (REVIEW context) | Dep #5 (`ReviewPreCheck` / `ReviewVerdict` events) | Event subscriber + formatter |
| 3 (submit) | Dep #4 (`submit_job_from_template_name` RPC) + `SubmitJobArgs.auto_bypass_policy` | Command parser |
| 4 (policy / hands-off) | SCOPE-MUTABLE-UI #7 (`auto_bypass_policy` column + thrashing guard) | `/policy <id>` parser; muted notification formatter |
| 5 (patches) | (deliberately out of scope) | (deliberately out of scope) |
| 6 (inbox) | None | `list_jobs` filter |

### Dependency #1 — `resume_job` accepts `bypass`

**Identical** to Slack Dep #1 and to SCOPE-MUTABLE-UI #6a. This
integration is a *third* consumer of the same plumbing; the field
ships once, all three consumers benefit. See Slack Dep #1 and
SCOPE-MUTABLE-UI #6a for the field shape.

### Dependency #2 — `resume_job` accepts `next_stage_comment`

**Identical** to Slack Dep #2. See that doc for the
`ResumeJobArgs` shape, the prompt-assembly rendering, and the
audit-trail story. The `JobResumed.actor` field is populated with
the Telegram user ID (an integer cast to string) in the Telegram
case, the Slack user ID in the Slack case, and the local username
in the CLI case. The field is for audit only and never
participates in authorisation (R5).

### Dependency #3 — Telegram adapter crate (or backend on shared adapter)

The Telegram-side analogue of Slack Dep #3. Two choices:

**Option A: dedicated crate `codeless-telegram`.** Symmetric with
`codeless-slack`; cleanest separation; two CI artefacts; two
feature flags (`--enable-slack`, `--enable-telegram`).

**Option B: shared crate `codeless-bot` with backend modules.** A
single `codeless-bot` crate with `transport/slack.rs` and
`transport/telegram.rs` modules, an internal `BotTransport` trait
each implements, and a single command parser + event subscriber
above the trait. One CI artefact; two feature flags
(`--enable-slack-bot`, `--enable-telegram-bot`); the parser and
the event-bus subscription are written once.

**Recommendation: Option B.** The command grammar, the failure-
notification template, the event-bus subscription, the rate-limit
logic, and the `reply_to_message_id → job_id` mapping (which is
the same idea as `thread_ts → job_id`) are all transport-agnostic.
Sharing them means a bugfix in either transport's parser path
fixes both, and the second transport is mostly a config file plus
a thin transport-specific glue module. Option A is the right call
only if the two transports diverge enough that the shared trait
becomes a leaky abstraction; on present evidence they will not.

Either way, the resulting crate is **host-only per R1**. It
contains no `tokio::process` or `std::process::Command` imports
and is not in `codeless-adapters-host`. The existing CI grep
enforces this.

The Telegram client itself is one of:
- [`teloxide`](https://crates.io/crates/teloxide) — high-level
  Rust framework, async, well-maintained, opinionated.
- [`frankenstein`](https://crates.io/crates/frankenstein) —
  thin auto-generated bindings, fewer opinions, easier to keep
  current with the Bot API.
- [`tgbotapi`](https://crates.io/crates/tgbotapi) — middle
  ground.

Pick at implementation time; the choice is internal to the
adapter and not load-bearing for the scope. `teloxide` is the
default recommendation because the long-polling event loop it
ships with covers the failure modes Codeless cares about (network
drop, restart, backoff) without the bot adapter having to
reimplement them.

**R5:** the bot has a single bearer token, read at startup from
the secrets store at `~/.config/codeless/secrets.toml` under the
key `telegram_bot_token`. The Telegram bot token *is itself* the
authoriser; there is no separate "app token + bot token" model
like Slack has. The Telegram chat's own auth controls *who can
talk to the bot* (the bot can be locked to a single user ID or
to messages from a specific group). Once they can, they are the
operator.

### Dependency #4 — `submit_job_from_template_name` RPC

**Identical** to Slack Dep #4. See that doc.

## Open questions worth fighting about

Same questions as the Slack doc with Telegram-flavoured answers
where the answer is transport-specific.

1. **One chat, reply-to-scoped commands — resolved.** A single
   chat per operator: either a 1:1 DM with the bot, or a private
   group the bot has been added to and that contains only the
   operator (+ optionally a small ops team). Chat ID is
   configurable via a `telegram_chat_id: i64` field on the Repo
   row. The bot posts each failure notification as a top-level
   message; the operator replies *to that message* (Telegram's
   native reply-to feature) with bare commands (`/resume bypass`,
   `/stop`) — no job ID required because the bot maps
   `reply_to_message_id → job_id` at notification time. Cold
   commands (`/status`, `/stop <id>`) work in any chat the bot
   is in or in DM, and still require an explicit job ID.

2. **DM vs group commands?** Both work. Notifications go to the
   configured chat (DM or group); cold commands can come from
   DM or from any group the bot is in. The bot replies in the
   same chat the command came from.

3. **What about Telegram user → operator mapping?** R5 says one
   trust boundary; the bot is the operator. But the audit trail
   should still record *which Telegram user* typed each command.
   The `JobResumed` event payload's `actor: Option<String>` field
   (introduced for Slack) is reused: in the Telegram case it
   carries the Telegram user ID (`from.id`) as a string. Not
   used for authorisation — only for the audit log.

4. **Who is allowed to talk to the bot?** Telegram bots are
   reachable from any Telegram user who knows the bot's
   username. Mitigation: the adapter has an `allowed_user_ids:
   Vec<i64>` (or `allowed_chat_ids: Vec<i64>`) config and
   silently drops messages from anyone else. This is **not a
   trust boundary** — R5 is preserved by the bearer token, not
   by the allowlist — but it cuts down on noise and accidental
   command-from-the-wrong-account incidents. If the allowlist is
   empty, the adapter refuses to start (fail-closed).

5. **Rate limits / spam protection?** Same as Slack: one
   command-from-Telegram per second per job. Telegram's own
   server-side rate limits also apply (30 messages/sec/chat for
   bots); the adapter respects them with a token-bucket on
   outbound posts.

6. **What about job submission with a typo?** Same as Slack: 5-
   second cooldown with a `/cancel <token>` escape. The bot
   replies "submitting…" and waits 5 seconds before actually
   calling the RPC.

7. **What about the `comment` containing quote characters?** Same
   as Slack — double-quotes with `\"` escape.

8. **Long-polling vs webhook?** Default to long-polling. Webhooks
   require a public HTTPS endpoint with a valid TLS cert; long-
   polling works from any machine that can talk *outbound* to
   `api.telegram.org`. For a self-hosted R5 operator that is
   strictly less plumbing. If a future deployment specifically
   wants webhooks (e.g. the operator already runs codeless-
   server on a public hostname with TLS), expose it as a config
   option but ship long-polling as the default.

## What this ramp deliberately does not include

Same exclusion list as the Slack doc:

- **Telegram message editing.** No editing of prior
  notifications when state changes.
- **Telegram scheduled messages / message-effect.** No "resume
  this job at 9am tomorrow." Scheduling is what cron is for.
- **Multi-job batch operations.** No `/resume all failed`.
- **Approval flows for `bypass`.** Single operator's call.
- **Inline keyboard buttons as decisions in v1.** May land later
  as a UX improvement, never as the *primary* command path.
- **Conversation threading with the agent.** The
  `[Talk to agent]` flow from SCOPE-MUTABLE-UI.md's Surface E
  is genuinely interactive and lives in the web UI.

## Risk and the failure modes

Largely the same as the Slack doc; only the transport-specific
ones differ.

**Risk 1 — Wrong job ID.** Same as Slack Risk 1: `/resume` echoes
the job's template name in the reply.

**Risk 2 — Notification noise.** Same as Slack Risk 2: 5-minute
debounce on event-driven outbound notifications, no debounce on
synchronous command replies.

**Risk 3 — Bypass abuse.** Same as Slack Risk 3.

**Risk 4 — Bot token leak.** A Telegram bot token *is* a bearer
credential; whoever holds it can post as the bot and read every
message in every chat the bot is in. Mitigation: store in
`~/.config/codeless/secrets.toml` under
`telegram_bot_token`; the `CODELESS_TELEGRAM_BOT_TOKEN` env var
is read by `init-session.sh` once at setup and written into the
secrets store, never read by the long-running server. Rotate via
`@BotFather`'s `/revoke` if compromise is suspected (the bot
keeps its identity; only the token changes).

**Risk 5 — Telegram platform outage / blocked.** Codeless still
works without Telegram; the bot is additive. Telegram is blocked
in some jurisdictions; for operators in those, the Slack adapter
or the web UI is the right fallback. The integration is not in
the runtime's hot path; it subscribes to the event bus the same
way any other consumer does.

**Risk 6 — Message arrives from a non-allowlisted user.** Covered
by Open Question 4. The adapter drops the message silently and
logs the rejected user ID for the operator to review.

## What lands where in the codebase

Mirrors the Slack doc, with the option B (shared adapter)
recommendation from Dep #3:

- **New crate** `codeless-bot` under `codeless/crates/`, with two
  transport-backend modules (`transport/slack.rs`,
  `transport/telegram.rs`) and a single `BotTransport` trait.
  Host-only per R1. Cargo features `--enable-slack-bot` and
  `--enable-telegram-bot` on the `codeless serve` CLI so
  deployments that need only one (or neither) pay zero cost for
  the other.
- If the Slack adapter has *already* shipped as a standalone
  `codeless-slack` crate by the time the Telegram work starts,
  the right move is to lift the parser / event-subscriber /
  rate-limiter into a shared `codeless-bot-core` crate that both
  the existing `codeless-slack` and the new `codeless-telegram`
  depend on, rather than restructure `codeless-slack` mid-flight.
- `codeless-rpc/src/methods.rs` — *no new fields beyond what
  Slack Dep #1 and #2 already added.* The Telegram integration
  reuses them as-is.
- `codeless-runtime/src/rpc/jobs.rs` — *no changes beyond Slack
  Dep #1 / #2.*
- `codeless-runtime/src/template_runner.rs` — *no changes beyond
  Slack Dep #2.*
- `codeless-types/src/event.rs` — *no changes beyond Slack
  Dep #2.* The `JobResumed.actor` field already accepts an
  arbitrary string; the Telegram adapter populates it with the
  Telegram user ID.
- New `codeless-bot/src/transport/telegram.rs` (or
  `codeless-telegram/src/lib.rs` under Option A) — Telegram
  adapter implementation.
- `setup/init-session.sh` — `--enable-telegram` flag plumbing,
  bot token env var, chat ID config, allowlist config.
- `codeless/.codeless/jobs/telegram-integration/` — the per-job
  scope dir for this work, when it gets turned into a real
  Codeless job.

**R1:** the Telegram adapter spawns no subprocesses. Confirmed
via the existing `no-process-spawn-outside-adapters-host`
predicate; the new crate (or module) is not in
`codeless-adapters-host`, so any `process::Command` in it would
fail CI.

**R5:** unchanged. One bot, one bearer token, one operator trust
boundary. The Telegram user ID is captured for audit only.

## What ships, in order

A ramp, not a tier list. Mirrors the Slack ramp; if the Slack
ramp has already landed, only Steps 2–6 need re-doing for
Telegram (the RPC scaffolding from Step 1 is shared).

### Step 1 — Dependencies (RPC arg additions)

**Shared with Slack Step 1.** If the Slack integration shipped
first, this is already done. If the Telegram integration ships
first, the same `bypass` + `next_stage_comment` fields land on
`ResumeJobArgs`, and the Slack integration inherits them.

### Step 2 — Telegram adapter scaffold + Surface 1 commands

New crate (or module) `codeless-bot` / `transport/telegram.rs`.
Telegram client (`teloxide` recommended). Bot user setup via
`@BotFather`, env var for token, command grammar parser. Five
commands: `/status`, `/status <id>`, `/start`, `/stop`,
`/resume <id> [bypass] [<comment>]`. No outbound notifications
yet.

Stopping here: the operator can drive a job entirely from
Telegram provided they already know the job ID. No surprises.

### Step 3 — Outbound failure notifications

Subscribe to the event bus, post on `JobFailed` and
`JobStopped`. Format per Surface 1's mockup. Per-job 5-minute
debounce.

Stopping here: the keep-it-running loop is end-to-end
operational from a phone. **This is the first scope's done
line.**

### Step 4 — Surface 2 REVIEW gate context

Same as Slack Step 4.

### Step 5 — Surface 4 policy commands (`hands-off` mode)

Same as Slack Step 5. Blocked on SCOPE-MUTABLE-UI Dependency #7.

### Step 6 — Surface 3 submit + Surface 6 inbox

Same as Slack Step 6.

### Stopping points

- Stop at Step 2: commands work; no notifications.
- Stop at Step 3: **the first scope is complete.** Operator can
  keep any job moving from a phone.
- Stop at Step 4: failures include rich context.
- Stop at Step 5: **hands-off operator mode is live.**
- Reach Step 6: keyboard-free Codeless surface fully
  operational.

## Pointers

- The Slack counterpart this doc mirrors:
  [`SCOPE-SLACK-INTEGRATION.md`](./SCOPE-SLACK-INTEGRATION.md)
- The runtime this integration sits on:
  [`SESSION-MUTABLE-SCOPE.md`](./SESSION-MUTABLE-SCOPE.md)
- The web-UI surfaces this integration shares dependencies
  with: [`SCOPE-MUTABLE-UI.md`](./SCOPE-MUTABLE-UI.md)
- Resume / state-machine reference:
  [`crates/codeless-runtime/src/rpc/jobs.rs`](../codeless/crates/codeless-runtime/src/rpc/jobs.rs)
- Event bus subscription pattern (for the outbound
  notifications): existing `subscribe` RPC + `EventEnvelope`
  serde shape
- Telegram Bot API reference:
  https://core.telegram.org/bots/api
- `@BotFather` (the canonical Telegram bot-creation flow):
  https://t.me/BotFather

## Appendix A — Operator setup (BotFather → first message)

This is the one-time setup an operator does *before* Step 2 of the
ramp can do anything. It is intentionally brief: if it takes more
than five minutes, the doc's "Telegram is the low-friction choice"
premise is wrong.

### A.1 — Create the bot

1. Open Telegram and start a chat with [`@BotFather`](https://t.me/BotFather).
2. Send `/newbot`.
3. Pick a display name (e.g. `Codeless`).
4. Pick a username — must end in `bot` (e.g. `codeless_nube_bot`).
5. BotFather replies with an HTTP API token of the form
   `123456789:ABCdefGHIjklMNOpqrSTUvwxYZ`. **This is the bearer
   credential.** Treat it as a secret per Risk 4.

### A.2 — Lock the bot down

By default, a Telegram bot is reachable from anyone who knows its
username. Two settings — both in BotFather — narrow that surface
before the bot ever sees a Codeless message:

1. `/setjoingroups` → `Disable` if the bot will only be used in
   1:1 DM. If the operator wants to use a private group instead,
   leave it enabled and skip to A.3.
2. `/setprivacy` → `Disable`. This sounds backwards but is correct:
   privacy mode *on* means the bot only sees messages that start
   with `/` or that @-mention it. Codeless commands all start with
   `/` so privacy-on would also work, but disabling it lets the
   reply-to-message flow work without the operator having to
   include the slash command at the start every time. Pick on or
   off based on preference; the parser handles both.

### A.3 — Find the operator's chat ID

The adapter needs to know which chat to post failure
notifications into. Steps:

1. Send any message (e.g. `hello`) to the bot from the Telegram
   account that will be the operator.
2. Open `https://api.telegram.org/bot<TOKEN>/getUpdates` in a
   browser, replacing `<TOKEN>` with the BotFather token.
3. Find the most recent `message` object in the JSON. Note:
   - `message.chat.id` — the chat ID (for 1:1, equals the
     operator's user ID; for a group, a negative integer).
   - `message.from.id` — the operator's Telegram user ID
     (needed for the allowlist per Open Question 4).

### A.4 — Store the credentials

Three secrets / config values land in
`~/.config/codeless/secrets.toml`:

```toml
telegram_bot_token  = "123456789:ABCdefGHIjklMNOpqrSTUvwxYZ"
telegram_chat_id    = 987654321
telegram_allowed_user_ids = [987654321]
```

The `init-session.sh` script reads `CODELESS_TELEGRAM_BOT_TOKEN`,
`CODELESS_TELEGRAM_CHAT_ID`, and `CODELESS_TELEGRAM_ALLOWED_USER_IDS`
from the environment once and writes them into the secrets store.
The long-running `codeless serve` process reads only the secrets
store, never the env vars (Risk 4).

### A.5 — Verify with `auth.test`-equivalent

Telegram's analogue of Slack's `auth.test` is `getMe`. From a
shell with the token exported:

```sh
curl -s "https://api.telegram.org/bot${CODELESS_TELEGRAM_BOT_TOKEN}/getMe" | jq .
```

A successful response includes `ok: true`, the bot's `id`,
`username`, and `first_name`. If `ok` is `false`, the token is
wrong; re-run A.1 or `/revoke` via BotFather.

To verify outbound posting works:

```sh
curl -s -X POST "https://api.telegram.org/bot${CODELESS_TELEGRAM_BOT_TOKEN}/sendMessage" \
  -H 'content-type: application/json' \
  -d "{\"chat_id\": ${CODELESS_TELEGRAM_CHAT_ID}, \"text\": \"codeless bot online\"}" \
  | jq .
```

The operator should see the message arrive in Telegram. If they
do, the bot is ready for Step 2 of the ramp.

### A.6 — Rotating the token

If the operator suspects the token has leaked: chat with
`@BotFather`, send `/revoke`, choose the bot, accept the new
token. The bot keeps its `id`, `username`, and chat history; only
the bearer credential changes. Update `secrets.toml` and restart
`codeless serve`.

### A.7 — Anti-setup notes

- **Do not** use BotFather's `/setcommands` to register the
  slash-command list with Telegram in v1. That feature populates
  the in-app `/` autocomplete UI; it is nice but it is *another
  place the command grammar is defined*, and a drift between
  Telegram's autocomplete and the parser is a real bug class. Add
  it later, when the grammar has stabilised.
- **Do not** turn on inline mode (`/setinline`). Inline mode is a
  separate UX pattern (the bot is invoked from the compose box of
  any chat) that does not match the operator-control-plane
  thesis.
- **Do not** add the bot to a public Telegram group. The
  `allowed_user_ids` allowlist is a defence-in-depth measure, not
  a trust boundary; the right answer is to put the bot in a
  chat that is private to the operator.
