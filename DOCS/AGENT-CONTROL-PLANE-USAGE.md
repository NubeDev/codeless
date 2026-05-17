# AGENT-CONTROL-PLANE-USAGE — operator quickstart

**Status:** quickstart, 2026-05-13. Sibling to
[`AGENT-CONTROL-PLANE.md`](./AGENT-CONTROL-PLANE.md) (which is the
*plan*). This doc is the *use* — what to type, what to expect, how to
debug it when it goes wrong. Some commands below describe the
end-state once `codeless-tools-jobs` is implemented; the "Today vs.
end-state" matrix at the bottom tells you which is which.

## TL;DR

```sh
codeless serve                                  # one daemon, on 127.0.0.1
                                                # (run under tmux / a user service —
                                                # `&` loses stderr and dies with the shell)

codeless job submit --repo foo --prompt "..."   # CLI path — works today

curl -X POST http://127.0.0.1:7777/rpc/list_jobs \
    -H "Authorization: Bearer $CODELESS_TOKEN" \
    -d '{}'                                     # REST path — works today

codeless-mcp                                    # MCP path — serves browse/http tools
                                                # today; job-control tools land in
                                                # Slice 1 per AGENT-CONTROL-PLANE.md.
                                                # See "Today vs. end-state" matrix.
```

All three call the same daemon, mutate the same SQLite, and observe
the same SSE event bus.

## Step 0 — start the daemon

The daemon is the runtime. Nothing in this doc works without it.

```sh
codeless serve --bind 127.0.0.1:7777
```

The first run writes:

- A SQLite DB (XDG data dir) — source of truth.
- `~/.config/codeless/auth.toml` — bearer token, chmod 600. XDG-aware
  (`$XDG_CONFIG_HOME` overrides on Linux; macOS default is the same
  path unless `XDG_CONFIG_HOME` is set elsewhere). The path used by
  the CLI today is asserted in
  [`codeless-cli/src/main.rs`](../codeless/crates/codeless-cli/src/main.rs).

