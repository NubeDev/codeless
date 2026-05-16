# DOGFOOD-MVP — "codeless develops codeless, visibly, from the browser"

> **Sister docs.** [`SCOPE.md`](./SCOPE.md) is the product scope.
> [`JOB-MODEL.md`](./JOB-MODEL.md) is the user-facing framework
> (we are the users now). [`LOOP-CODER.md`](./LOOP-CODER.md) is the
> long-run design intent. [`DEMO-UI.md`](../DEMO-UI.md) is the
> existing two-terminals-one-browser demo path that the MVP is
> built on top of.

## The goal, in one sentence

**A codeless contributor opens the browser, clicks "New job", types
a goal in plain English, and watches codeless make that change to
its own source — stages landing as visible commits, live events
streaming into the UI, a worktree they can inspect afterwards.**

That's the bar. Not "all features". Not "polished". One golden
path that closes the loop end-to-end. Everything else is later.

## Why this goal, and why now

Codeless's whole product thesis is "long unsupervised coding runs".
The honest test of that thesis is: **can codeless drive a job
against the codeless repo itself, with the codeless contributor
watching from the browser, and have something useful land at the
end?** If yes, dogfooding is real and every later feature has a
direct feedback loop. If no, the framework is still theory.

Right now (as of 2026-05-13) the parts exist — crates, runners,
runtime, UI dashboard, file explorer, worktree manager, event
stream — but they have never been wired end-to-end into the
specific path above. The MVP is **wiring**, not building.

## The end-to-end path the MVP must support

A user with codeless-server running and the UI open at
`http://127.0.0.1:5173` can do this without leaving the browser:

1. **See the codeless repo** in the sidebar (the repo is the
   codeless workspace itself, registered via `codeless repo add`
   or seeded by a bootstrap command).
2. **Click "New job"** in the jobs dashboard. The existing
   `NewJobDialog.tsx` opens.
3. **Type a short goal** in plain English. Example: "add a
   `/healthz` route to `codeless-server` that returns `200 OK`."
4. **See proposed stages.** For the MVP, the planner is a
   hard-coded fallback that produces 2-3 stages from the goal
   string. A real Rig-backed planner lands later — see
   [`SCOPE.md`](./SCOPE.md) "Where Rig is genuinely useful
   long-term".
5. **Click Queue.** The job appears in the jobs list as `queued`.
6. **Watch it promote to `running`.** The job-queue scheduler
   picks it up under the concurrency cap; the SSE event stream
   delivers `job-promoted` and `job-started`.
7. **A worktree is cut** at
   `$XDG_DATA_HOME/codeless/worktrees/<job-name>-<id>/` on a
   branch named `codeless/<job-name>-<id>`. (Per JOB-MODEL.md.)
8. **The Claude Code CLI is invoked** against that worktree by
   `ClaudeRunnerAdapter`. Events stream into the UI:
   `text` deltas as the agent reasons, `tool-use` entries for
   each `Edit` / `Write` / `Bash` the agent runs.
9. **Stage 1 finishes.** Verify runs (the repo's configured
   verify command — `cargo test && cargo clippy && cargo fmt`).
   The stage commits. `runs/<job-name>/handover.md` and
   `runs/<job-name>/log.md` are written and committed in a
   second `handover N` commit. Push, if a remote is configured.
   UI shows stage 1 ✓.
10. **Subsequent stages run** the same way. The user can watch the
    live stream or close the tab and come back later — the events
    are persisted, the UI replays them on reconnect.
11. **All stages done.** The UI shows the final stage list, the
    handover preview, the branch name, and a button to open the
    worktree in the user's editor (or a `gh pr create` affordance
    if `gh` is configured).
12. **The user inspects the diff** — in the codeless file
    explorer, or by opening the worktree in VS Code / Cursor /
    their editor of choice — and decides whether to merge.

That is the entire MVP. Twelve numbered steps. If any of them
doesn't work end-to-end against the codeless repo on master, the
MVP isn't done.

## What's already on the ground (so we don't rebuild it)

Verified by inspection of the repo at 2026-05-13:

- **Crate split is real.** `codeless-types`, `codeless-rpc`,
  `codeless-runtime`, `codeless-adapters-host`, `codeless-server`,
  `codeless-cli`, `codeless-client`, `codeless-tauri-desktop`
  all exist.
- **Runners are wired.** `ClaudeRunnerAdapter`,
  `AnthropicRunnerAdapter`, `MockRunner` all live in
  `codeless-runtime`.
- **`drive_job` exists** and has end-to-end tests against the
  fake-claude-binary harness (`tests/claude_runner.rs`).
- **Handover persistence has a module** (`handover.rs` in
  `codeless-runtime`). Whether it writes the file format
  JOB-MODEL.md specifies is an audit item, not a build item.
- **Reviews, gc-worktrees, rerun-job, event-persistence, since-replay,
  cost-rollup, cap-cancellation, queue-caps, heartbeat, resumability,
  notifier** — all have integration tests in
  `codeless-runtime/tests/`. The runtime is more complete than the
  UI surface around it.
- **UI dashboard exists** — `JobsDashboard.tsx`, `JobDetail.tsx`,
  `NewJobDialog.tsx`, `SubmitJobDialog.tsx` are all there.
- **The demo path works** — `DEMO-UI.md`'s two-terminals-one-browser
  flow boots codeless-server, seeds a demo repo + mock job, and
  shows it in the UI.

The MVP is about **closing the gap between the demo path (mock
runner, separate demo repo) and the dogfood path (Claude Code CLI,
codeless repo as the target)**. Most of that gap is wiring,
configuration, and a small number of UI affordances.

