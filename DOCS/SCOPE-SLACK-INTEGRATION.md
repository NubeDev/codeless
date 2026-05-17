# SCOPE-SLACK-INTEGRATION

A design proposal for driving Codeless jobs from Slack. Not a spec —
a thesis to argue with before any integration code lands.

Read [`SESSION-MUTABLE-SCOPE.md`](./SESSION-MUTABLE-SCOPE.md) and
[`SCOPE-MUTABLE-UI.md`](./SCOPE-MUTABLE-UI.md) first if you have
not already. The first names the runtime contract this integration
sits on; the second names the editor surfaces (REVIEW gate panel,
patch inbox, escape hatch) that Slack is being asked to mirror in
text.

This doc tackles a specific operator workflow: **the operator is
on their phone, away from a keyboard, and needs to keep a Codeless
job moving forward.** Today that requires a desktop, a web UI, or
the `codeless` CLI on an SSH session. The thesis here is that
~80% of the keep-the-job-running workflow is text-message-shaped
already (status, decisions, "yes proceed", "skip this gate") and a
Slack surface that mirrors those primitives unlocks the operator's
phone as a real control plane.

## Instructions to the reader

Before you read further: **think operator, not feature list.** The
Slack surface is not "the UI ported to Slack." Most of the UI's
surfaces (patch inbox, rule maturity badge, cross-job worklist) are
denser than a chat thread can carry. Specifically:

- **Reject "everything the UI does, in Slack."** The Slack surface
  is narrow on purpose. Status, resume, bypass, comment. Patch
  approval and rulebook editing stay in the UI.
- **Challenge the load-bearing premise.** This doc rests on one
  claim: that the keep-it-running workflow is text-message-shaped
  and the *editor* workflow is not. If patch approvals from Slack
  turn out to be the real ask, the doc is wrong and the right
  shape is a different one (interactive Block Kit cards, modal
  flows, etc.). Attack this first.
- **One bot user, one trust boundary.** Codeless is R5
  single-tenant; the Slack integration must not introduce a
  multi-tenant trust model. The bot has the same bearer token the
  UI does. If you find yourself reaching for per-Slack-user
  permissions, you have crossed a line.
- **Name what you are willing to throw away.** Interactive Block
  Kit messages, ephemeral confirmations, multi-step modal flows
  — all of it is tempting and most of it is wrong for this scope.
  Plain message commands and one reaction-as-confirmation are
  enough to start.

If your reaction to any of this is "we could ship a smaller
version of this," you have read it right.

## Out of scope: Slack-as-an-agent-tool

There are two plausible Slack integrations and this doc only
covers one of them. Naming the other explicitly so a future
reader does not assume this doc subsumes it:

1. **Slack as operator control plane** — what this doc is
   about. The human operator drives jobs from Slack; the
   runtime posts notifications back. The bot acts *as* the
   operator, with the operator's bearer token. Lives next to
   `codeless-server`, subscribes to the event bus.
2. **Slack as an agent tool** — *not this doc.* The LLM inside
   a running job calls `slack.post_message` or
   `slack.read_thread` the same way it calls `browser.fetch`
   or `github_issue`. Would live in
   [`codeless-tools`](../codeless/crates/codeless-tools/src/lib.rs)
   as an MCP tool, gated by tool policy.

They share an SDK and a bot token and nothing else: different
caller (human vs. LLM), different auth model (operator bearer
token vs. tool policy), different trigger (inbound message vs.
agent decision), different failure mode (wrong-job mistake vs.
**prompt injection from Slack message content**). The tool
variant in particular is a real risk surface — any Slack
message the agent reads is untrusted input that can attempt to
jailbreak the prompt — and deserves its own thesis. If/when
it ships, it ships as a separate doc.

If you find yourself adding `slack.read_thread` to this
integration, stop: you are writing the wrong doc.

## The thesis (one paragraph)

A Codeless job is a sequence of stages, each of which is either
"running and healthy" or "failed and waiting for the operator to
decide what to do." The first state needs status; the second state
needs a binary decision (retry, bypass, stop). Both are
text-message-shaped: a status line is a short string, a decision
is one word. The operator on a phone away from their desk wants
to **see the failure**, **understand the reason in one line**, and
**make one decision** without leaving Slack. Anything beyond that —
inspecting diffs, approving rulebook patches, walking the rule
stratification — belongs in the web UI. The first scope is the
narrowest possible surface that covers the keep-it-running loop:
status, start, stop, resume, resume-and-bypass-failing-stage,
resume-with-comment, and a failure notification with enough
context to act on. Everything else is a follow-up.

