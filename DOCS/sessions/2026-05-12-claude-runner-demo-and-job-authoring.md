# Build status — Claude Code runner demo + rich job authoring

> ⛔ **AGENT REMINDER — READ BEFORE TOUCHING THIS FILE**
>
> 1. JOB-LOOP spec: `DOCS/JOB-LOOP.md`. Project scope: `DOCS/SCOPE.md`.
>    Code-style rules: `CLAUDE.md`.
> 2. One logical batch per tick; verify + commit + push per stage.
> 3. ⛔ Schedule the next tick before exiting; report DONE if all `[x]`.
> 4. ⛔ Commit AND push every stage via mani; never `--force`, never `--no-verify`.
> 5. ⛔ Comments: why, not what. No emojis, no task-status, no banners.
> 6. ⛔ Cross-platform reach: UI imports `RpcClient` only.

File: DOCS/sessions/2026-05-12-claude-runner-demo-and-job-authoring.md

Goal in one sentence:
  Make Codeless eat its own dog food. The user opens the UI, types a
  one-line prompt, picks `claude` from a dropdown, watches Claude
  Code do real work in a `git worktree`, reviews the result, and
  iterates. Then layer a richer job-authoring wizard on top so a job
  is not "two text inputs" but a real planning artefact the user
  can refine (and optionally have an AI scope) before submitting.

Started: 2026-05-12
Last tick: 2026-05-12 (stage 7 landed; stage 6 still [!])
Current stage: 8 / 13 ← Phase 1 closed pending the stage 6 unblock

Repo:        codeless
Branch:      master
Scheduler:   CronCreate one-shot, ~1 min between ticks
Max ticks:   30

## Architecture context (no rewrite, no Go service)

The existing Rust runtime + UI are the architecture. The Claude Code
runner already exists in two places:

- `ai-runner/src/runners/claude.rs` — wraps the `claude` binary
  (`claude auth login` handles auth, no API key in our code path).
- `crates/codeless-runtime/src/claude_runner.rs::ClaudeRunnerAdapter`
  — bridges `ai-runner` events to `EventBus`. Discoverable via
  CLI flag `--enable-claude` on `codeless serve` today.

What is **not** in place:
- A UI path that picks the runner from a dropdown sourced from the
  server's enabled list (it's a free-text input).
- A way to know from the UI whether `claude` is installed and
  authenticated on the host.
- Worktree provisioning wired in the demo flow (`--worktree-root`
  is a CLI flag, not exposed in the UI; the demo currently skips
  worktrees, so a real runner has nothing to edit).
- A job-authoring surface richer than the current two-field form.

The two phases below address those in order.

## Phase 1 — Claude Code runner demo, one-prompt path

End state: user opens the UI on a workspace that has Claude Code
installed and authenticated. They click "submit job", type a one-line
prompt, pick `claude` from a dropdown that's only present because
the server reports claude as enabled. The job picks up a freshly
provisioned `git worktree` for that repo, the runner streams events
into the timeline (real Claude responses, real tool calls), and the
job either runs to completion or hits the review gate.

- [x] 1. [S] Server: `GET /server/info` (no auth) returns
       `{ runners: [...], fs_root: Option<String>, version: String,
       worktree_root: Option<String> }`. Wired in `codeless-server`
       routes, with the data sourced from a new `ServerInfo` field
       on `AppState`. The runner list reflects what the CLI passed
       to `DefaultRunnerFactory` (mock always; claude if
       `--enable-claude`; anthropic if `--enable-anthropic`). Used
       by the UI to populate the runner dropdown and to detect
       "demo mode" vs "real runners available".

- [x] 2. [M] `codeless-adapters-host::claude` (new module): a host
       helper that locates the `claude` binary using the same
       discovery as `ai-runner::runners::claude::discover_claude_binary`
       (env → PATH → known install paths) and probes `claude --version`.
       Exposes `ClaudeStatus { binary_path, version, authenticated }`.
       The `authenticated` probe is best-effort: run
       `claude /status --output-format json` with a 2s timeout and
       parse the response if the wrapper provides it; otherwise
       fall back to "binary exists, auth unknown". Result lands on
       `/server/info` so the UI can render the right hint
       ("Install Claude Code", "Run `claude auth login`", "Ready").

- [x] 3. [S] CLI: `codeless serve --enable-claude --worktree-root
       <path>` already exists. Add a `--worktree-root` default that
       points at `<fs-root>/.codeless/worktrees` when both are set,
       so the demo path does not require the operator to invent a
       second flag. Document the implicit default in
       `codeless serve --help`. Worktree dirs are gitignored at the
       repo level (`.codeless/` is in our `.gitignore` already? if
       not, add it as part of this stage).

- [x] 4. [S] UI: Submit dialog reads `/server/info` once, populates
       the Runner field as a `<Select>` with the enabled runners.
       Default selection: server's reported default. `mock` shown
       with a "(demo)" label so the user does not confuse it with
       a real runner. The branch field stays free-text, defaulting
       to `codeless/job-<short-id>` so each run gets a fresh branch
       — the demo's `main` default was wrong (real runs commit on
       the branch and we want isolation).