## What's deliberately out of scope for the MVP

Anything that doesn't sit on the golden path above. Specifically:

- **Multi-job concurrency UX.** The scheduler supports it; the UI
  for showing N jobs running at once is later.
- **Rig-backed planner.** Hard-coded fallback for MVP. A real
  planner lands in a follow-up loop.
- **Rig-backed reviewer / summariser / RAG.** Same.
- **Long unsupervised runs.** The session scheduler (per
  LOOP-CODER.md "What blocks this today") is **not** in the MVP.
  MVP is "one session, one job, supervised from the browser." The
  multi-session 8-hour-run loop lands once the single-session
  story is solid.
- **REVIEW gate UX.** Single-session jobs with no REVIEW stages
  for the MVP. The DB row exists; surfacing it nicely is later.
- **Loop-level aggregate budget.** Per-job caps only for MVP.
- **Tauri desktop / mobile shells.** Browser-only.
- **Polish, theming, empty states, error toasts.** Add after the
  loop works.
- **MCP surface.** Already covered by `codeless-mcp` stub; not
  exercised in MVP.
- **PR auto-creation.** Print the `gh pr create` command for the
  user to run; don't try to authenticate on their behalf.

If a stage proposal touches anything in this list, push it to a
follow-up loop. Discipline here is what keeps the MVP shippable.

## The MVP gap list — what actually needs wiring

These are the only stages the MVP should produce. Each is sized
deliberately small so progress is visible after every tick.

1. **Baseline audit.** Boot codeless-server pointing at the
   codeless repo (`--fs-root` = the workspace root, the repo
   registered against the codeless source). Run the existing demo
   bootstrap. Open the UI. Document, in the status file, exactly
   what works and what doesn't against this target. This is the
   only stage allowed to be exploration.

2. **Register the codeless repo as a managed target.** Either
   extend `codeless demo bootstrap` with a flag (`--target-self`)
   or add a one-line `codeless repo add` invocation in DEMO-UI.md.
   End state: the codeless repo shows up in the UI sidebar after
   `codeless demo bootstrap --target-self`.

3. **Hard-coded planner fallback.** Given a goal string, produce
   2-3 stages without an LLM. Plain Rust function in
   `codeless-runtime` (or a thin wrapper). Pattern-matches a few
   shapes ("add X endpoint", "rename Y to Z", "fix N in module
   M") and otherwise returns a single stage that's literally the
   goal string. Good enough for MVP — Rig planner replaces it
   later.

4. **NewJobDialog → real job creation.** The dialog exists; wire
   it to `job.create` over RPC with the planner-generated stages,
   `claude-code-cli` runner, default caps from `config.yaml`.
   Submitting the form must result in a new row in the jobs list.

5. **Live event stream renders into JobDetail.** The SSE
   subscription is there; the rendering for `text`, `tool-use`,
   `stage-started`, `verify-passed`, `verify-failed`,
   `stage-completed` needs to actually show in the panel. Reuse
   whatever's already in `JobDetail.tsx`; fill the gaps.

6. **Worktree + branch verification.** Confirm `drive_job` cuts
   `codeless/<job-name>-<id>` against the registered codeless
   repo and that the worktree appears under
   `$XDG_DATA_HOME/codeless/worktrees/`. If branch naming
   doesn't match SCOPE.md "Workspace = one git worktree per job",
   fix it here.

7. **Handover write verification.** Run a job end-to-end with
   `MockRunner` first. Confirm `runs/<name>/handover.md` and
   `runs/<name>/log.md` are created in the worktree and committed
   per JOB-MODEL.md. If `handover.rs` doesn't yet write the
   five-section format, this stage makes it do so.

8. **Claude runner end-to-end on codeless.** Replace the
   `MockRunner` with `ClaudeRunnerAdapter`, kick off a job with a
   trivial goal ("add a one-line doc comment to
   `codeless-server/src/main.rs`"), and watch it succeed from the
   browser. **This is the moment the MVP is real.**

9. **"Open worktree" affordance.** A button or link in
   `JobDetail.tsx` that, on a completed job, surfaces the
   worktree path so the user can `cd` into it in their editor.
   Also print the `gh pr create --draft --base master --head
   codeless/<job-name>-<id>` command in the UI so PR creation is
   one copy-paste away.

10. **Document the MVP path in DEMO-UI.md.** Add a new section
    "Dogfood: codeless develops codeless" with the exact
    commands and the click sequence. The doc is part of the MVP
    — without it, the next contributor can't reproduce.

Ten stages, all on the golden path. None of them adds polish or
breadth.

## When the MVP is done

You should be able to record a 90-second screencast that shows
steps 1–12 from "The end-to-end path" above, against the codeless
repo, with no manual intervention beyond clicking buttons and
typing the goal. If the screencast needs a cut, the MVP isn't done.

## After the MVP

Once the single-session dogfood loop works, the next loops are
obvious — each picks one item from "Out of scope" above and
adds it. In rough priority order:

1. **Session scheduler** so multi-session long runs are real
   (LOOP-CODER.md "What blocks this today" #1).
2. **REVIEW gate UX** so the user can resolve pending reviews
   from the UI.
3. **Loop-level budget ceilings.**
4. **Rig planner** replacing the hard-coded fallback.
5. **Rig commit-message summariser** so the auto-generated
   commit messages improve.
6. **Job-memory RAG** over `runs/*/log.md` so a new job knows
   what prior jobs tried.

But MVP first. Each of those is its own kickoff later.
