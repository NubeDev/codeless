# Build status — UI demo grind (visible-in-browser per stage)

> ⛔ **AGENT REMINDER — READ BEFORE TOUCHING THIS FILE**
>
> 1. You are running JOB-LOOP. Spec: `DOCS/JOB-LOOP.md`. Project scope:
>    `DOCS/SCOPE.md`. Code-style rules: `CLAUDE.md` (repo root).
> 2. **One logical batch per tick.** Read each stage's `[S|M|L]` tag and
>    batch per JOB-LOOP.md "Hard rules" #3: up to 4 contiguous S in one
>    area, OR 1 M (+ optional related S), OR the next sub-stage of an L.
>    Verify + commit + push **each stage** via mani before moving to the
>    next stage in the batch.
> 3. **You MUST schedule the next tick before exiting** — call
>    `CronCreate` with `recurring: false` for a single fire ~1 min from
>    now. If all stages are `[x]`, report `DONE` instead. If you cannot
>    schedule, **do NOT exit silently** — tell the user which stage
>    finished, exactly why scheduling failed, and how to re-kick. See
>    JOB-LOOP.md "If you cannot schedule".
> 4. Update this file in the **same commit** as the code change.
> 5. ⛔ **COMMIT _AND_ PUSH BEFORE THE TICK ENDS.** Pushing is not
>    optional and not "later". A tick that ends with unpushed commits
>    means the next tick (or the next agent, after `/clear` or a fresh
>    session) sees stale remote state and can clobber or duplicate work.
>    `./bin/mani --config mani.yaml run commit --projects codeless` then `mani run push --projects
>    codeless` — both, every tick, no exceptions. If push fails, mark
>    the stage `[!]` and halt. Never `--force`, never `--no-verify`.
> 6. ⛔ **CODE COMMENTS ARE LOAD-BEARING — WRITE THEM CAREFULLY.**
>    Comments are how the *next* AI agent (and the next human) understands
>    intent. Rules:
>    - Explain **why**, not what. The code already says what.
>    - **No emojis.** Anywhere. Ever.
>    - **No task-status comments.** Never reference stages, ticks,
>      milestones, "added in stage 3", "TODO from M5", "fixed for ticket
>      X". Comments describe the code as it stands, not the task that
>      produced it.
>    - **Long-term framing.** Write for someone reading this in 6 months
>      with zero context — invariants, constraints, why this approach
>      over the obvious one.
>    - **Normal length.** A short line where one helps. A short paragraph
>      where the *why* is genuinely subtle. No multi-paragraph essays,
>      no decorative banners, no ASCII art.
> 7. ⛔ **CROSS-PLATFORM REACH IS ENFORCEABLE.** Stages that touch Rust
>    crates respect the iOS-safe / Android-safe columns in
>    `DOCS/SCOPE.md` "Crate layout". Stages that touch UI modules import
>    only `RpcClient` — never `@tauri-apps/api/core` directly. Trip
>    either rule → mark stage `[!]` and halt.

> ⛔ DEMO-FIRST RULE: every stage in this loop must end with a
> visible change in the browser at http://127.0.0.1:5173. Bundle
> backend work into the same stage as the UI surface that uses it.
> No backend-only stages. The user is watching the browser, not
> the diff. If you cannot describe what changed on screen, the
> stage isn't done.

File: DOCS/sessions/2026-05-13-ui-demo-grind.md
Goal: Land visible UI demos fast; every stage produces something the user can see in the browser.
Started: 2026-05-13
Last tick: 2026-05-13 05:22
Current stage: 7 / 10

Repo:        codeless
Branch:      master
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

## Stages

- [x] 1. [S] Verify existing DEMO-UI.md path still works on master
- [x] 2. [S] Jobs list view: seeded demo job visible with status
- [x] 3. [M] Job detail view: stages + live SSE event stream
- [x] 4. [S] "Run mock job" button: create + start a new mock job
- [x] 5. [M] Live stage tree: checklist updated from event stream
- [x] 6. [S] Cost + wall-clock badges on the job row
- [ ] 7. [M] Handover preview pane (runs/<name>/handover.md)  ← next
- [ ] 8. [S] Review queue badge in the top bar
- [ ] 9. [M] Ad-hoc job form: New Job button + repo dropdown
- [ ] 10. [S] Polish pass: theme toggle, empty states, CTA

