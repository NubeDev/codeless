# HACKLINE-DEV — running codeless against the hackline repo

How to bring up the codeless stack, point it at the real `hackline`
checkout, control whether each job spawns a new branch, and pick
between Claude and Copilot at submit time.

This is a follow-on to [`START-SERVER-UI.md`](./START-SERVER-UI.md)
(which only covers the demo path) and the runner notes in
[`SCOPE.md`](./SCOPE.md). When the two disagree, SCOPE wins.

## TL;DR

```sh
# one-time: init bearer token store (no token needed for loopback,
# but the secrets file must exist so anthropic_api_key etc. can be set)
cargo run -p codeless-cli --bin codeless -- serve --init-token || true

# Terminal A — server
cd /home/user/code/rust/codeless-workspace/codeless
cargo run -p codeless-cli --bin codeless -- \
  --db /tmp/codeless-hackline.db serve \
  --bind 127.0.0.1:7777 \
  --fs-root /home/user/code/rust/codeless-workspace/hackline \
  --worktree-root ~/.cache/codeless/worktrees \
  --enable-claude
  # --enable-copilot   # not yet wired; see "Copilot gap" below

# Terminal B — UI
pnpm -C /home/user/code/rust/codeless-workspace/codeless/ui/codeless-ui dev

# Browser
open http://127.0.0.1:1420
```

Then in the UI: **Repos → Add repo** pointing at the hackline path,
**New Job** on that repo, pick the runner from the dropdown.

## 1. Target repo: hackline

The hackline checkout is already on disk at
[`/home/user/code/rust/codeless-workspace/hackline`](../hackline). The
codeless server treats it like any other git repo: point `--fs-root`
at it, then register it through `repos.add` (the UI's "Add repo"
button posts that RPC). The `RepoId` you get back is what
`SubmitJobArgs.repo_id` refers to.

Two things to know before submitting work:

1. **`--fs-root` is a security boundary, not a hint.** Every `fs.*`
   RPC canonicalises paths and rejects anything outside this root.
   If you want the editor pane to read files in hackline, the server
   must be started with `--fs-root` pointing at hackline (or an
   ancestor that contains it).

2. **The hackline directory currently has a `codeless-workspace`
   remote.** Check with `git -C /home/user/code/rust/codeless-workspace/hackline remote -v`.
   That looks wrong — if hackline has its own GitHub repo, re-clone
   it to a separate path (e.g. `~/code/hackline`) and use that as
   `--fs-root` instead. Otherwise codeless will push job branches to
   the codeless-workspace remote.

## 2. Branch control per job