## What the operator can do today

- Web UI on a desktop browser.
- `codeless` CLI over SSH.
- Direct RPC against `http://127.0.0.1:7777` if they know the URL.
- Nothing from a phone unless they SSH from it.

## What the operator gets from Surface 1 of this integration

From any Slack channel the bot is in, or via DM to the bot:

```
@codeless status                              → list of jobs, status, cost
@codeless status <job-id>                     → one-job detail
@codeless start <job-id>                      → transition Draft → Running
@codeless stop <job-id>                       → transition Running → Stopped
@codeless resume <job-id>                     → standard resume
@codeless resume <job-id> bypass              → resume past the failed stage
@codeless resume <job-id> "<comment>"         → resume with operator comment
@codeless resume <job-id> bypass "<comment>"  → both
```

Plus an **outbound failure notification** posted by Slack when any
job transitions to `Failed`, with the failure reason, the failing
stage's title, and the message format the operator can copy back
to act on:

```
🚨 Job 01KRPVJX...M4S59Z5D — Failed at stage 8/13
   Stage: "REVIEW after per-job action loop"
   Reason: diff-verify pre-check failed; handover claims paths
           not in the diff: DOCS/SCOPE-MUTABLE-UI.md
   Cost: $52.64 / $150.00 cap
   Reply: `resume bypass` or `resume "<comment>"` or `stop`
```

That is the whole first surface.

## The six surfaces (smallest to broadest)

The boundary between surfaces is what each one assumes about the
operator's intent. Surface 1 is "keep the job moving"; the rest
are progressively richer engagements.

| #  | Surface                                  | Operator stance     |
|----|------------------------------------------|---------------------|
| 1  | Keep-it-running commands                 | Present, deciding   |
| 2  | REVIEW gate failure context              | Present, debugging  |
| 3  | Job submission from Slack                | Present, creating   |
| 4  | Job policy commands (hands-off mode)     | Walking away        |
| 5  | Patch approvals (deliberately not v1)    | Editor — not Slack  |
| 6  | Cross-job inbox                          | Triage              |

### Surface 1 — Keep-it-running (first scope)

The minimum-viable Slack surface. Five commands, one notification.

**Commands** (each maps directly to an existing RPC):

| Slack input                          | RPC                                  | Notes |
|--------------------------------------|--------------------------------------|-------|
| `status`                             | `list_jobs`                          | Bot filters to non-terminal + last 3 terminal per repo. |
| `status <job-id>`                    | `get_job`                            | Includes status, cost, current stage, stop_reason. |
| `start <job-id>`                     | `start_job`                          | Only valid when status is `Draft`. |
| `stop <job-id>`                      | `stop_job`                           | Valid when status is `Running` or `Queued`. |
| `resume <job-id>`                    | `resume_job`                         | Valid when status is `Stopped` or `Failed` or `Paused`. |
| `resume <job-id> bypass`             | `resume_job` with `bypass: true`     | Requires Dependency #1 below. |
| `resume <job-id> "<comment>"`        | `resume_job` with `comment: <str>`   | Comment threaded into the prompt of the next-run stage. |
| `resume <job-id> bypass "<comment>"` | Both. |  |

**Grammar — `bypass` is a positional keyword, not part of the
comment.** If present, it appears immediately after `<job-id>`
and before the optional quoted comment. The comment, if present,
is the *last* token and is always double-quoted. A comment that
happens to contain the word `bypass` is unambiguous because the
parser only treats the literal keyword in the keyword slot:

```
resume 01KRP... bypass "this also bypasses linting"   # bypass=true, comment set
resume 01KRP... "please bypass the linter manually"   # bypass=false, comment set
```

Embedded double-quotes in the comment are escaped with `\"`. The
bot's help text restates this; do not invent a second quoting
convention.

**Outbound notification** fires on `JobFailed` and `JobStopped`
events (subscribed via the existing event bus). Format above. One
message per terminal transition, no flapping.

**Why this surface first:** the keep-it-running loop is the
operator-on-phone use case. Five commands cover the entire workflow
the user described in their ask. Each command is a thin wrapper
around an existing RPC; the integration is mostly transport.

**Status:**
- `start` / `stop` / `status` / standard `resume` — unblocked,
  RPCs exist today.