- [x] 5. [S] UI: settings → Models section grows a "Coding agents"
       block above the API keys. Lists Claude Code with the status
       reported by `/server/info` and a one-line action ("Install
       Claude Code" linking to the docs; "Run `claude auth login`
       in a terminal"; "Ready"). Future Codex / Copilot rows live
       in the same block when those runners arrive.

- [!] 6. [M] End-to-end exercise: from a fresh git repo on the              ← BLOCKED
       host (`cd /tmp && git init demo-target && cd demo-target &&
       echo "# demo" > README.md && git add -A && git commit -m
       init`), run:
       ```
       codeless --db /tmp/codeless-demo.db demo bootstrap \
         --local-path /tmp/demo-target
       codeless --db /tmp/codeless-demo.db serve \
         --fs-root /tmp/demo-target --enable-claude
       ```
       Then in the UI: submit a job with prompt "add a hello.txt
       file with the word 'hi'", runner = claude, branch =
       `codeless/job-<n>`. Expect: worktree provisioned, Claude
       runs, hello.txt appears in the worktree, job-completed.
       Capture rough notes for the demo doc and any rough edges
       hit. This is the dogfood proof.

- [x] 7. [S] DEMO-UI.md grows a "Real runner: Claude Code" section
       with the prereqs (`claude` installed, `claude auth login`
       run once), the additional `serve` flags, and the
       expected timeline shape (`tool-call` events appearing
       alongside `ai-token` deltas).

## Phase 2 — Rich job authoring (the "wizard")

The current submit dialog is two text inputs. Phase 2 turns it into
a real authoring surface that grows with the job. The user can:

- Start a draft (saved to the DB so closing the tab does not lose
  the WIP).
- Outline stages — each stage gets a name, optional `verify_cmd`,
  optional reviewer policy (`always pause` | `pause on diff
  >N lines` | `pause never`). Mirror the YAML template shape so the
  authoring surface and the on-disk template are isomorphic.
- Ask the configured runner to "scope this draft" — a meta-mode
  run that takes the prompt and returns a proposed stage list +
  per-stage prompts. Stored as a draft revision; user accepts,
  edits, or discards.
- Save the draft as a job template YAML in the repo (e.g.
  `.codeless/jobs/<slug>.yaml`) so the same job can be re-submitted
  later without re-typing.
- Submit the draft as a job. After completion, fork it into a new
  draft pre-populated with the previous outcome so iterating on the
  same task is one click.

- [ ] 8. [M] DB + types: `job_drafts` table (id, repo_id, title,            ← next
       prompt, stages_json, created_at, updated_at). New `Draft` /
       `DraftStage` types in `codeless-types`, RPC methods
       `list_drafts`, `get_draft`, `save_draft`, `delete_draft`,
       `submit_draft -> SubmitJobArgs`. Specta + wire-ts updates.

- [ ] 9. [M] UI: replace the current single-dialog submit flow with
       a multi-step wizard. Steps: Prompt → Stages → Review →
       Submit. Saves to `job_drafts` on every step. "Submit"
       calls `submit_draft` server-side, which converts the draft
       to `SubmitJobArgs` and submits as today.

- [ ] 10. [M] AI-assisted scoping: a "Scope with AI" button on the
        Prompt step invokes the configured runner in a one-shot
        meta-mode that returns a proposed stage outline. Server
        side: new `scope_draft` RPC that wraps the runner in a
        non-persistent run and returns the structured outline.
        Treat the returned outline as a draft revision, not a
        commitment — the user edits before moving on.

- [ ] 11. [S] Repo-local templates: a "Save as template" action on
        a draft writes `.codeless/jobs/<slug>.yaml` in the
        worktree (or main checkout, depending on what makes sense
        — TBD in stage 8 design). `codeless job submit
        .codeless/jobs/<slug>.yaml` is the existing CLI hook;
        UI gets a parallel "Load template" action.

- [ ] 12. [S] Re-run support: from a completed (or failed) job's
        detail panel, a "Fork as draft" button creates a new
        draft pre-populated with the previous job's prompt +
        stages + a note pointing at the parent job's id. User
        edits, resubmits.

- [ ] 13. [S] DEMO-UI.md + screencast notes: walk through the
        wizard flow end-to-end, then the fork/re-run flow.

## Open questions to answer during the loop

- Where does the worktree live for a "demo bootstrap" repo whose
  `local_path` is a real checkout? Today we point the runner at
  the bare `local_path`, which means a real Claude run would
  commit on top of the user's working branch — bad. Phase 1
  stage 3 needs to make worktrees the default for any non-`mock`
  runner. Sub-question: do we refuse to run a non-mock job
  without a worktree-root configured, or auto-provision one?
  Stage 3 answer: implicit default `<fs-root>/.codeless/worktrees`
  when --worktree-root is unset but --fs-root is set. The conservative
  "refuse non-mock without worktree-root" refusal lands when stage 6
  end-to-end exposes a footgun; today the implicit default makes the
  demo work without a second flag and the existing "worktrees=disabled"
  log line covers the no-fs-root case.
- Scope-with-AI in stage 10: does the meta-mode share a session
  with the eventual job run, or is it a separate transient run?
  Affects whether the draft's "context window" survives into the
  job. First version: separate transient run, no context sharing.
- Save-as-template path: workspace-root vs worktree? The template
  needs to be reachable from the next job's worktree, so the
  natural home is the repo's main checkout, not a per-job
  worktree. Confirm before stage 11 lands.

## Notes
- The mock runner is staying. Phase 1 makes it visually distinct
  ("(demo)") but does not remove it — it is the no-prereqs path
  for new contributors and the only reliable runner in CI.
- Phase 2 stages are individually shippable; stage 8 (DB + RPC)
  unblocks 9, 10, 11, 12 in any order.
- Anthropic REST runner is out of scope for both phases here — it
  needs API-key UX that the Models settings section already
  scaffolds. A follow-up loop wires it.

## Blockers

- **stage 6**: Claude Code runs in headless mode but the wrapper feeds
  it tool calls in interactive-permission mode. The smoke run shows
  three `tool-call` events (2x Write, 1x Bash) followed by an
  `ai-token` saying _"I need permission to create the file and run
  the git commands"_; no commit lands on the job branch. claude-wrapper
  exposes `QueryCommand::dangerously_skip_permissions()` and
  `PermissionMode::BypassPermissions`, which is what headless server
  use needs. Neither is wired through `ai-runner::CliCfg`, and
  CLAUDE.md treats `ai-runner/` as read-only (updates flow from
  rubix-agent upstream). Unblocks: either upstream a new
  `permission_mode` field on `CliCfg` and surface it on
  `ClaudeRunnerAdapter`, or take a workspace decision to vendor the
  flag locally. Until then, the dogfood proof cannot pass with a real
  claude binary. The smoke script
  `scripts/smoke-claude-demo.sh` survives as the regression net for
  every step *up to* the tool-permission gate.

## Tick log
- stage 7: DEMO-UI.md gains a "Real runner: Claude Code" section
  covering binary discovery + `claude auth login` prereqs, the
  `--enable-claude` flag, the implicit `<fs-root>/.codeless/worktrees`
  default, the expected `tool-call` + `ai-token` timeline shape, and
  the headless-permission gate captured in stage 6. Phase 1 is closed
  on documentation grounds; the dogfood proof is still blocked.
- stage 6: HALT [!]. Built `scripts/smoke-claude-demo.sh` that boots
  serve --enable-claude against a fresh /tmp/demo-target, asserts
  /server/info reports the implicit `<fs-root>/.codeless/worktrees`
  default and the claude probe, submits a job, polls get_job to
  terminal, then verifies the result against the job branch in the
  source repo (worktrees are reaped post-completion, so checking the
  worktree dir directly is wrong — the branch is the durable proof).
  Two rough edges surfaced:
    1. `WorktreeManager::create` always names the branch
       `codeless/job-<job_id>`, ignoring `SubmitJobArgs.branch`. The
       user-supplied field is effectively vestigial; the UI should
       either stop offering it or the manager should honour it.
       Tracking as a follow-up to stage 4.
    2. Headless claude needs `dangerously_skip_permissions` /
       `PermissionMode::BypassPermissions` from claude-wrapper. The
       smoke run captured three `tool-call` events (Write, Bash,
       Write) followed by claude asking _"I need permission to
       create the file..."_; no commit. ai-runner's CliCfg has no
       knob for this, and CLAUDE.md forbids editing ai-runner — see
       Blockers.
- stages 3+4+5: implicit --worktree-root default is
  <fs-root>/.codeless/worktrees when --fs-root is set; codeless repo
  gitignore now excludes `.codeless/worktrees/` (templates remain
  trackable). RpcClient gains `serverInfo()`; HttpSse fetches the
  unauth GET, Tauri invokes `rpc_server_info` (Phase 5 backend will
  wire it), mock returns a fixture with claude=null. SubmitJobDialog
  loads server-info on open, populates Runner as a `<Select>`
  honouring repo.default_runner then server default, branch defaults
  to `codeless/job-<6char>`, mock label shows "(demo)". Settings
  Models gets a "Coding agents" block above API keys, surfacing the
  Claude probe state with one-line actions.
- stage 2: codeless-adapters-host::claude (new module) ports the
  upstream ai-runner discovery (env -> PATH -> well-known dirs -> editor
  installs) and adds an async probe that surfaces ClaudeStatus on
  /server/info. Auth is a best-effort `claude /status --output-format
  json` with a 2s timeout; ambiguous outcomes return None so the UI's
  hint stays neutral instead of misreporting. host crate gains a
  codeless-rpc dep (mobile-safe layering preserved). Unix tests
  serialize on a tokio Mutex because they mutate process-wide env vars.
- stage 1: ServerInfo/RunnerInfo added to codeless-rpc (specta + wire-ts).
  AppState carries an Arc<ServerInfo> with a builder method
  `with_server_info`; serve.rs derives the snapshot from ServeArgs.
  Default-runner rule: real runners outrank `mock` when at least one is
  `--enable-*`'d, with `claude` beating `anthropic`. Route is plain
  `GET /server/info` outside the bearer gate (alongside healthz/version).
