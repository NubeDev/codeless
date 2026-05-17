# JOB-MODEL — the user-facing framework

> **Sister docs.** This is the *user-facing* contract for what
> codeless does. [`SCOPE.md`](./SCOPE.md) is the product scope.
> [`LOOP-CODER.md`](./LOOP-CODER.md) is the design intent for the
> 24-hour unsupervised run. [`JOB-EXAMPLE.md`](./JOB-EXAMPLE.md) is
> a worked walkthrough. [`JOB-LOOP.md`](./JOB-LOOP.md) is the
> developer-only loop used to *build* codeless and is unrelated to
> this doc.
>
> If anything below contradicts SCOPE.md, **SCOPE.md wins** — fix
> this file rather than diverge.

## What codeless is for

VS Code, Cursor, JetBrains, and Claude Code already do the editing,
the chat, the refactor, the autocomplete. They are good at all of it.
**Codeless does not compete with any of that.**

Codeless does the one thing those tools don't: **run a long,
unsupervised coding session for hours, across many fresh agents,
without losing context or burning the budget.** Start a job before
bed, wake up to a branch with commits. That's the product. The rest
of this doc is what it takes to make that real.

## The framework — three files, fixed format

```
acme/app/                                 ← user's repo
├── .codeless/
│   ├── config.yaml                       ← repo defaults
│   └── jobs/
│       └── <name>.yaml                   ← one file per job
└── runs/
    └── <name>/
        ├── handover.md                   ← current state (overwritten each session)
        └── log.md                        ← session history (append-only)
```

Two committed config files. Two committed run files per job. SQLite
exists for the UI's live event feed, but the framework above does
not depend on it — if `codeless.db` is wiped, the next session reads
`handover.md` and the job continues.

Everything is plain text in the user's repo. VS Code, Cursor, vim,
GitHub diff view — all of them work on it. **That's the point.**
Codeless does not try to be the editor; it leaves the editing to
the tools that already do it well.

### Where the job actually runs — the worktree

A job does not run in the user's main checkout. When a job is
queued, codeless cuts a `git worktree` off the user's repo onto a
new branch named `codeless/<job-name>-<id>` (e.g.
`codeless/user-profile-42`). Every session attaches to the same
worktree, sees what previous sessions committed, and pushes to the
same branch. The worktree persists for the job's lifetime, not the
session's — only when the job reaches a terminal state is the
worktree reaped.

The user can `cd` into the worktree at any time and open it in any
editor. Edits the user makes there land on the job's branch like
any other commit. The next session sees them.

### Files in the user's repo vs. the runtime's data

| Path | Who writes it | Committed? | Why |
|---|---|---|---|
| `.codeless/config.yaml` | User | yes | Repo defaults the user authors. |
| `.codeless/jobs/<name>.yaml` | User | yes | Job template the user authors. |
| `runs/<name>/handover.md` | Session (and user, between sessions) | yes | The contract between sessions. |
| `runs/<name>/log.md` | Session | yes | Audit trail of every session. |
| `$XDG_DATA_HOME/codeless/codeless.db` | Runtime | n/a (not in repo) | Live event log, UI queries. Cache. |
| `$XDG_DATA_HOME/codeless/worktrees/<job-name>-<id>/` | Runtime | n/a (gitignored worktree) | The actual checkout the agent edits in. |

The user's repo holds **inputs and the inter-session contract**.
The runtime's data dir holds **derived state and the worktrees**.
Wipe the runtime's data dir and the next session can still
reconstruct everything it needs from the repo. The reverse is not
true — the runtime cannot reconstruct user-authored templates.

## `.codeless/config.yaml`

Four fields. That's the whole repo config.

```yaml
runner: claude-code-cli
cost_cap_usd: 5.00
wall_cap_hours: 8
verify: cd backend && go test ./... && cd ../frontend && pnpm test
```

| Field | Meaning |
|---|---|
| `runner` | Which coding runner to use. Today: `claude-code-cli`, `codex-cli`, `anthropic-api`, `openai-api`. |
| `cost_cap_usd` | Hard ceiling for any one job. Loop halts at this number. |
| `wall_cap_hours` | Hard ceiling on wall-clock time per job. |
| `verify` | One shell command. Must exit 0 for a stage to count as done. Runs from the worktree root. |

A job can override any of these in its own YAML.

## `.codeless/jobs/<name>.yaml`

Three fields. `name`, `goal`, `stages`.

```yaml
name: user-profile
goal: Add GET /api/users/:id endpoint and a profile page.

stages:
  - add User model and migration
  - add GET /api/users/:id handler with tests
  - REVIEW api shape before frontend uses it
  - add getUser(id) on the frontend RPC client
  - build UserProfilePage component
  - wire the /users/:id route
```

Rules:

- `name` is unique per repo. A second job with the same name fails
  at queue time; saving an ad-hoc run with `--save-as <name>`
  refuses to overwrite an existing template.
- `stages` is an ordered list of strings. Each string is one stage's
  title and is also the commit subject when the stage lands.
