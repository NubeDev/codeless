# JOB-EXAMPLE — a worked run

> Companion to [`JOB-MODEL.md`](./JOB-MODEL.md). Reference doc is
> JOB-MODEL; this is the walkthrough that makes it concrete. If the
> two disagree, **JOB-MODEL wins** — fix this file rather than
> diverge.

## The setup

A developer has a repo `acme/app` with a Go backend and a React
frontend. They want to add a user-profile feature. The repo already
has `.codeless/config.yaml`:

```yaml
runner: claude-code-cli
cost_cap_usd: 5.00
wall_cap_hours: 8
verify: cd backend && go test ./... && cd ../frontend && pnpm test
```

## The job

The user opens `.codeless/jobs/user-profile.yaml` in their editor
(VS Code, Cursor, vim — doesn't matter) and writes:

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

Commits it to `main`, pushes. Opens codeless, clicks **Queue
user-profile**. Closes the laptop.

## Session 1 — stages 1 and 2

codeless-runtime spawns a fresh agent. The agent:

1. Reads `.codeless/jobs/user-profile.yaml`.
2. Reads `runs/user-profile/handover.md` — doesn't exist yet, so
   treats the full stages list as "Next" and starts at item 1.
3. Reads `CODELESS.md`, `CLAUDE.md`, `git log` (just the branch
   cut from main).

It does stage 1 (User model + migration), runs verify, commits,
pushes. Then stage 2 (GET handler + tests), verify, commit, push.

By the end of stage 2 the agent's context is at ~70% — time to hand
off. It writes:

**`runs/user-profile/handover.md`:**

```markdown
# Handover — user-profile

Last updated: 2026-05-13 11:14 by session 1 of this job.
Branch: codeless/user-profile-42, head def5678.

## Done
- add User model and migration
- add GET /api/users/:id handler with tests

## Next
- REVIEW api shape before frontend uses it
- add getUser(id) on the frontend RPC client
- build UserProfilePage component
- wire the /users/:id route

## What you need to know
- Backend uses sqlc (not gorm) — see CODELESS.md.
- User model has fields: id, email, display_name, created_at.
- No avatar_url field. If the frontend wants one, it'll need a
  separate backend change.

## Don't redo
(none yet)

## Where I stopped
Worktree clean. Last commit pushed. Next is the REVIEW stage so
session 2 will halt almost immediately for the user.
```

**`runs/user-profile/log.md`** (appended):

```markdown
# Log — user-profile

## Session 1 — 2026-05-13 10:00 → 11:14
Did: stages 1, 2.
Cost: $0.31. Reason for ending: context handoff.
```

Commits both files: `git commit -m "handover 1"`. Pushes. Exits.

## Session 2 — the REVIEW gate

codeless-runtime fires session 2. Fresh agent. Reads
`handover.md`. First item in `Next` is a `REVIEW` stage.

The agent does no coding. It:

1. Looks at the diff between `origin/main` and `HEAD` — that's the
   API surface so far.
2. Writes a short review summary into the codeless UI's review
   queue: "API response shape is `{id, email, display_name,
   created_at}`. Confirm before frontend binds to it. No avatar field."
3. Updates `handover.md`: moves nothing to `Done`, but adds a
   one-liner under `Where I stopped` saying "paused on REVIEW —
   waiting for user".
4. Appends to `log.md`.
5. Commits "handover 2", pushes, exits.

codeless-runtime sees the job is awaiting review and does **not**
fire session 3.

## The user wakes up

7:12am. The user opens the codeless UI:

```
Job: user-profile               $0.36 / $5.00     1h 13m / 8h
─────────────────────────────────────────────────────────────
Awaiting review:
  REVIEW api shape before frontend uses it

  Summary: API response shape is {id, email, display_name,
  created_at}. Confirm before frontend binds to it. No avatar field.

  [ View diff ]   [ Approve ]   [ Comment ]   [ Stop ]
```

User clicks **View diff** in the UI (or opens the worktree in their
editor — same thing). Decides the shape is fine. Clicks **Approve**.

codeless updates `handover.md` automatically — moves the REVIEW
stage to `Done` with `(approved by ap@nube-io.com)` next to it —
commits, pushes, and fires session 3.

(Alternative path: the user could have edited `handover.md`
directly. Moving the REVIEW line from `Next` to `Done` and committing
has the same effect. codeless watches the file.)

## Session 3 — stages 4 and 5

Fresh agent. Reads `handover.md`:

```markdown
## Done
- add User model and migration
- add GET /api/users/:id handler with tests
- REVIEW api shape (approved by ap@nube-io.com)