Get the token. Don't `awk` the TOML — quoting variants will burn you.
The robust path is the CLI subcommand (lands as a one-line addition
if it isn't there yet); the fallback is a real TOML parser:

```sh
# preferred
export CODELESS_TOKEN=$(codeless auth print-token)

# fallback if you have tq / dasel
export CODELESS_TOKEN=$(tq -r .token ~/.config/codeless/auth.toml)

# last resort — only valid if you've eyeballed the file shape
export CODELESS_TOKEN=$(python3 -c \
  "import tomllib,sys; print(tomllib.load(open('$HOME/.config/codeless/auth.toml','rb'))['token'])")

export CODELESS_SERVER_URL=http://127.0.0.1:7777
```

## Step 1 — the CLI path

The CLI is the simplest client and the canonical way to verify the
daemon is alive.

```sh
# repos
codeless repo add --name myproj \
                  --clone-url git@github.com:me/myproj.git \
                  --default-branch main \
                  --local-path /home/me/code/myproj
codeless repo list

# jobs
codeless job submit --repo myproj \
                    --prompt "add a dark-mode toggle to settings" \
                    --runner claude \
                    --branch feat/dark-mode
codeless job list
codeless job get <id>
codeless job tail <id>          # SSE stream, follow events live
codeless job stop <id>
codeless job rerun <id>
```

Every one of these is a thin wrapper over `RpcClient`. There is no
separate "CLI runtime" — the CLI is a client of the daemon.

## Step 2 — the REST path

For scripts, CI, and "is the daemon up?" smoke checks:

```sh
# Health-ish: list repos
curl -sX POST $CODELESS_SERVER_URL/rpc/list_repos \
     -H "Authorization: Bearer $CODELESS_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{}'

# Submit a job
# Shape as of 2026-05-13; canonical definition is SubmitJobArgs in
# codeless-rpc/src/methods.rs — trust the source over this snippet.
curl -sX POST $CODELESS_SERVER_URL/rpc/submit_job \
     -H "Authorization: Bearer $CODELESS_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{
       "repo_id": "01934c8e-0000-7000-8000-000000000001",
       "prompt": "add a dark-mode toggle",
       "runner": "claude",
       "branch": "feat/dark-mode",
       "cost_cap_cents": 500,
       "wall_clock_cap_ms": 1800000,
       "start_immediately": true
     }'

# Tail events for a job (SSE). The filter uses ?scope= per
# codeless-server/src/sse.rs; "job" scope requires job_id.
curl -N "$CODELESS_SERVER_URL/events?scope=job&job_id=01934c8e-0000-7000-8000-000000000002" \
     -H "Authorization: Bearer $CODELESS_TOKEN" \
     -H "Accept: text/event-stream"

# Or every event the runtime emits:
curl -N "$CODELESS_SERVER_URL/events?scope=all" \
     -H "Authorization: Bearer $CODELESS_TOKEN" \
     -H "Accept: text/event-stream"
```

Every method on the `RpcServer` trait has a `POST /rpc/<method>` route
in [`routes.rs`](../codeless/crates/codeless-server/src/routes.rs).
The wire shapes are the `*Args` / result structs in
[`codeless-rpc/src/methods.rs`](../codeless/crates/codeless-rpc/src/methods.rs).
The SSE handler and its query shape live in
[`codeless-server/src/sse.rs`](../codeless/crates/codeless-server/src/sse.rs).

## Step 3 — the MCP path

This is what makes Claude Code, Codex, and Copilot CLI able to drive
codeless from a conversation.

### Register `codeless-mcp` with your agent host

**Claude Code** — add to the project-local `.mcp.json`, or the
global Claude Code settings file (path varies by version — `claude
mcp add` is the version-safe way; check `claude --help` if the
manual path below has rotted):

```json
{
  "mcpServers": {
    "codeless": {
      "command": "codeless-mcp",
      "env": {
        "CODELESS_SERVER_URL": "http://127.0.0.1:7777",
        "CODELESS_TOKEN": "<paste from ~/.config/codeless/auth.toml>"
      }
    }
  }
}
```

**Codex CLI** — same shape, different config file path; consult the
Codex docs.

**Copilot CLI** — same shape; consult the Copilot CLI MCP docs.

The agent host spawns `codeless-mcp` as a stdio subprocess on demand.
That subprocess connects to the daemon over HTTP using the bearer
token, and translates every MCP `tools/call` into an `RpcClient`
method call.

### Tools the agent sees

End-state (per [AGENT-CONTROL-PLANE.md](./AGENT-CONTROL-PLANE.md)
slice plan):

| Tool | Purpose |
|---|---|
| `codeless.job.create` | submit a job |
| `codeless.job.get` | fetch a job + current stage |
| `codeless.job.list` | enumerate jobs |
| `codeless.job.start` | promote Draft → Queued |
| `codeless.job.stop` | cancel a running job |
| `codeless.job.rerun` | re-run with optional note |
| `codeless.job.diff` | unified diff of the worktree |
| `codeless.stages.list` | stage tree + rollups |
| `codeless.events.tail` | last N events — the **debug primitive** |
| `codeless.repo.{add,list,remove}` | repo CRUD |

`codeless.secrets.*` and `codeless.auth.*` are **never** exposed via
MCP. They stay CLI-only. See SCOPE.md "What never crosses the MCP
boundary".

### A real Claude conversation

```
User:         submit a codeless job to add a dark-mode toggle to the
              settings page in myproj, using claude on a fresh branch

Claude:       I'll submit it now.
Tool call:    codeless.job.create {
                repo_id: "01934c8e-...-myproj",
                prompt: "...",
                runner: "claude",
                branch: "feat/dark-mode",
                start_immediately: true
              }
Tool result:  { job_id: "01934c8e-...-job", state: "Queued" }
Claude:       Submitted job 01934c8e-...-job. Want me to watch it?

User:         yes

Tool call:    codeless.events.tail { job_id: "01934c8e-...-job", limit: 50 }
Tool result:  [ { kind: "stage-started", stage: "plan", ... },
                { kind: "tool-call", tool: "read_file", path: "settings.tsx" },
                { kind: "stage-completed", stage: "plan" },
                { kind: "stage-started", stage: "implement" },
                ... ]
Claude:       It's on stage `implement`, last tool-call was read_file
              on settings.tsx 30s ago. Looks healthy.

User:         it looks stuck, stop it

Tool call:    codeless.job.stop { job_id: "01934c8e-...-job" }
Tool result:  { state: "Cancelling" }
Claude:       Stopped. The partial diff is on the worktree branch
              feat/dark-mode if you want to inspect it.
```

`Tool call:` / `Tool result:` lines are what the MCP transport
carries. `Claude:` lines are the assistant's prose to you. Those are
two different surfaces; in the real Claude Code UI they render
distinctly.

This is the workflow loop. You are talking to Claude; Claude is
talking to the daemon through MCP; the daemon is running the actual
coding job in a real git worktree, persisting every step to SQLite,
and broadcasting events that the browser UI is *also* watching live.

## Debugging — what to check when something is wrong

### Test the MCP server in isolation

Before blaming the agent host, confirm the MCP server itself answers:

```sh
# List the registered tools — should print a JSON-RPC response with
# every tool codeless-mcp registered at startup.
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
| CODELESS_SERVER_URL=$CODELESS_SERVER_URL CODELESS_TOKEN=$CODELESS_TOKEN \
  codeless-mcp
```

If `tools/list` returns a tool array, the MCP server is fine and the
issue is in the agent host's wiring. If it errors or hangs, the
server itself is broken — check its stderr.

### "MCP tool isn't appearing in Claude"

1. Is `codeless-mcp` on `$PATH`? (`which codeless-mcp`)
2. Did the agent host swallow stderr? Run the isolation test above
   to see the server's actual reply.
3. Check the agent host's MCP logs (Claude Code: `~/.claude/logs/`).

### "Tool calls fail with a 401 / auth error"

`CODELESS_TOKEN` is wrong or missing. Re-read it from
`~/.config/codeless/auth.toml`. Note: the env var lives in the agent
host's config (`.mcp.json`'s `env` block), not in your shell — the
agent host spawns the MCP subprocess and chooses its env.

