# ADDING-JOB — how to add a proper job to codeless

This is the workflow for **you (the operator)** to add a new job to a
running codeless server. You bring the **scope** — what you want
done, in/out, constraints, deliverables. The assistant (or you, if
you prefer) works out the **stages** and the **workflow** from the
scope.

For the architectural background see:

- [`DOCS/JOB-MODEL.md`](../../DOCS/JOB-MODEL.md) — three-fields template (name + goal + stages).
- [`DOCS/JOB-DIR.md`](../../DOCS/JOB-DIR.md) — directory layout and prompt assembly order.
- [`DOCS/JOB-WORKFLOW.md`](../../DOCS/JOB-WORKFLOW.md) — iterate loop + per-run notes.
- [`DOCS/HACKLINE-DEV.md`](../../DOCS/HACKLINE-DEV.md) — full SubmitJobArgs reference.

This doc is the **how-to**, not the why.

## TL;DR

```
You write:        SCOPE.md   ← what + why + constraints
Agent works out:  template.yaml (name, goal, stages)
                  WORKFLOW.md   (how to drive the work)
You add:          curl POST /rpc/submit_job  → status: "draft"
                  curl POST /rpc/write_job_file × N  (overlay docs)
You start (later): UI "start" button, or POST /rpc/start_job
```

The job lives at:

```
<repo>/.codeless/jobs/<name>/
├── template.yaml      ← spec the runner reads
├── SCOPE.md           ← brief, agent-read every stage
├── WORKFLOW.md        ← process, agent-read every stage
└── *.md               ← any other supporting docs
```

All three files are committed in the user's repo. `git log` records
every edit.

## Step 1 — write the scope

Start with **just the scope**. Don't pre-bake stages — that's the
agent's job. The scope is prose, no schema. A good `SCOPE.md` answers:

- **Goal** — one paragraph, what success looks like.
- **In scope** — bulleted, the deliverables.
- **Out of scope** — bulleted, what *not* to touch. This matters
  more than people think.
- **Constraints** — versions, libraries, naming conventions, R1–R5
  rules from CLAUDE.md, any "always X never Y" rules.
- **Open questions** — explicit. The agent should resolve these in
  stage 1, not silently guess.

A scope is a **brief**, not a spec. If you find yourself writing
implementation steps, stop — those are stages.

### Example scope skeleton

```markdown
# Scope — <job-name>

## Goal

<one paragraph describing the outcome a user gets>

## In scope

- <deliverable 1>
- <deliverable 2>

## Out of scope

- <something tempting but explicitly not this job>

## Constraints

- <version pin / library choice>
- <coding rule that must hold>
- R<N> from CLAUDE.md if it applies

## Open questions

1. <unresolved decision the agent must answer in stage 1>
```

Save it somewhere you can paste from — a scratch file, the chat,
whatever. You don't need to write it on disk first; the workflow
below puts it in the right place.

## Step 2 — derive the template

`template.yaml` is three fields. The agent (or you) translates the
scope into stages.

```yaml
name: <kebab-case, becomes the directory name>
goal: |
  <one-paragraph version of SCOPE.md goal — keep it short>

stages:
  - <stage 1 description, plain prose>
  - <stage 2>
  - REVIEW <gate description>      # any stage starting with REVIEW pauses for the user
  - <stage N>
```

Stage-writing rules:

- One **outcome** per stage, not one **action**. "Add the model and
  migration" is a stage; "create file `X`" is a step.
- Insert `REVIEW` gates at risky boundaries — before extraction,
  before destructive changes, before spending real money. Every
  REVIEW pauses the runner until you approve.
- Tag complexity (`(S)`, `(M)`, `(L)`) only if you care; the runner
  doesn't enforce it.
- Stages are **persistent** — they live in the repo. Don't write
  one-off "fix the typo" stages here; that's chat.

## Step 3 — derive the workflow

`WORKFLOW.md` is how the agent should *drive* the stages. Same
freeform markdown. A good `WORKFLOW.md` covers:

- **Sequencing** — which stages can be batched, which must stop.
- **Per-stage discipline** — what to read before writing, what to
  verify before committing, what counts as "done".