- **`REVIEW`-prefixed stages are the user-authoring surface for
  review gates.** A stage whose title starts with `REVIEW` halts the
  loop and waits for a human. The runtime materialises it into a
  `Review` row in SQLite (per [`SCOPE.md`](./SCOPE.md) — `Review` is
  a state on a Stage, joined via `stage_id`). The user resolves it
  via the UI, the CLI, or by moving the stage line from `Next` to
  `Done` in `handover.md`.
- No tags, no IDs, no per-stage flags. If a stage is too big, the
  agent says so in the handover and the user splits it by editing
  this file.
- The user can add, remove, or rewrite stages mid-run by editing
  this file. The next session reads it. Stages already done stay
  done; everything else is re-read.

## `runs/<name>/handover.md` — the contract between sessions

This is the load-bearing file. Every session reads it first; every
session rewrites it before exiting. **The handover is the only
durable knowledge transfer between sessions.**

The runtime writes this file automatically on job termination. CLI
runners (claude, codex) are told via system prompt to end their
final reply with a fenced ```handover block whose body contains the
four `##` sections below; the runtime extracts it verbatim. Runners
that emit nothing parseable get a default fallback whose `Done`
section names the runner and status and whose `What you need to
know` section carries a truncated tail of the assistant's final
message. The fallback is observably worse than a real handover but
prevents a blank file from blocking the next session.

```markdown
# Handover — user-profile

Last updated: 2026-05-13 13:47 by session 3 of this job.
Branch: codeless/user-profile-42, head abc1234.

## Done
- add User model and migration
- add GET /api/users/:id handler with tests
- REVIEW api shape (approved by ap@nube-io.com)
- add getUser(id) on the frontend RPC client
- build UserProfilePage component

## Next
- wire the /users/:id route

## What you need to know
- Router is react-router-dom v6. Use <Route>, not <Switch>.
- The user object uses `display_name`, not `name`.
- Frontend verify takes ~40s — that's normal, not a hang.

## Don't redo
- Tried adding an avatar — the backend has no avatar_url field.
  Confirmed with the user at the REVIEW stage. Don't add it back.

## Where I stopped
Worktree clean. Last commit pushed. Stage 6 is what's next.
```

Five fixed sections, in order. If a section is empty, the heading
stays with `(none)` underneath.

| Section | What goes in it |
|---|---|
| `Done` | Stages completed across the whole job. The full list, every session. |
| `Next` | The remaining stages, in order. The first item is what the next session runs. |
| `What you need to know` | Facts the agent learned that aren't obvious from the code. Curated each session — drop entries that no longer matter. |
| `Don't redo` | Approaches tried and rejected, with the reason. Stops the next agent from re-walking the same dead end. |
| `Where I stopped` | Worktree state, last commit, what's next. One paragraph. |

A session that can't write a clean handover (mid-stage failure,
runtime crash, hard cap hit) writes whatever it can, marks the job
halted, and exits. **A blank handover is forbidden** — at minimum a
session writes "halted at stage N, reason X". codeless-runtime then
does not fire the next session.

## `runs/<name>/log.md` — append-only audit

One block per session, never rewritten. The handover has the
content; the log has the receipts.

```markdown
# Log — user-profile

## Session 1 — 2026-05-13 10:00 → 11:14
Did: stages 1, 2.
Cost: $0.31. Reason for ending: context handoff.

## Session 2 — 2026-05-13 11:15 → 12:13
Did: stage 3 (REVIEW — paused for user).
Cost: $0.05. Reason for ending: review gate.

## Session 3 — 2026-05-13 12:14 → 13:47
Did: stages 4, 5.
Cost: $0.62. Reason for ending: context handoff.
```

Three fields per session: what got done, how much it cost, why it
ended. That's enough to debug a bad overnight run.

## The loop, end to end

```
Session N starts:
  1. Read .codeless/jobs/<name>.yaml          (what to do)
  2. Read runs/<name>/handover.md              (what last session learned)
  3. Read CODELESS.md, CLAUDE.md               (repo-level rules)
  4. Read git log                              (what actually landed)
  5. Pick the first item from "Next" in handover.

Session N works:
  - Does one or more stages, in order.
  - Each stage: edit code, run verify, **commit, push** — push
    after every stage, never deferred to end-of-session.
  - If a REVIEW stage: write a one-line summary, halt.
  - If verify fails: stop, do not try the next stage.

Session N exits:
  1. Rewrite handover.md (Done / Next / What you need to know /
     Don't redo / Where I stopped).
  2. Append a block to log.md.
  3. Commit both files together with message "handover N".
  4. Push.
  5. Exit.

codeless-runtime fires Session N+1.
```

### Resolving a REVIEW stage out-of-band

When the user clicks **Approve** on a REVIEW gate in the UI (or
runs `codeless review approve <job>:<stage>`), no session is
running. The runtime itself writes the handover update:

1. Updates the `Review` row in SQLite (status, who approved, when).
2. Rewrites `runs/<name>/handover.md` — moves the REVIEW line from
   `Next` to `Done`, suffixed `(approved by <user>)`. No other
   sections change.
3. Commits with message `review approved: <stage title>` and a
   committer/author identity of the runtime's configured git user
   (typically the same as the human's `git config`).
4. Pushes.
5. Schedules the next session.

The user's alternative is to edit `handover.md` by hand: move the
REVIEW line, commit, push. The next session sees the same end
state either way — the handover is the contract, and any party
(agent, runtime, user) that follows the contract is a valid writer.

### Crash-resumption invariant

Stage commits and handover commits are kept separate so `git log`
reads cleanly: code lands first, then "handover N" with the two
markdown files. This means a session can crash in three places:

| Crashes at | Recovery |
|---|---|
| After stage commit + push, before handover commit | The handover is stale, but stage is fully landed. Next session rebuilds Done/Next by diffing handover against `git log` and reconciles. |
| After handover commit, before push | Push is the next session's first action (idempotent). |
| Mid-stage, dirty worktree | Next session discards the dirty diff (no commit), replays the stage from scratch. |

**Handover commit is idempotent: replays of a fully-handed-over
stage produce no diff.** That's what makes the second row above
safe. It's also why the handover is a separate commit — folding
it into the stage commit would couple the code change to the
handover write and make replay harder.

### Every session is fresh

A new agent process, a new runner invocation, **no `resume_id`
carried forward**, no in-memory state. The only thing session N+1
inherits from session N is what's on disk and on the remote. That's
the whole point — see [`LOOP-CODER.md`](./LOOP-CODER.md) for the
design intent and the long-run constraint this protects.

## Ad-hoc jobs

A user can kick off a job without writing YAML — UI: enter a one-line
goal, codeless generates the stage list, user approves. CLI:
`codeless run --goal "..."` same thing. The generated job lives only
in SQLite by default; `--save-as <name>` writes it to
`.codeless/jobs/<name>.yaml` for re-use.

The planner that turns a goal into stages is a Rig helper, not part
of the coding loop. A failed planner doesn't break codeless — the
user can always hand-write the YAML.

## What codeless never does

- Write outside `.codeless/`, `runs/`, or the code paths the job is
  meant to change.
- Run `git push --force`.
- Skip verify or pass `--no-verify` to git.
- **Carry a runner session token (`resume_id`, `--continue`, or
  any provider-side conversation state) across sessions.** The
  runtime enforces this — it never passes a session token to the
  runner. This is what keeps the runner's context bounded over a
  long unsupervised run; see [`SCOPE.md`](./SCOPE.md) "Hard rules
  for the coding runner" and [`LOOP-CODER.md`](./LOOP-CODER.md)
  "What blocks this today".
- Auto-merge a PR.
- Touch SQLite as a substitute for the handover. The handover is
  the contract; SQLite is the runtime's private cache.

## What the user does, total

Authors:
- `.codeless/config.yaml` once per repo.
- `.codeless/jobs/<name>.yaml` once per job (or one click for
  ad-hoc).

During a run:
- Reviews `REVIEW` stages when they come up.
- Optionally edits `handover.md` to inject a fact ("use library X,
  not Y") or `<name>.yaml` to change direction.

At the end:
- Reviews the draft PR codeless opens, merges or rejects.

Everything else is the loop.

## Why this framework is robust

The framework survives anything because the rule is small enough to
hold in your head: **one YAML for what, one handover for state, one
log for history.**

| What goes wrong | What recovers it |
|---|---|
| Crash mid-session, worktree dirty | Next session reads the same handover the crashed one started from. The uncommitted diff is discarded; the stage replays. |
| `codeless.db` deleted | Next session reads handover.md and the job continues. SQLite was a cache. |
| Agent edits the wrong files | Revert the commit. Handover still says what should happen next. Stage retries. |
| User wants to inject knowledge | Edit `handover.md` between sessions. No new mechanism. |
| User wants to add a stage mid-run | Edit `<name>.yaml`. No new mechanism. |
| User wants to change direction | Edit both files. Stop the run. Re-queue. |
| Cost cap hit | Loop halts cleanly. Handover says where it stopped. User raises the cap or stops the job. |
| Cap fires mid-stage | The runtime kills the session before it can finish writing a clean handover. The next session sees a dirty worktree and a handover that doesn't match — the dirty diff is discarded (no commit), the stage replays from scratch. **Work between the last clean handover and the cap can be lost** — keep the cap headroom realistic for the stages you have. |

## What's deliberately not here yet

These exist as ideas but are not in the framework today. Each is a
feature, not a fix — add them when a real run shows the simple
version isn't enough:

- Stage size tags (S/M/L) and a batcher that combines small stages.
- Per-stage verify commands.
- Session modes (fresh / continue / sticky).
- Per-stage caps.
- Multiple verify commands keyed by file path.
- Rich review-gate metadata.
- A `sessions/` directory with per-session detail files.

The framework above is what ships first. The features above land
only after a real long run shows their absence is hurting users.