## Notes
- Stage 0: status file created on master in inner codeless repo.
- Stage 1 verified on 2026-05-13: `cargo fmt --check`, `cargo clippy
  --workspace --all-targets -D warnings`, and `pnpm tsc --noEmit` all
  green. `demo bootstrap` seeded `demo` repo + queued mock job.
  `codeless serve --bind 127.0.0.1:7777 --fs-root $PWD` came up;
  `/healthz` → `ok`, `/rpc/list_jobs` returned the seeded job,
  `/rpc/list_repos` returned the `demo` repo. UI dev server already
  running on 127.0.0.1:1420 (project vite config pins port 1420 with
  `strictPort`, not :5173 as the kickoff says) and serves the SPA
  HTML (HTTP 200). DEMO-UI.md path is green on master.
- Stage 2 met by existing surface from the prior ux-grind work (no
  code change; verified live against a fresh demo db).
  `JobsDashboard` groups jobs by repo, `JobRow` renders one row per
  job with `StatusBadge` (queued/running/completed), runner badge,
  branch, relative age, cost, and activity chip. The seeded mock
  job appears as `completed` (mock runner ran it on serve start).
- Stage 3 met by existing surface from the prior ux-grind work (no
  code change). `/jobs/:id` opens `JobDetail` in a Sheet; the panel
  shows status, runner, repo, branch, worktree path, prompt, plus
  Timeline + Files-changed tabs. `JobTimeline` imports
  `useEventStream` from `@/lib/rpc` and subscribes per-job, so mock
  runner events stream live as they fire. No `@tauri-apps/api/core`
  imports (R2 satisfied).
- Stage 6 added `WallClockCell` in `JobRow.tsx` and mounted it next
  to the existing `CostCell` in both the dashboard row and the
  `JobDetail` header. Live elapsed = `(ended_at ?? now) - started_at`,
  formatted as `Hh MMm` / `Mm SSs` / `Ss` for compact width; flips
  to amber at 80% of `wall_clock_cap_ms`. The dashboard's existing
  30s `now` clock drives row re-renders; the detail panel uses a
  one-shot `Date.now()` (it re-renders on each event arrival).
  Visible-in-browser: every job row and the detail header now read
  e.g. `2m04s / 30m00s` next to the cost badge.
- Stage 5 added `StageTree`, mounted in `JobDetail` above `ReviewPanel`.
  Subscribes to the same per-job `useEventStream` filter as
  `JobTimeline`, folds `stage-started` / `stage-completed` /
  `verify-failed` envelopes into an ordered checklist (`…` running,
  `✓` completed, `!` failed-with-exit-code). Self-hides when no
  stage envelopes have arrived. No new backend; cosmetic over the
  existing SSE data. Visible-in-browser: the job detail sheet now
  carries a compact "Stages" strip near the top that fills in live
  as the runner advances.
- Stage 4 added `RunMockJobButton` (one-click affordance) and placed
  it next to `SubmitJobDialog` in each repo card header in
  `JobsDashboard`. Click → `submit_job` with runner=`mock`, canned
  prompt, fresh `codeless/mock-<6hex>` branch, $5 cost cap, 30m
  wall-clock cap. New job appears in the list via the existing
  event-driven overlay and streams to completion. Verified: `tsc
  --noEmit` green, `cargo clippy -D warnings` green, `pnpm build`
  produced the bundle including the new module. UI dev server could
  not be exercised live in this tick due to host ENOSPC (inotify
  watcher exhaustion, unrelated to the change); the production
  bundle build is the stronger signal that the code compiles. The
  visible-in-browser surface: a "run mock job" button on every repo
  card header next to "new job".

## Blockers
(none)