- `resume bypass` — **blocked** on Dependency #1 below (also
  consumed by SCOPE-MUTABLE-UI Surface E, where it is
  numbered #6a). The `resume_job` RPC needs a
  `bypass: Option<BypassRequest>` argument that marks the
  most recently failed stage as bypassed before requeuing. The scope-mutable-ui job is
  currently in flight delivering this; the Slack integration
  can ship the surface today and conditionally enable the
  bypass command once #6a lands.
- `resume "<comment>"` — **partially blocked**. There is no
  existing RPC arg for "prepend a comment to the next stage's
  prompt." The smallest fix: add `next_stage_comment:
  Option<String>` to `ResumeJobArgs`; the runtime threads it
  into the next stage's prompt assembly. New work but small
  (~50 lines in `template_runner.rs` + arg plumbing).
- Failure notification — unblocked. The Codeless event bus
  already emits `JobFailed`; the Slack adapter subscribes via
  the existing `subscribe` RPC.

**Anti-patterns to avoid:**

- **Multi-step interactive flows.** Slack supports Block Kit
  modal flows; resist. A two-message exchange ("are you sure?"
  → "yes I'm sure") is exactly the friction the operator-on-
  phone scenario does not have time for. One command, one
  result.
- **Mirroring the entire JobsDashboard in Slack.** Slack is a
  control plane, not a dashboard. Keep `status` output to
  ~10 lines max; if the operator needs more, they open the
  web UI.
- **Per-channel state.** The bot does not remember "the last
  job ID you typed in this channel" so a follow-up `resume`
  can omit the ID. That kind of state is what the UI is for.
  Every command takes the job ID explicitly.
- **Slack reactions as confirmation.** Tempting but fragile —
  reactions race, the bot has no way to know "this 👍 is the
  approval, that 👍 is just enthusiasm." If a confirmation is
  truly needed, use an explicit follow-up command
  (`confirm-bypass <job-id>`); never react.

### Surface 2 — REVIEW gate failure context

When a REVIEW gate auto-fails (diff-verify pre-check rejected the
handover; PASS/FAIL sentinel unparseable; model emitted FAIL), the
failure notification grows a structured context block:

```
🚨 Job 01KRPVJX...M4S59Z5D — Stage 8/13 FAILED
   Stage: "REVIEW after per-job action loop"
   Type:  diff-verify pre-check auto-fail
   Reason: handover claims paths not in the diff:
           • DOCS/SCOPE-MUTABLE-UI.md
           • ui/codeless-ui/src/modules/jobs/patches
   Prior handover bullet that triggered it:
   > "Shipped Surface B per `DOCS/SCOPE-MUTABLE-UI.md`
   >  Step 3..."

   Reply: resume bypass | resume "<comment>" | stop
```

The structured block is the same data Surface A from
SCOPE-MUTABLE-UI.md surfaces in the web UI; this is the Slack
equivalent for the keep-it-running loop.

**Status:** depends on Dependency #1 from
[`SCOPE-MUTABLE-UI.md`](./SCOPE-MUTABLE-UI.md) (`ReviewPreCheck`
and `ReviewVerdict` events on the bus). The Slack adapter
subscribes to those events and formats them. No new backend work
beyond what Surface A already requires.

**Anti-pattern to avoid:**

- **Posting every event.** Slack channels become unreadable
  fast. Only `JobFailed` and `JobStopped` get an outbound post
  by default; `StageStarted` / `StageCompleted` / individual
  AI messages do NOT. A future "verbose mode" can opt a
  specific channel into a richer firehose but it is not part
  of the first scope.

### Surface 3 — Job submission from Slack

```
@codeless submit <repo> <template-name>
```

Looks up a saved job template by name and submits a new job. The
templates live in `<repo>/.codeless/jobs/<name>/template.yaml`,
which is already how the web UI's "rerun" surface works.

**Status:** requires a small `submit_job_from_template_name` RPC
that reads the template file from the repo and forwards to the
existing `submit_job`. The web UI's rerun path already does this;
extracting it as its own RPC is ~20 lines.

**Why this is Surface 3 and not Surface 1:** job *submission* is
where mistakes get expensive. A typo in a template name from a
phone, fat-fingered into a real production repo, costs real
money. The operator-on-phone case is "I have a running job and
something happened to it"; "I want to start a new job" is a
desktop use case where the slow-deliberate web UI is the right
tool.

### Surface 4 — Job policy commands (`hands-off` mode)

The Slack surface for **Surface F** in
[`SCOPE-MUTABLE-UI.md`](./SCOPE-MUTABLE-UI.md). One command sets
or changes a job's auto-bypass policy; the runtime applies the
policy whenever a stage fails after that point.

```
@codeless policy <job-id>                     → show current policy
@codeless policy <job-id> <preset>            → set to a preset
@codeless policy <job-id> custom "<text>"     → set to a custom comment
@codeless policy <job-id> none                → disable auto-bypass
```

Where `<preset>` is one of `quick`, `long-term`, `cheap`,
`best-judgement`, `just-code` (lowercased, hyphenated). The
canned comments live in the runtime (
`codeless-runtime::auto_bypass_policy`); Slack just names them.

Two paths to set a policy at submission time:

- The submit form on the web UI (preset picker).
- `@codeless submit <repo> <template> policy:<preset>` (covered
  in Surface 5 below).

Once a policy is in effect, **failure notifications from
Surface 1 change shape**. A `Failed` stage that auto-bypassed
under a policy posts a *muted* notification:

```
↪ Job 01KRPVJX...M4S59Z5D — Stage 8/13 AUTO-BYPASSED
   Stage: "REVIEW after per-job action loop"
   Policy: Quick
   Auto-comment threaded into stage 9: "You are auto-bypassed
     past the prior failed stage..."
   Reply: `policy <id> none` to take manual control