### "Tool calls fail with 'connection refused'"

The daemon isn't running, or it's bound to a different address.
`curl -sX POST $CODELESS_SERVER_URL/rpc/list_repos -H ...` is the
isolation test — if curl can't reach it, neither can MCP.

### "A job hangs in Queued"

The driver loop didn't pick it up. Most likely cause (per PROGRESS.md
"Driver surfaces worktree-allocation failures"): the branch already
exists in another worktree. Check:

```sh
codeless job get <id>       # state, current_stage_seq
codeless job tail <id>      # last events — driver failures land here
```

If the driver isn't surfacing the failure yet (it's a tracked gap),
look at the daemon's stderr or `~/.local/share/codeless/codeless.log`.

### "Two agents are stepping on each other"

Per-job state transitions are serialised in the daemon, so the common
suspicion ("both my agents wrote to the same job at once") is wrong —
they got two `JobId`s, or the second mutation was a no-op against the
state machine. Two real conflict shapes do exist, though:

- **Same repo, same branch.** Two `submit_job` calls naming the same
  `branch` against the same repo race on worktree allocation; the
  second one hangs in `Queued` until the driver surfaces the failure
  (see "A job hangs in Queued" above). Use distinct branches.
- **Concurrent `job.stop` on the same id.** First call wins and
  flips state to `Cancelling`; second call either no-ops or returns
  a state-error depending on timing. Both are safe — the job ends
  up cancelled either way.

If what you're seeing looks like "two streams of the same job," that's
two clients subscribed to the same SSE stream, not two jobs.

## Today vs. end-state (what works as of 2026-05-13)

Slice numbers below refer to the tool catalogue in
[`AGENT-CONTROL-PLANE.md` § Tool catalogue](./AGENT-CONTROL-PLANE.md#tool-catalogue-first-slice--full-parity).

| Capability | CLI | REST | MCP |
|---|---|---|---|
| Submit job | yes | yes | **not yet** (Slice 1) |
| Get job / list jobs | yes | yes | **not yet** (Slice 1) |
| Tail events | yes (`job tail`) | yes (`/api/events`) | **not yet** (Slice 1) |
| Start / stop / rerun | yes | yes | **not yet** (Slice 2) |
| Diff / stages | yes | yes | **not yet** (Slice 2) |
| Repos CRUD | yes | yes | **not yet** (Slice 3) |
| Job files / templates / handover | partial | yes | **not yet** (Slice 3) |
| Reviews | yes | yes | **not yet** (Slice 4) |
| Browse / HTTP tools | n/a | n/a | **yes today** |
| Secrets / auth | yes | yes | **never** (intentional) |

The MCP server runs today and serves the browse/http tools — it just
doesn't yet expose any job operations. That gap is the entire content
of [AGENT-CONTROL-PLANE.md](./AGENT-CONTROL-PLANE.md).

## Pointers

- Plan / architecture: [`AGENT-CONTROL-PLANE.md`](./AGENT-CONTROL-PLANE.md)
- MCP surface contract: [`SCOPE.md` — MCP surface](./SCOPE.md#mcp-surface-phase-2--headless-control-plane-for-ai-agents)
- Existing MCP server: [`codeless/crates/codeless-mcp/`](../codeless/crates/codeless-mcp/)
- REST routes: [`codeless/crates/codeless-server/src/routes.rs`](../codeless/crates/codeless-server/src/routes.rs)
- RPC trait: [`codeless/crates/codeless-rpc/src/server.rs`](../codeless/crates/codeless-rpc/src/server.rs)
- Method wire shapes: [`codeless/crates/codeless-rpc/src/methods.rs`](../codeless/crates/codeless-rpc/src/methods.rs)
- Server / UI startup: [`START-SERVER-UI.md`](./START-SERVER-UI.md)