`SubmitJobArgs` already carries the two knobs you need; the UI just
needs to surface them. The Rust side is in
[`crates/codeless-rpc/src/methods.rs:42`](../codeless/crates/codeless-rpc/src/methods.rs#L42).

| Field | Effect |
|---|---|
| `workspace_mode: "in_repo"` (default) | Agent edits the local clone in place. **No new branch, no worktree.** Whatever branch the clone is currently on is what the agent commits to. Best for "let me iterate against my actual working tree." |
| `workspace_mode: "worktree"` | Server runs `git worktree add -b <branch> <worktree-root>/job-<id>`. Fresh checkout, fresh branch, never touches your working tree. Source: [`codeless-adapters-host/src/worktree.rs:74`](../codeless/crates/codeless-adapters-host/src/worktree.rs#L74). |
| `branch: "<name>"` | The branch name used when `workspace_mode = "worktree"`. Blank/whitespace falls back to `codeless/job-<job_id>`. Ignored in `in_repo` mode. |

So "better control over whether the session makes a new branch" is
already there on the wire — the gap is UX. Two ways to drive it
today:

**A. Direct RPC (curl).** Skip the UI; submit the job by hand:
```sh
curl -s http://127.0.0.1:7777/rpc/submit_job \
  -H 'content-type: application/json' \
  -d '{
    "repo_id": "<repo-id-from-list_repos>",
    "prompt": "Add a /healthz endpoint",
    "runner": "claude",
    "branch": "",
    "workspace_mode": "in_repo",
    "cost_cap_cents": 200,
    "wall_clock_cap_ms": 600000,
    "start_immediately": true
  }'
```

**B. UI (current).** The submit wizard sets `workspace_mode` from a
toggle. If yours doesn't, the field is in the generated wire types
at `codeless/ui/codeless-ui/src/lib/rpc/generated/wire.ts` — wiring
it into the New Job form is a ~20-line change in the wizard
component. Open [`UI-PORT-AUDIT.md`](./UI-PORT-AUDIT.md) for where
the submit form lives.

### Cleaning up worktree-mode runs

`worktree` mode leaves a `codeless/job-<id>` branch in the source
repo even after the worktree is reaped. Prune from the hackline
checkout:
```sh
git -C /home/user/code/rust/codeless-workspace/hackline worktree prune
git -C /home/user/code/rust/codeless-workspace/hackline branch | grep codeless/job-
```

## 3. The Copilot gap

The vendored `ai-runner` crate already has a working Copilot wrapper
at [`ai-runner/src/runners/copilot.rs`](../ai-runner/src/runners/copilot.rs).
It is used **only** by the footer agent-chat panel (one-shot
free-form chat against the cwd of `codeless serve`), via the
registry constructed at
[`crates/codeless-cli/src/serve.rs:226`](../codeless/crates/codeless-cli/src/serve.rs#L226).

It is **not** wired into the job-driver path. So today, submitting a
job with `runner: "copilot"` returns `None` from `DefaultRunnerFactory`
and the driver fails the job. PROGRESS.md line 39 lists
`CopilotRunnerAdapter` as if it exists — that line is stale.

To get it working in the job-submit flow, three changes are needed
inside `codeless/`:

### 3.1 Add `CopilotRunnerAdapter` (codeless-runtime)

Mirror `ClaudeRunnerAdapter`. The Claude adapter lives at
[`crates/codeless-runtime/src/claude_runner.rs`](../codeless/crates/codeless-runtime/) —
it takes the host `ai_runner::CopilotRunner`, wraps it as a
`codeless_runtime::Runner`, and translates `ai_runner::Event`s into
`codeless_runtime::Event`s on the bus. Copy that file and:

- swap `CopilotRunner` in for `ClaudeRunner` at the spawn site
- drop the `permission_mode` / `effort` / `system_prompt` knobs
  (Copilot doesn't take them — comment at
  [`methods.rs:55-58`](../codeless/crates/codeless-rpc/src/methods.rs#L55-L58)
  already calls this out)
- keep `model` (the wrapper passes `--model`)

### 3.2 Add `--enable-copilot` flag (codeless-cli)

In [`crates/codeless-cli/src/serve.rs`](../codeless/crates/codeless-cli/src/serve.rs):

- Add `pub enable_copilot: bool` to `ServeArgs` next to `enable_claude`
  (line 83).
- In `build_server_info` (line 397), publish a `RunnerInfo { id:
  "copilot", default: false }` when the flag is set, and adjust the
  `mock` gate so `real_runner_enabled` also considers copilot.
- In `DefaultRunnerFactory::build` (line 547), add:
  ```rust
  "copilot" if self.enable_copilot => {
      let mut adapter = CopilotRunnerAdapter::new(prompt, TaskId::new());
      if let Some(m) = job.model.as_deref() {
          adapter = adapter.with_model(m);
      }
      Some(Arc::new(adapter))
  }
  ```
- Add the boot probe (mirror `probe_claude` at line 244) so the
  startup log says whether the `copilot` binary was found.

### 3.3 No UI change required

The runner dropdown is populated from `ServerInfo.runners`. Once
3.2 publishes `copilot`, it appears in the submit form automatically.
The job-page header's "[re-run ▾]" inherits it too.

### Verifying Copilot

Outside codeless, confirm the binary is installed and authenticated:
```sh
copilot --version
copilot -p 'hello' --allow-all-tools --no-ask-user
```

The wrapper requires `--allow-all-tools` and `--no-ask-user` for
non-interactive use — that's hard-coded in [`copilot.rs:74`](../ai-runner/src/runners/copilot.rs#L74).
If `copilot -p` won't run cleanly here, the codeless job won't
either; fix it at the binary level first.

## 4. Running both Claude and Copilot, user picks

Once 3.1–3.3 land, start the server with both flags:
```sh
cargo run -p codeless-cli --bin codeless -- \
  --db /tmp/codeless-hackline.db serve \
  --bind 127.0.0.1:7777 \
  --fs-root /home/user/code/rust/codeless-workspace/hackline \
  --worktree-root ~/.cache/codeless/worktrees \
  --enable-claude \
  --enable-copilot
```

Server log will report:
```
codeless-server: background driver enabled (runners=claude,copilot, ...)
```

The UI submit form's runner dropdown shows both; the user picks
per-job. The default (the one already selected when the form opens)
is `claude` — see the `default: true` in `build_server_info`
[line 421](../codeless/crates/codeless-cli/src/serve.rs#L421). If you
want Copilot as the default, swap which entry sets `default: true`.

## 5. Quick smoke test against hackline

After both runners are wired:

```sh
# 1. register hackline
curl -s http://127.0.0.1:7777/rpc/add_repo \
  -H 'content-type: application/json' \
  -d '{"path":"/home/user/code/rust/codeless-workspace/hackline"}'
# → { "repo_id": "01H..." }

# 2. submit a tiny in-repo job under Claude
curl -s http://127.0.0.1:7777/rpc/submit_job \
  -H 'content-type: application/json' \
  -d '{
    "repo_id":"01H...",
    "prompt":"Print the names of the top-level dirs in this repo.",
    "runner":"claude",
    "branch":"",
    "workspace_mode":"in_repo",
    "cost_cap_cents":50,
    "wall_clock_cap_ms":120000,
    "start_immediately":true
  }'

# 3. repeat with runner:"copilot" to compare
```

Watch the live events in the browser's job page, or stream them
directly:
```sh
curl -N http://127.0.0.1:7777/sse/jobs/<job-id>/events
```

## 6. Things to know / common stumbles

- **`mock` disappears the moment `--enable-claude` is passed.** That's
  on purpose ([`serve.rs:412`](../codeless/crates/codeless-cli/src/serve.rs#L412))
  so a real-runner server doesn't silently fall back to a no-op. If
  you want to demo without burning tokens, drop the flag and use
  `runner: "mock"`.
- **Process-spawn lives only in `codeless-adapters-host`** (R1 in
  [`CLAUDE.md`](../CLAUDE.md)). The Copilot adapter you add in 3.1
  must keep that property — `ai_runner::CopilotRunner` is the
  process owner, and the runtime crate only consumes its events.
- **The session env scrub at [`serve.rs:660`](../codeless/crates/codeless-cli/src/serve.rs#L660)**
  is Claude-specific. Copilot has its own auth store at `~/.copilot/`
  and isn't affected, so no analogous scrub is required.
- **`in_repo` mode and the editor pane share a working tree.** If
  you have unsaved edits in your editor and the agent commits over
  them, you'll fight the merge. Either work in `worktree` mode while
  iterating, or stage your edits first.