```

A `Stop_reason: AutoBypassThrashing` halt posts a louder
notification:

```
🚨 Job 01KRPVJX...M4S59Z5D — HALTED (auto-bypass thrashing)
   Two stages auto-bypassed in a row with no successful
   stage between. The policy is not rescuing this failure
   class; manual intervention needed.

   Recent failures:
     Stage 8: "REVIEW after per-job action loop"
     Stage 9: "Step 4 Dependency #4"
   Reply: `resume bypass "<custom>"` or `policy <id> <new>`
         or `stop`
```

**Anti-patterns to avoid:**

- **Slack `bypass` as a shortcut for `policy just-code`.** A
  one-shot `resume <id> bypass` is exactly that — one bypass,
  not a permanent policy change. The bot rejects `resume <id>
  bypass-forever` or similar shortcut grammar; if the
  operator wants a policy, they type `policy`.
- **Auto-bypass notifications flooding the channel.** Each
  auto-bypass posts ONE message; multiple auto-bypasses on
  the same job within 5 minutes coalesce into a single
  "auto-bypassed N stages" thread reply rather than N top-
  level posts.
- **Letting `none` accidentally turn off the policy
  mid-failure.** If the job is currently `Failed`, `policy
  <id> none` sets the policy for future failures but does
  NOT auto-resume the current one. Resume is a separate
  command.

**Status:** depends on Surface F in
[`SCOPE-MUTABLE-UI.md`](./SCOPE-MUTABLE-UI.md) (Dependency
#7 — the `auto_bypass_policy` column, the auto-bypass branch
in the stage-failed handler, the thrashing guard).
Slack-side work is small (~20 lines for the parser, ~30
lines for the muted-notification formatter).

### Surface 5 — Patch approvals (deliberately out of scope of v1)

The patch inbox (Surface B in SCOPE-MUTABLE-UI.md) is genuinely
denser than a Slack thread carries — kind, target, rationale,
evidence stage diff link, predicate-shipped flag, prior history.
Approving in a hurry is exactly the failure mode
SESSION-MUTABLE-SCOPE.md's risk section warns against.

If patch approval from Slack ever becomes a real ask, it ships
as a separate doc with its own thesis. Don't fold it into this
one.

### Surface 6 — Cross-job worklist / "what needs my attention"

```
@codeless inbox
```

Returns a list of jobs that are `Failed`, `Stopped`, or have
unresolved REVIEW gates — i.e., everything the operator might
want to act on. The operator's "where do I start" view.

**Status:** unblocked; uses existing `list_jobs` plus a server-
side filter. Adds maybe 30 lines to the bot.

**Why this is Surface 5:** valuable but not essential to the
keep-it-running loop. Ship it when the bot has been in real use
for two weeks and the operator notices they keep typing `status`
to figure out what to look at.

## The user journeys

Three journeys this integration has to support, all from a phone
in Slack.

### Journey 1 — "Job failed while I was at lunch"

The operator gets a Slack notification:

```
🚨 Job 01KRPVJX...M4S59Z5D — Failed at stage 8/13
   Stage: "REVIEW after per-job action loop"
   Reason: diff-verify pre-check failed; handover claims paths
           not in the diff: DOCS/SCOPE-MUTABLE-UI.md
   Cost: $52.64 / $150.00 cap
   Reply: resume bypass | resume "<comment>" | stop