## Next
- add getUser(id) on the frontend RPC client
- build UserProfilePage component
- wire the /users/:id route

## What you need to know
- Backend uses sqlc (not gorm).
- User has: id, email, display_name, created_at. No avatar_url.
- The router is react-router-dom v6 (just checked package.json).
- Frontend verify is slow (~40s) — that's normal.

## Don't redo
- Tried adding avatar fields — user explicitly said no avatar.
```

Note: the agent inherited the backend facts from session 1's
handover, and added its own new fact ("router is v6, just checked
package.json"). Curated, not appended.

Session 3 does stage 4 (RPC method — small) and stage 5
(UserProfilePage — medium). Verify green. Two commits, two pushes.

Context hits ~75%. Hand off. Rewrites `handover.md`:

```markdown
# Handover — user-profile

Last updated: 2026-05-13 13:47 by session 3 of this job.
Branch: codeless/user-profile-42, head jkl3456.

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
- The page is `frontend/src/pages/UserProfilePage.tsx`.

## Don't redo
- Avatar fields — user said no, backend has no field.

## Where I stopped
Worktree clean. Last commit pushed. Stage 6 is what's next —
needs to add a <Route> entry in App.tsx (line ~31).
```

Appends to log.md, commits "handover 3", pushes, exits.

## Session 4 — stage 6, done

Fresh agent. Reads handover. One stage left. Adds the route in
`App.tsx`, verify, commit, push.

All stages done. The agent:

1. Rewrites `handover.md` one final time with `Next: (none)`.
2. Appends to `log.md`.
3. Opens a draft PR via `gh pr create` with a body generated from
   `handover.md`'s `Done` list.
4. Marks the job complete in SQLite.
5. Does **not** schedule a successor.

## What the user did

- Wrote `.codeless/jobs/user-profile.yaml` (8 lines).
- Clicked **Queue** at the start.
- Clicked **Approve** on one review gate at 7:12am.
- Merged the draft PR.

## What codeless did

- 4 sessions across ~3h 45m wall time.
- 5 commits on `codeless/user-profile-42` (one per coded stage).
- 4 "handover N" commits with `handover.md` + `log.md` updates.
- Cost: $1.94 against the $5 cap.

## What it looks like on disk at the end

```
acme/app/
├── .codeless/
│   ├── config.yaml
│   └── jobs/
│       └── user-profile.yaml
├── runs/
│   └── user-profile/
│       ├── handover.md       ← final state, "Next: (none)"
│       └── log.md             ← 4 session blocks
├── backend/
│   └── ... (changed files from the job)
├── frontend/
│   └── ... (changed files from the job)
└── ... (rest of repo)
```

The user can `git log codeless/user-profile-42` to see what happened. They
can `cat runs/user-profile/handover.md` to see the final state.
They can `cat runs/user-profile/log.md` to see the per-session
cost trail. They can open any of it in VS Code or Cursor and read
it like any other file in the repo — because that's what it is.

## Failure paths

**A stage's verify fails.** Session 3 runs stage 5, verify fails.
Agent stops, does *not* try stage 6. Writes handover with stage 5
back in `Next` and a fresh entry under `Don't redo` describing what
it tried. Appends to log with reason "verify failed". codeless
flags the job halted in the UI; user looks at the diff, fixes the
issue (by editing files directly in the worktree, or by adding a
note to `handover.md`), and re-queues.

**Cost cap hit mid-session.** Session 3 spends $1.50 of remaining
$1.80 budget. codeless-runtime kills the session before it can
finish a stage. The killed session was *meant* to write a handover
but didn't get the chance. The next session sees the worktree may
be dirty and `handover.md` is stale. **Rule:** if the worktree is
dirty at session start, the session discards the dirty diff (no
commit) and starts over from where the last clean handover said.

**User edits the YAML mid-run.** Between session 2 and session 3
the user adds a stage `- add User settings page` after `wire the
/users/:id route`. Session 3 reads the YAML, sees the new stage,
treats it as part of `Next`. No special handling — the YAML is
re-read every session.

**User wants to inject knowledge.** Between session 2 and session
3 the user adds a line to `handover.md` under `What you need to
know`: `Use shadcn/ui Avatar component if we ever add avatars`.
Session 3 reads the handover, sees the new line, treats it like any
other entry. The agent is told nothing special — the handover is
the contract, the user is allowed to write to it.