- **Commit + push at the end of every stage** — non-negotiable;
  see Hard rule 5 below. The workflow must say this explicitly so
  the agent doesn't end a stage with uncommitted work in the
  worktree.
- **REVIEW gate behaviour** — what to write into the handover at
  the gate. REVIEW gates still commit + push the stage that *led*
  to the gate; they only pause the *next* stage.
- **Anti-patterns** — patterns specific to this job that the agent
  should not adopt.

If the workflow is generic enough that it would apply to any job in
this repo, it belongs in `CLAUDE.md` instead, not here.

### Per-stage commit + push — exact wording for `WORKFLOW.md`

Paste this block into every job's `WORKFLOW.md` (adjust the branch
name). It survives stage boundaries because the agent re-reads
`WORKFLOW.md` at the top of each stage.

```markdown
## Commit + push after every stage

At the end of every stage — including stages that precede a REVIEW
gate, including stages that only edit docs — the agent MUST:

1. Stage every change the stage produced (`git add -A` from the
   worktree root, or specific paths if the stage was surgical).
2. Commit with the message `stage N: <one-line title from
   template.yaml>` so the history mirrors the template stages
   one-for-one.
3. Push to the job's branch (`codeless/<job-name>`) so the work is
   recoverable even if the worktree is wiped.

A stage is not "done" until the push succeeds. If the commit or
push fails, fix the cause and retry — do not mark the stage `[x]`,
do not advance, and never `--force` or `--no-verify`. If a stage
genuinely produced no change (e.g. an investigation stage that
only updated `SCOPE.md` and that doc was already current), say so
in the handover and skip the commit, but the next stage's commit
must include any side-effect files the investigation touched.
```

## Step 4 — submit it (curl)

The server creates the directory and seeds **placeholder** SCOPE.md
and WORKFLOW.md from a preset. You overwrite both with your real
content via `write_job_file` after submit returns.

```sh
# 0. find the repo id
REPO_ID=$(curl -s -X POST http://127.0.0.1:7777/rpc/list_repos \
  -H 'content-type: application/json' -d '{}' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["repos"][0]["id"])')
echo "REPO_ID=$REPO_ID"

# 1. write the template to a tmp file (or pipe; either works)
cat >/tmp/job-template.yaml <<'YAML'
name: my-job
goal: |
  One paragraph.
stages:
  - first stage outcome
  - REVIEW before the destructive step
  - second stage outcome
YAML

# 2. build the submit payload
python3 <<PY >/tmp/submit.json
import json
print(json.dumps({
  "repo_id":          "$REPO_ID",
  "prompt":           None,
  "template_yaml":    open("/tmp/job-template.yaml").read(),
  "runner":           "claude",
  "branch":           "codeless/my-job",
  "workspace_mode":   "worktree",
  "cost_cap_cents":   3000,
  "wall_clock_cap_ms": 1800000,
  "start_immediately": False
}))
PY

# 3. submit — status comes back as "draft" (because start_immediately=false)
JOB_ID=$(curl -s -X POST http://127.0.0.1:7777/rpc/submit_job \
  -H 'content-type: application/json' --data @/tmp/submit.json \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
echo "JOB_ID=$JOB_ID"

# 4. overlay your real SCOPE.md and WORKFLOW.md
for f in SCOPE.md WORKFLOW.md; do
  python3 -c "import json;print(json.dumps({
    'job_id':'$JOB_ID','filename':'$f','content':open('/tmp/job-'+'$f').read()
  }))" > /tmp/wf.json
  curl -s -X POST http://127.0.0.1:7777/rpc/write_job_file \
    -H 'content-type: application/json' --data @/tmp/wf.json
done

# 5. verify
curl -s -X POST http://127.0.0.1:7777/rpc/get_job \
  -H 'content-type: application/json' -d "{\"job_id\":\"$JOB_ID\"}" \
  | python3 -m json.tool | grep -E 'status|branch|workspace_mode|cost_cap'
```

Expected:

```
"status":         "draft"
"branch":         "codeless/my-job"
"workspace_mode": "worktree"
"cost_cap_cents": 3000
```

## Step 5 — start the job

Either click **start** in the UI, or:

```sh
curl -s -X POST http://127.0.0.1:7777/rpc/start_job \
  -H 'content-type: application/json' \
  -d "{\"job_id\":\"$JOB_ID\"}"
```

