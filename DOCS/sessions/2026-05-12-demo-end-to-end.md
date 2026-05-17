# Build status — full working browser demo against codeless-server

> ⛔ **AGENT REMINDER — READ BEFORE TOUCHING THIS FILE**
>
> 1. You are running JOB-LOOP. Spec: `DOCS/JOB-LOOP.md`. Project scope:
>    `DOCS/SCOPE.md`. Code-style rules: `CLAUDE.md` (repo root).
> 2. **One logical batch per tick.** Read each stage's `[S|M|L]` tag and
>    batch per JOB-LOOP.md "Hard rules" #3.
> 3. **You MUST schedule the next tick before exiting** — call
>    `CronCreate` with `recurring: false` for a single fire ~1 min from
>    now. If all stages are `[x]`, report `DONE`. If you cannot schedule,
>    do NOT exit silently — follow JOB-LOOP.md "If you cannot schedule".
> 4. Update this file in the **same commit** as the code change.
> 5. ⛔ **COMMIT _AND_ PUSH BEFORE THE TICK ENDS.** Never `--force`,
>    never `--no-verify`.
> 6. ⛔ Comments: explain **why**, not what. No emojis, no
>    task-status comments, no decorative banners.
> 7. ⛔ Cross-platform reach is enforceable. UI imports `RpcClient`
>    only; process-spawn lives only in `codeless-adapters-host`.

File: DOCS/sessions/2026-05-12-demo-end-to-end.md
Goal: A user with a fresh checkout can run two commands
      (`codeless serve --fs-root <repo>` + `pnpm dev`) and reach a
      browser UI that lists repos, browses files from the real
      filesystem, opens a file in the editor, submits a mock job,
      and watches it complete via SSE — all against the real
      `codeless-server`, no mocks.
Started: 2026-05-12
Last tick: 2026-05-12 18:50
Current stage: 8 / 8 — DONE

Repo:        codeless
Branch:      master
Memory policy: each tick fresh; status file is source of truth
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

## Stages
Format: `[ ] N. [S|M|L] title` — complexity tag is mandatory.

- [x] 1. [S] `fs_cwd` lives in `codeless-rpc::methods::FsCwdResult`,
       trait method `fs_cwd(&self) -> RpcResult<FsCwdResult>`. Runtime
       reads `HostFs::root()`; without `with_fs` it returns the same
       `Internal("fs.* not available")` as the other fs methods.
       Server exposes `/rpc/fs_cwd`; http client wraps it. Both
       snapshots regenerated.

- [x] 2. [S] UI fallback to `fs_cwd` was already wired in `App.tsx`
       from a prior pass — `usePaths().homeDir()` first; if null,
       `rpc.call("fs_cwd", {})`. Stage 1 made the Rust side actually
       answer the call. UI tsc clean. Mock client already implements
       `fs_cwd`. Effective result: opening the browser against a
       real server now shows the workspace contents instead of "no
       current directory".

- [x] 3. [S] `codeless demo bootstrap --db <path>` seeds one repo
       named "demo" (idempotent — name-collision short-circuits)
       and submits one queued mock job. `--local-path` defaults to
       cwd. Refuses to run without `--db` so the seed cannot vanish
       between bootstrap and `codeless serve`. Smoke-tested both
       paths (fresh seed + skip-on-rerun).

- [x] 4. [S] `NoReposCta` component rendered in JobsDashboard when
       `repos.data.length === 0`. Includes the exact bootstrap
       command line so a user landing on an empty dashboard knows
       what to type. Doesn't replace the header (still shows 0/0
       counters) so the relationship between "no repos" and "no
       jobs" stays visible.

- [x] 5. [M] End-to-end mock-runner verified: bootstrap a tempdb,
       boot `codeless serve` on an ephemeral port, wait, list_jobs
       returns `status: "completed"` and `cost_cents: 0`. To make
       the job visibly alive on the dashboard (instead of going
       Queued -> Completed with no body), the `DefaultRunnerFactory`
       mock case now scripts a short `TaskStarted` -> N x `AiToken`
       (word-sized chunks of the prompt) -> `AiMessageComplete` ->
       `TaskCompleted` -> `Finish(Completed)` sequence with 120 ms
       sleeps between tokens. `FAIL` sentinel still goes straight to
       `Failed`. Real AI runners drive the same event variants, so
       the timeline renders identically for both.

- [x] 6. [S] `DEMO-UI.md` rewritten against the new flow:
       `demo bootstrap`, `--fs-root`, expected timeline events, and
       a troubleshooting block that names the failure modes the
       demo path actually exhibits.

- [x] 7. [S] `scripts/smoke-demo.sh` boots the server on
       `127.0.0.1:7799`, seeds via bootstrap, polls `/rpc/list_jobs`
       until terminal, asserts the demo repo exists and `fs_cwd`
       returns a path. Honours `CODELESS_BIN` to skip the build for
       quick reruns. Smoke-tested against the just-built binary —
       PASS. Cleans up the tempdir + server PID via `trap`.

- [x] 8. [S] Cleanup pass: clippy + fmt + wire-ts-check all clean.
       `codeless/README.md` quickstart updated to mention `demo
       bootstrap` and `--fs-root` (it was silently behind the new
       flow). Final scripts/smoke-demo.sh run: PASS.

## Notes
- The `fs_cwd` method also unlocks the explorer's path bar showing the
  workspace root instead of "/" — small UX win that costs nothing now
  that the method is there.
- Stages 11+12 of the prior session already left the editor + explorer
  on `useRpc()`; this loop only adds the missing demo connective
  tissue.
- The followup-fs methods (`fs_create_*`, `fs_move`, `fs_delete`,
  `fs_search`, `fs_glob`) deliberately stay out of scope — the demo
  goal is read-only browsing + edit + save, plus the jobs surface.

## Blockers
(none)

## Tick log
- Tick 7 (2026-05-12 18:50): stage 8. Cleanup landed:
  `codeless/README.md` now references `demo bootstrap` + `--fs-root`.
  Final smoke-demo.sh PASS. Loop DONE.
- Tick 6 (2026-05-12 17:50): stages 6 + 7. DEMO-UI.md replaced with
  the post-bootstrap flow; `scripts/smoke-demo.sh` codifies the
  end-to-end happy path and exits 0 on success. Both verified.
- Tick 5 (2026-05-12 17:47): stage 5. Mock runner now scripts a
  visible token stream so the JobsDashboard timeline has content
  during a demo run. Smoke-tested with a tempdb, ephemeral port,
  and curl against /rpc/list_jobs at t=0 and t=4s — second call
  returns status:completed. Stage 7 captures this as a script.
- Tick 4 (2026-05-12 17:44): stage 4. Empty-state CTA in
  JobsDashboard explicitly points at `codeless demo bootstrap` so a
  user landing on a fresh database has a one-command path forward
  without having to read README.
- Tick 3 (2026-05-12 17:42): stage 3. New `Cmd::Demo` verb;
  bootstrap path verified end-to-end against a tempfile db. Mock
  runner kind chosen so the seeded job can complete without
  external dependencies once the server's background driver picks
  it up.
- Tick 2 (2026-05-12 17:38): stage 2. No code change needed — the
  UI fallback already existed. Stage 1's Rust impl makes it actually
  resolve. The explorer now shows the workspace root on first paint
  against a real server.
- Tick 1 (2026-05-12 17:36): stage 1. `fs_cwd` end-to-end (types,
  trait, runtime, server, client, snapshots).