```

They look at the reason. "Yeah, that's the same false positive
class I've seen before; the gate is wrong here." They reply:

```
@codeless resume 01KRPVJX...M4S59Z5D bypass
```

Bot confirms:

```
✓ Resuming job 01KRPVJX...M4S59Z5D with stage 8 bypassed.
  Status: queued. Watching for next event.
```

Operator goes back to lunch. The runtime advances past the
bypassed stage. The audit trail records the bypass with the
Slack user as the operator.

### Journey 2 — "I want to give the agent guidance"

Same setup, but the failure is the agent making a wrong choice
the operator wants to correct rather than bypass:

```
@codeless resume 01KRPVJX...M4S59Z5D "When you redo this stage,
do not list the design doc by name in the Done section; only list
files you actually created or modified."
```

The bot threads the comment into the next stage's prompt as a
preamble. The agent re-runs the stage with the operator's
guidance. Either it now passes, or it fails again with new
information the operator can act on.

### Journey 3 — "Just tell me the state of everything"

The operator opens Slack between meetings:

```
@codeless status
```

Bot replies:

```
3 active jobs across 1 repo:
  • 01KRPVJX...M4S59Z5D  Failed at 8/13   scope-mutable-ui    $52.64
  • 01KRPRT2...3PA3MV    Completed 4/4    smscope-smoke       $0.21
  • 01KRPS53...M0TTC     Failed at 2/4    smscope-smoke-2     $0.66

Reply: status <id> for details, resume <id> [bypass | "<comment>"] to act.
```

10 lines. Glance, act, leave. That's the whole point of Surface 1.

## Dependencies — what has to land before each surface

Numbering below is local to this doc. SCOPE-MUTABLE-UI uses its
own numbering (e.g. its #6a is this doc's #1, its #1 is this
doc's #5); cross-references call out both numbers in parens.

| Surface | Backend | Slack-side |
|---------|---------|------------|
| 1 (keep-it-running) | Dep #1 (`resume_job.bypass`), Dep #2 (`resume_job.next_stage_comment`) | Dep #3 (Slack adapter crate); bot user setup |
| 2 (REVIEW context) | Dep #5 (`ReviewPreCheck` / `ReviewVerdict` events) | Event subscriber + formatter |
| 3 (submit) | Dep #4 (`submit_job_from_template_name` RPC) + `SubmitJobArgs.auto_bypass_policy` | Command parser |
| 4 (policy / hands-off) | SCOPE-MUTABLE-UI #7 (`auto_bypass_policy` column + thrashing guard) | `policy <id>` parser; muted notification formatter |
| 5 (patches) | (deliberately out of scope) | (deliberately out of scope) |
| 6 (inbox) | None | `list_jobs` filter |

### Dependency #1 — `resume_job` accepts `bypass` (= SCOPE-MUTABLE-UI #6a)

This integration is the *second* consumer of the bypass arg (the
web UI is the first). Lifting it out of the scope-mutable-ui job and
landing it as a small standalone PR is a real option — both
consumers benefit, and the Slack integration unblocks faster.

### Dependency #2 — `resume_job` accepts `next_stage_comment`

New `ResumeJobArgs` field:

```rust
pub struct ResumeJobArgs {
    pub job_id: JobId,
    pub additional_cost_cap_cents: Option<i64>,
    pub additional_wall_clock_cap_ms: Option<i64>,
    pub bypass: Option<BypassRequest>,           // from #6a
    pub next_stage_comment: Option<String>,      // new
}
```

The runtime threads `next_stage_comment` into the prompt
assembly for whichever stage runs next after resume. Format:
the comment is rendered above the stage prompt as an
`## Operator comment` block. The agent sees it as guidance,
not as part of the stage's own goal.

**Mobile-safe:** field lives in `codeless-rpc`. Stage prompt
assembly is `codeless-runtime`. No new dependencies.

**Audit:** the comment is captured as a `JobResumed` event
payload field so `git log` of the resume sequence can show
what guidance the operator gave when.