The runner allocates a worktree at
`~/.codeless/worktrees/job-<JOB_ID>/` on a fresh `codeless/my-job`
branch (because `workspace_mode: worktree`), reads the docs in the
order `JOB-DIR.md` defines, and works the stages.

## SubmitJobArgs cheat-sheet

| Field                | Common values                          | Notes |
|----------------------|----------------------------------------|-------|
| `runner`             | `claude` / `anthropic` / `mock`        | server must have `--enable-<runner>` for non-mock |
| `branch`             | `codeless/<job-name>`                  | only used in `worktree` mode |
| `workspace_mode`     | `worktree` (recommended) / `in-repo`   | **prefer `worktree`** — `in-repo` commits straight onto the source tree |
| `cost_cap_cents`     | `3000` ($30) typical                   | hard cap; runner stops when hit |
| `wall_clock_cap_ms`  | `1800000` (30 min) typical             | hard cap; runner stops when hit |
| `start_immediately`  | `false` (this doc) / `true`            | `false` = `draft`; you start later |

Full schema lives in
[`crates/codeless-rpc/src/methods.rs`](../crates/codeless-rpc/src/methods.rs)
under `SubmitJobArgs`.

## Hard rules — do not violate

1. **Never use `workspace_mode: in-repo` for real work.** It commits
   onto the current branch of your source tree and fights with
   normal development. Use `worktree`.
2. **Never start a job whose scope you haven't read.** A REVIEW gate
   does not save you from a bad scope.
3. **Never bypass the placeholder overlay.** If you submit and skip
   the `write_job_file` step, the agent reads the preset
   placeholders, not your scope.
4. **Never delete `template.yaml` to "reset" a job.** Use
   `delete_job_file` (which refuses) or remove the whole directory
   with the server stopped.
5. **Every stage commits and pushes its own work, on its own
   branch.** No batching across stages, no "I'll commit at the
   end". The recovery story for a crashed worktree is `git fetch`
   plus the per-stage commits — without them, the work is lost.
   `WORKFLOW.md` must include the per-stage commit-and-push block
   from Step 3 above so the agent re-reads the rule at every
   stage. Never `--force`, never `--no-verify`; if a hook fails,
   fix the cause.

## Common gotchas

- **`409 a job named X already exists`** — directory already there.
  Either pick a different `name:` or remove
  `<repo>/.codeless/jobs/<name>/` and resubmit. The server seeds the
  dir, so it must not pre-exist.
- **`422 unknown variant 'in_repo'`** — the wire form is hyphenated:
  `"workspace_mode": "in-repo"` (use `worktree` anyway).
- **Job stays in `draft` forever** — that's the whole point of
  `start_immediately: false`. Hit start in the UI or call `start_job`.
- **Server says repo doesn't exist after wipe** — DB is the source of
  truth; if you `reset` `~/.codeless/`, the on-disk job dir survives
  but you must re-`add-repo` and re-submit. Use the recovery flow in
  [`GETTING-STARTED.md`](./GETTING-STARTED.md).
- **Editor can't read scope file** — `--fs-root` must include the
  repo. Set `CODELESS_FS_ROOT` and restart via
  `./setup/init-session.sh start --bg`.

## Worked example — the assistant job

This very repo has one. See
[`.codeless/jobs/assistant/`](../.codeless/jobs/assistant/):

- [`template.yaml`](../.codeless/jobs/assistant/template.yaml) — 9
  stages, 2 REVIEW gates.
- [`SCOPE.md`](../.codeless/jobs/assistant/SCOPE.md) — references
  [`DOCS/ASSISTANT-SCOPE.md`](../../DOCS/ASSISTANT-SCOPE.md) for the
  full design; the per-job scope is the trimmed brief.
- [`WORKFLOW.md`](../.codeless/jobs/assistant/WORKFLOW.md) —
  per-stage discipline, REVIEW gate behaviour, anti-patterns.

The pattern: the **deep design lives in `DOCS/`**, the **per-job
brief in `.codeless/jobs/<name>/SCOPE.md`** points at it. That keeps
the per-stage prompt short while preserving the full context one
link away.
