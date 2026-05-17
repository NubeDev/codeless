# Scope — telegram-integration

## Goal

Ship the Telegram counterpart to the Slack integration: a bot that
lets the operator drive Codeless jobs from a phone, away from a
keyboard. Surface 1 first — `/status`, `/start`, `/stop`,
`/resume` (with `bypass` and quoted comment args), plus outbound
failure notifications. The two integrations are siblings, not
alternatives; they share the same runtime contract, the same
event-bus subscription, the same operator trust boundary (R5).

The full design rationale, surface ramp, dependencies, risks, and
operator setup live in
[`DOCS/SCOPE-TELEGRAM-INTEGRATION.md`](../../../DOCS/SCOPE-TELEGRAM-INTEGRATION.md).
That doc is the source of truth for the *why*; this scope is the
*what + how* for this specific job.

## In scope

- Telegram transport for the operator-control-plane bot.
- Long-polling event loop (no public HTTPS webhook required).
- Surface 1 commands end-to-end: `/status`, `/status <id>`,
  `/start`, `/stop`, `/resume` with `bypass` and quoted comment.
- `reply_to_message_id → job_id` mapping so reply-to-notification
  commands can omit the job ID (mirror of Slack's `thread_ts →
  job_id`).
- Outbound failure notifications on `JobFailed` and `JobStopped`,
  one per terminal transition, debounced 5 minutes per job.
- Allowlist of Telegram user IDs as noise/typo protection (NOT a
  trust boundary).
- Bot token, chat ID, allowlist loaded from
  `~/.config/codeless/secrets.toml` via the existing `SecretStore`.
- `--enable-telegram-bot` flag on `codeless serve`; deployments
  that don't want Telegram pay zero cost.
- Shared parser + event-subscriber + rate-limiter layer with the
  Slack integration (see "Constraints / Approach" below).

## Out of scope

- **The RPC scaffolding (`ResumeJobArgs.bypass`,
  `ResumeJobArgs.next_stage_comment`, `JobResumed.actor`,
  `JobResumed.comment`).** That work is owned by
  [`.codeless/jobs/slack-integration`](../slack-integration/) and
  this job must NOT duplicate or fight with it. Stage 1 of this
  job is a hard gate: if Slack stage 1 has not landed on main,
  this job halts and waits.
- Surface 3 (`/submit`), Surface 4 (`/policy`), Surface 6
  (`/inbox`) — out of v1, ship after the keep-it-running loop is
  live.
- Surface 5 (patch approvals) — deliberately never in this
  integration; see the design doc.
- Inline keyboard buttons as the primary command path. May land
  later as a UX polish PR; not in v1.
- Webhook transport. Long-polling only in v1.
- BotFather `/setcommands` autocomplete registration. The grammar
  is in the parser; registering it with Telegram is a second
  source of truth and a drift hazard. Defer.
- Inline mode (`/setinline`). Wrong UX shape for an operator
  control plane.
- A standalone `codeless-telegram` crate parallel to
  `codeless-slack`. Use the shared `codeless-bot` / `codeless-bot-
  core` layout per the design doc's Dep #3 recommendation.

## Constraints

- **R1** — host-only crate. No `tokio::process` or
  `std::process::Command` imports. Confirmed by the existing
  `no-process-spawn-outside-adapters-host` predicate.
- **R5** — one bot, one bearer token, one operator trust
  boundary. The Telegram user ID is captured into
  `JobResumed.actor` for *audit only*, never for authorisation.
  The `allowed_user_ids` list is defence-in-depth, not the trust
  boundary; if it is empty the adapter refuses to start
  (fail-closed).
- **Bot-token handling** — read from secrets store only.
  `CODELESS_TELEGRAM_BOT_TOKEN` is a setup-time env var consumed
  by `setup/init-session.sh` and written into the secrets store;
  the long-running `codeless serve` never reads the env var
  directly.
- **`teloxide`** is the default Rust client. Pick a different one
  only if a real blocker is hit; the choice is not load-bearing
  for the scope.
- **Rate limits** — respect Telegram's 30 msg/sec/chat outbound
  limit via a token-bucket on outbound posts. Internal per-job
  rate limit of 1 inbound command/sec is identical to Slack.
- **No new RPC fields.** Everything Telegram needs already lands
  in slack-integration stage 1.
- **No drift from the design doc.** If a design decision in this
  job conflicts with
  [`DOCS/SCOPE-TELEGRAM-INTEGRATION.md`](../../../DOCS/SCOPE-TELEGRAM-INTEGRATION.md),
  update the doc in the same commit — never let code and doc
  diverge silently.

## Approach — shared code with slack-integration

**Locked in stage 2: Approach path 1 — extract a `codeless-bot-core`
crate.** Rationale and the planned crate / module layout live in
[`DOCS/SCOPE-TELEGRAM-INTEGRATION-DECISIONS.md`](../../../DOCS/SCOPE-TELEGRAM-INTEGRATION-DECISIONS.md).
Summary:

- `codeless-bot-core` (new, host-only) — transport-agnostic
  parser, `BotTransport` trait, `CommandBackend` slice of
  `RpcServer`, `ReplyContextMap<K>` (was `ThreadMap`), reply
  renderers, and the inbound + outbound rate limiters. Both
  transports depend on it.
- `codeless-slack` (existing, on `codeless/slack-integration`)
  rebases to depend on `codeless-bot-core`. Keeps `socket_mode`,
  `web_api`, `config`, and the Slack-shape envelope decoder.
  The crate's public API is preserved via re-exports from
  `codeless-bot-core` so slack-integration's stages 5+ rebase
  trivially.
- `codeless-bot` (new, this job) — the Telegram transport. Owns
  the long-poll client, the `teloxide` (or equivalent) glue,
  the Telegram-shape envelope decoder, and the
  `codeless-bot-core::BotTransport` impl. The name `codeless-bot`
  is reserved on the assumption that further transports (Matrix,
  Discord, SMS, …) join as sibling crates under the same core
  rather than as more modules inside one crate.

Path 2 (single `codeless-bot` crate with
`transport/{slack,telegram}.rs`) was rejected because
`codeless-slack` already exists with shipped stages on the
`codeless/slack-integration` branch (commits `293ff33`, `129054a`,
`ffa11bd`). Folding it into a transport module would touch every
file in `codeless-slack` and force a rebase of every slack-
integration stage from 5 onward; extraction touches only the
crate's re-export surface.

The REVIEW gate after this stage locks the choice in with the
operator before any Telegram code is written. The actual
extraction (creation of `codeless-bot-core`, moving files out of
`codeless-slack`) is its own stage, and it MUST wait until
slack-integration's stage-1..N work is merged to `master` —
performing the refactor against the in-flight `codeless/slack-
integration` branch from this worktree would re-introduce exactly
the cross-branch coupling the gate is there to prevent.

## Open questions

1. **Branch naming for the extraction stage.** The
   `codeless-bot-core` extraction is its own stage *inside* this
   job, not a separate job; it lands on
   `codeless/telegram-integration` after slack-integration merges
   to `master`. The slack-integration branch is then expected to
   rebase onto the post-extraction `master` (its stage 5 onward
   imports from `codeless-bot-core` instead of redefining the
   shared types in-crate). If the slack-integration owner pushes
   back on that rebase, the fallback is to spin
   `codeless/codeless-bot-core` as a tiny intermediary branch
   that both this job and slack-integration rebase onto — but the
   default is "one extraction stage in telegram-integration."
2. **Where does `--enable-telegram-bot` live in the CLI?** Next
   to `--enable-slack` in `crates/codeless-cli/src/serve.rs` is
   the obvious place; confirm at scaffold time.
3. **`allowed_user_ids` in `secrets.toml` shape.** TOML supports
   arrays of integers; the `SecretStore` today stores flat
   `String` values. Options: (a) extend `SecretStore` to support
   typed values (scope creep); (b) store as comma-separated
   string and parse at adapter startup; (c) move telegram chat
   config out of `secrets.toml` into a separate `telegram.toml`.
   Lean (b) — smallest change, matches the "secrets store is for
   secrets, not config" spirit.
4. **`chat_id` per repo vs. global.** The design doc says "a
   `telegram_chat_id: i64` field on the Repo row" — but Codeless
   today is R5 single-tenant with one operator. A global chat ID
   is simpler for v1; per-repo chat IDs land if/when an operator
   actually has two repos they want piped to two different chats.
   Default to global.
5. **Test strategy.** The Slack integration likely ships with a
   mock transport for unit tests. Reuse it: the shared
   `BotTransport` trait is what the mock implements. Telegram-
   specific tests are limited to the parser's `/`-prefixed
   command surface and the long-polling client glue (which can
   be smoke-tested against `api.telegram.org` with a throwaway
   bot).