The `JobResumed.actor` field added by this integration is
**not Slack-specific**. The runtime stamps whichever client
identifier the RPC carries — web UI sessions populate it with
the UI session token, the CLI populates it with the local
username, the Slack bot populates it with the Slack user ID.
The field is for audit only and never participates in
authorisation (R5). A short note on this should also land in
SCOPE-MUTABLE-UI so the web UI side does not assume the field
is Slack-only.

### Dependency #3 — Slack adapter crate

New crate `codeless-slack` (or feature on `codeless-server`),
host-only per R1. Spawns a thread that:

1. Connects to Slack via Bolt SDK or direct Web API + Socket
   Mode.
2. Parses inbound messages matching the command grammar.
3. Translates each command to an `RpcClient` call.
4. Subscribes to the event bus (existing `subscribe` RPC) and
   posts outbound notifications on `JobFailed` / `JobStopped`.

**R1:** the crate contains no `tokio::process` or
`std::process::Command` imports, and is not in
`codeless-adapters-host`. The existing CI grep
(`no-process-spawn-outside-adapters-host`) enforces this — it
is the same enforcement applied to every other host-side
crate. A Slack websocket is not a subprocess; the
rule's enforcement mechanism is the grep, not the description.

**R2:** the crate is server-side, not UI; R2 (single transport)
does not constrain it.

**R5:** the bot has a single bearer token, read at startup
from the secrets store at `~/.config/codeless/secrets.toml`
(see Risk 4 below). The env var `CODELESS_BOT_TOKEN` exists
*only* as a first-time-setup convenience for
`setup/init-session.sh` to write the secrets-store row; the
long-running server never reads it. The bot is authorised as
the single Codeless tenant. The Slack workspace's
own auth controls *who can talk to the bot*; once they can, they
are the operator.

### Dependency #4 — `submit_job_from_template_name` RPC

Reads `<repo>/.codeless/jobs/<name>/template.yaml`, forwards to
`submit_job`. The web UI's rerun path already does this; extract
the helper.

## Open questions worth fighting about

1. **One channel or many?** Argument for one: a single
   `#codeless` channel where the bot posts notifications and
   the operator types commands. Argument for many: each repo
   gets its own channel; notifications go to the matching
   channel only. Probably: configurable per repo (a
   `slack_channel: String` field on the Repo row), defaults
   to a single channel from env.
2. **DM vs channel commands?** Both should work. Notifications
   go to the configured channel; commands can come from DM or
   from any channel the bot is in. The bot replies in the same
   thread the command came from.
3. **What about Slack user → operator mapping?** R5 says one
   trust boundary; the bot is the operator. But the audit
   trail should still record *which Slack user* typed each
   command. The `JobResumed` event payload grows a
   `actor: Option<String>` field with the Slack user's ID.
   It is not used for authorisation — only for the audit
   log.
4. **Rate limits / spam protection?** Yes. The bot limits one
   command-from-Slack per second per job. Bursty `resume`
   commands are reasonable from a human but should not flood
   the runtime if a bot or automation goes wrong.
5. **What about job submission with a typo?** The bot
   confirms with a 5-second cooldown: after `submit`, it
   replies "submitting…" and waits 5 seconds before actually
   calling the RPC, posting a one-line "type `cancel`
   <token>" if the operator wants to abort. This is the
   ONE place the bot earns its confirmation friction
   because the action is creative, not corrective.
6. **What about the `comment` containing quote characters?**
   The grammar uses double-quotes; embedded double-quotes
   need to be escaped or the grammar should use a different
   quote convention (triple-backticks?). Pick one in
   implementation; document it in the bot's help text.

## What this ramp deliberately does not include

- **Slack message editing.** The bot does not edit prior
  notifications when state changes. If a job goes
  Failed → Resumed → Running, that is two separate messages,
  not one message that mutates. Editing is fragile (Slack
  rate-limits edits aggressively, and the message history
  becomes a worse audit trail than two distinct posts).
- **Slack workflows / scheduled actions.** The bot does not
  let the operator type "resume this job at 9am tomorrow."
  Scheduling is what cron is for; mixing it into Slack is
  scope creep.
- **Multi-job batch operations.** No `resume all failed`.
  Each command targets exactly one job. Batch operations
  invite the wrong-job mistake the careful per-job grammar
  exists to prevent.
- **Approval flows for `bypass`.** Bypass is a single
  operator's call. R5 means there is no "another team
  member must approve" step. The audit trail (the
  StageBypassed event with the Slack user ID) is the
  accountability story.
- **Slack message reactions as decisions.** Already named
  as an anti-pattern above. Reactions are signals, not
  commands.
- **Conversation threading with the agent.** The
  `[Talk to agent]` flow from SCOPE-MUTABLE-UI.md's Surface
  E is genuinely interactive and lives in the web UI.
  Slack carries one-shot comments via `resume "<comment>"`;
  multi-turn agent chat is the wrong shape for a chat-app
  surface.

## Risk and the failure modes

**Risk 1 — Wrong job ID.** Operator types `resume 01KRP...PT2MJ`
when they meant `01KRP...PVJX`. The job goes from a state the
operator did not want to change. Mitigation: `resume` echoes the
job's template name in the reply ("Resuming smscope-smoke-2…").
That reply is the load-bearing safety net — it is **never
debounced**, only outbound event notifications are (see Risk 2).
A visible name disagreement gives the operator a chance to type
`stop` before too much damage is done. The cost cap is a weak
backstop: on a fresh job at $0/$150 a wrong-target `resume` has
the full cap of headroom, so the template-name echo is doing
almost all the work here.

**Risk 2 — Notification noise.** The bot posts every
`JobFailed` and the operator's Slack gets unreadable. Mitigation:
the first scope's notification firehose is just *terminal*
transitions (Failed, Stopped). Verbose mode is opt-in. If a single
job is in a retry loop, the bot debounces *event-driven outbound
notifications* (one per job per 5 minutes max). Command replies
are not debounced — every `resume` / `stop` / `status` always
gets its synchronous confirmation.

**Risk 3 — Bypass abuse.** The operator gets in the habit of
bypassing every gate. The runtime never catches a real failure.
Mitigation: bypass is logged in `StageBypassed` events; a future
Surface (maybe in SCOPE-MUTABLE-UI.md) shows "% of stages
bypassed in the last 7 days" as a maturity signal. The bot itself
does not gate bypass usage — that is what review and audit are
for.

**Risk 4 — Bot token leak.** The bot has a bearer token
authorising it as the Codeless operator. If the token leaks, the
attacker can act as the operator from any Slack account they can
plant a message from. Mitigation: store the token in
`~/.config/codeless/secrets.toml` (existing secrets store); the
`CODELESS_BOT_TOKEN` env var is read by `init-session.sh` once
at setup and written into the secrets store, never read by the
long-running server. Rotate the token if any operator suspects
compromise. R5 means the blast
radius is the entire Codeless workspace, so this matters.

**Risk 5 — Slack platform outage.** Codeless still works without
Slack; the bot is additive. If Slack goes down, the operator
falls back to the web UI or the CLI. The integration is not in
the runtime's hot path; it subscribes to the event bus the same
way any other consumer does.

## What lands where in the codebase

- **New crate** `codeless-slack` under `codeless/crates/`. Host-
  only per R1. Cargo features behind `--enable-slack` on the
  `codeless serve` CLI so deployments without Slack pay zero
  cost.
- `codeless-rpc/src/methods.rs` — `ResumeJobArgs` gains `bypass`
  (shared with SCOPE-MUTABLE-UI #6a) and `next_stage_comment`.
- `codeless-runtime/src/rpc/jobs.rs` — resume path reads
  `next_stage_comment` and stamps a `JobResumed` event with
  `comment` and `actor` fields.
- `codeless-runtime/src/template_runner.rs` — next-stage prompt
  assembly reads the comment if present and prepends an
  `## Operator comment` block.
- `codeless-types/src/event.rs` — `JobResumed` event variant
  gains `comment: Option<String>` and `actor: Option<String>`
  fields. Backwards-compatible serde (missing fields = None).
- New `codeless-server/src/slack.rs` or `codeless-slack/src/
  bot.rs` (depending on whether it's a feature or its own
  crate) — Slack adapter implementation.
- `setup/init-session.sh` — `--enable-slack` flag plumbing,
  bot token env var, channel config.
- `codeless/.codeless/jobs/slack-integration/` — the per-job
  scope dir for this work, when it gets turned into a real
  Codeless job.

**R1:** the Slack adapter spawns no subprocesses beyond the
SDK's websocket. Confirmed via the existing
`no-process-spawn-outside-adapters-host` predicate; the new
crate is not in `codeless-adapters-host`, so any
`process::Command` in it would fail CI.

**R5:** unchanged. One bot, one bearer token, one operator
trust boundary. The Slack user ID is captured for audit only.

## What ships, in order

A ramp, not a tier list.

### Step 1 — Dependencies (RPC arg additions) — scaffolding, not a stopping point

Land `bypass` and `next_stage_comment` on `ResumeJobArgs`,
together as one small PR. The bypass piece is shared with
SCOPE-MUTABLE-UI Surface E and unblocks both consumers; the
comment piece is new and small. Both are R1-mobile-safe (wire
types in `codeless-types`, impl in `codeless-runtime`).

This is scaffolding only — nothing operator-visible yet. Do not
mark this as a stopping point; Step 2 must follow before this
integration delivers any value. (The web UI side of #1 ships on
its own timeline via SCOPE-MUTABLE-UI; that is a separate
consumer of the same plumbing.)

### Step 2 — Slack adapter scaffold + Surface 1 commands

New crate `codeless-slack`. Bolt SDK or equivalent. Bot user
setup, env var for token, command grammar parser. Five
commands: `status`, `status <id>`, `start`, `stop`, `resume
<id> [bypass] [<comment>]`. No outbound notifications yet.

Stopping here: the operator can drive a job entirely from
Slack provided they already know the job ID. No surprises.

### Step 3 — Outbound failure notifications

Subscribe to the event bus, post on `JobFailed` and
`JobStopped`. Format per Surface 1's mockup. Per-job 5-minute
debounce.

Stopping here: the keep-it-running loop is end-to-end
operational from a phone. **This is the first scope's
done line.**

### Step 4 — Surface 2 REVIEW gate context

Subscribe to `ReviewPreCheck` and `ReviewVerdict` events
(SCOPE-MUTABLE-UI Dependency #1). Enrich the failure
notification with the structured pre-check / verdict block.
Without this, the notification reads "diff-verify failed"; with
it, the reader sees the path list and the offending bullet.

### Step 5 — Surface 4 policy commands (`hands-off` mode)

Add the `policy <job-id> [<preset> | custom "<text>" | none]`
command and the corresponding muted "auto-bypassed"
notification + thrashing-halt notification. Lifts the
`SubmitJobArgs.auto_bypass_policy` field into the
`@codeless submit … policy:<preset>` form so a job can be
submitted hands-off from a single Slack message.

This step is **blocked on SCOPE-MUTABLE-UI Dependency #7**
(the runtime side: `auto_bypass_policy` column + the
auto-bypass branch in the stage-failed handler + the
thrashing guard). Slack-side work is small once that lands.

Stopping here: the operator can submit a long-running job
from Slack, set a policy, and walk away. The runtime
auto-advances under the policy and posts a louder
notification only when the thrashing guard fires.

### Step 6 — Surface 3 submit + Surface 6 inbox

The two minor commands. `submit <repo> <template>
[policy:<preset>]` (with the 5-second cooldown). `inbox`
(filtered `list_jobs`). Both ship together because they are
small.

The ramp ends here.

### Stopping points

- Stop at Step 2: commands work; no notifications. Useful for
  trusted operators who already watch their jobs another way.
- Stop at Step 3: **the first scope is complete.** Operator can
  keep any job moving from a phone.
- Stop at Step 4: failures include rich context; bypass
  decisions become better-informed.
- Stop at Step 5: **hands-off operator mode is live.** Long
  jobs run unattended under a Slack-set policy.
- Reach Step 6: the operator's keyboard-free Codeless surface
  is fully operational, including job creation.

Each stopping point is a real win. Step 3 is the production-
readiness line. Step 4 makes the bypass decisions sharper.
**Step 5 is the hands-off line** — the difference between "stay
glued to Slack" and "submit it and check back." Step 6 fills in
the convenience commands.

## Pointers

- The runtime this integration sits on:
  [`SESSION-MUTABLE-SCOPE.md`](./SESSION-MUTABLE-SCOPE.md)
- The web-UI surfaces this integration shares dependencies
  with: [`SCOPE-MUTABLE-UI.md`](./SCOPE-MUTABLE-UI.md)
- Resume / state-machine reference:
  [`crates/codeless-runtime/src/rpc/jobs.rs`](../codeless/crates/codeless-runtime/src/rpc/jobs.rs)
- Event bus subscription pattern (for the outbound
  notifications): existing `subscribe` RPC + `EventEnvelope`
  serde shape
- The CLI this integration sits next to (does not replace):
  `codeless` (the operator on a desktop still has it)
