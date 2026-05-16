# AGENT-CONTROL-PLANE — one job surface, three clients

**Status:** proposal, 2026-05-13. Concretises the load-bearing rule
that **CLI, REST, and MCP each expose the same job operations**, and
plans the work to close the gap between today's partial state and that
goal.

If anything below contradicts [`SCOPE.md`](./SCOPE.md), SCOPE wins.

## One-line summary

There is exactly **one** job-control surface — the `RpcServer` trait in
[`codeless-rpc`](../codeless/crates/codeless-rpc/src/server.rs). REST,
CLI, and MCP are three transports that adapt to that trait. Any agent
(Claude Code, Codex, Copilot, a script, a human typing) can create,
inspect, debug, and steer jobs through whichever transport is most
convenient, and they all see the **same** SQLite, the **same** event
bus, the **same** job rows.

## Why one surface, three transports

Three clients want to drive jobs:

| Client | Why it exists | Transport |
|---|---|---|
| Browser UI | the operator GUI | REST + SSE (axum) |
| `codeless` CLI | scripting, ssh, CI, power-user | REST + SSE under the hood |
| Claude / Codex / Copilot via MCP | agent-driven workflow automation | MCP (stdio or HTTP) |

If each client owned its own job logic, the system would diverge the
moment any one of them added a feature. The whole architecture is
designed around the opposite: **the runtime is the one and only place
state changes happen**, and clients are dumb translators. That is what
makes "Claude submits a job, you watch it in the browser, you cancel
it from the CLI" a one-line statement instead of an integration
project.

## The trust line

```
                       codeless-server (axum daemon)
                       SQLite + event bus + queue
                          ▲       ▲       ▲
                          │       │       │
                          │       │       │   each one is an
                          │       │       │   RpcClient impl
                          │       │       │
                       REST    REST    MCP (stdio | HTTP)
                       + SSE   + SSE     │
                          ▲       ▲       │
                          │       │       │
                       Browser   CLI    codeless-mcp
                                          │
                                  ┌───────┴────────┐
                                  │       │        │
                              Claude   Codex   Copilot
                                Code   CLI      CLI
```

The daemon is the **single source of truth**. Every client speaks to
it. There is no "MCP runtime" or "CLI runtime" — there is just **the
runtime**, addressed by three transports.

## What already exists today

| Surface | State |
|---|---|
| `codeless-rpc::RpcServer` trait | Defined. 29 methods cover repos, jobs, stages, reviews, fs, job files, events. |
| `codeless-server` REST routes | Wired. Every RpcServer method has a `POST /rpc/<method>` route in [routes.rs](../codeless/crates/codeless-server/src/routes.rs). |
| `codeless-client::HttpSseClient` | Implements `RpcClient` over HTTP + SSE. Used by the UI today. |
| `codeless` CLI | `codeless job submit`, `list`, `get`, `stop`, `rerun`, `start`, `tail`, plus repo/review/cost subcommands. Calls into `RpcClient`. |
| `codeless-mcp` (binary) | Runs stdio MCP via `rmcp`. Registers `BrowseFetchTool` and `HttpRequestTool` only. **No job tools.** This is the gap. |

So three of the four pieces are real and shipping. The MCP server
exists and serves tools — it just doesn't yet expose the job surface.

## What changes

### One new crate: `codeless-tools-jobs`

```
codeless/crates/codeless-tools-jobs/
├── Cargo.toml
└── src/
    ├── lib.rs                  // registers everything onto a ToolRegistry
    ├── ctx.rs                  // holds Arc<dyn RpcClient>; ToolCtx extension
    ├── job_create.rs           // codeless.job.create   → RpcClient::submit_job
    ├── job_start.rs            // codeless.job.start    → start_job
    ├── job_get.rs              // codeless.job.get      → get_job
    ├── job_list.rs             // codeless.job.list     → list_jobs
    ├── job_stop.rs             // codeless.job.stop     → stop_job
    ├── job_rerun.rs            // codeless.job.rerun    → rerun_job
    ├── job_diff.rs             // codeless.job.diff     → job_diff
    ├── stages_list.rs          // codeless.stages.list  → list_stages
    ├── events_tail.rs          // codeless.events.tail  → subscribe + take(N)
    └── repos.rs                // codeless.repo.{add,list,remove}
```

**Dependency graph (R1-safe):**

```
codeless-tools-jobs
   ├── codeless-tools   (Tool trait + registry)
   ├── codeless-client  (HttpSseClient, RpcClient)
   ├── codeless-types   (Job, JobId, Stage, Event, ...)
   └── codeless-rpc     (method arg/result structs)
```

No dependency on `codeless-runtime` or `codeless-adapters-host`. The
job tools are pure translators: deserialize MCP args → call
`RpcClient::method(args)` → serialize result.

Each tool is ~30 lines. Schemas are generated once at crate-build time
from the `*Args` structs in `codeless-rpc::methods` via `schemars`
(those structs already derive `specta::Type` for the TS bindings;
adding `schemars::JsonSchema` is one derive). The generated
`serde_json::Value` is held in a `OnceLock<serde_json::Value>` per
tool — no hand-written schemas, no drift from the wire types.

```rust
pub struct JobCreateTool { client: Arc<dyn RpcClient> }

impl Tool for JobCreateTool {
    fn name(&self) -> &str { "codeless.job.create" }
    fn schema(&self) -> &serde_json::Value { schema_for::<SubmitJobArgs>() }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> ToolResult<Value> {
        let args: SubmitJobArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let job = self.client.submit_job(args).await
            .map_err(translate_rpc_err)?;
        serde_json::to_value(job).map_err(|e| ToolError::Failed(e.to_string()))
    }
}
```

### Error translation across the boundary

`RpcClient` returns `Result<T, RpcError>`. MCP tools return
`ToolResult<Value>` (`InvalidArgs` / `Cancelled` / `Denied` /
`Failed`). The mapping:

| `RpcError` kind | `ToolError` | Notes |
|---|---|---|
| Transport (connection refused, timeout) | `Failed("daemon unreachable: ...")` | The agent host sees a tool failure result; the conversation can continue and try again. |
| Auth (401) | `Failed("auth rejected; check CODELESS_TOKEN")` | Distinct message so the operator can debug. |
| Not found (e.g. unknown `job_id`) | `Failed("not found: ...")` | Domain failure, not a protocol error. |
| Bad request (server-side validation) | `InvalidArgs(...)` | Surfaces the daemon's validator message — the schema check should have caught it client-side. |
| Server internal | `Failed(...)` | Opaque; daemon logs carry the detail. |

`Cancelled` is reserved for `ToolCtx` cancellation (the MCP call was
aborted mid-flight) — RPC calls don't produce it.

### `codeless-mcp` binary picks up two env vars

[main.rs](../codeless/crates/codeless-mcp/src/main.rs) gains:

- `CODELESS_SERVER_URL` — default `http://127.0.0.1:7777` (or whatever
  port the dev daemon binds to)
- `CODELESS_TOKEN` — bearer token, read from
  `~/.config/codeless/auth.toml` if unset

It constructs an `HttpSseClient`, then registers job tools alongside
the existing browse/http ones:

```rust
let client: Arc<dyn RpcClient> =
    Arc::new(HttpSseClient::new(server_url, token));

let mut registry = ToolRegistry::new();
registry.register(Arc::new(BrowseFetchTool::new()));
registry.register(Arc::new(HttpRequestTool::new()));
codeless_tools_jobs::register_all(&mut registry, client.clone());
```

### Parity check (SCOPE rule, currently aspirational)

Rust traits aren't reflectable at runtime, so the parity check needs
a concrete source of truth. The chosen mechanism is the existing
**HTTP route table** in
[`codeless-server/src/routes.rs`](../codeless/crates/codeless-server/src/routes.rs):
it already enumerates every RpcServer method that has crossed the
network boundary, it's a single file, and it's where new methods get
wired up anyway. A test in `codeless-mcp` reads the route names (a
small `pub const RPC_METHODS: &[&str]` exported from
`codeless-server`, populated next to the router build) and asserts
that every entry has either:

- a registered MCP tool whose name maps via a `TOOL_FOR_METHOD` table,
  **or**
- an entry in `opt_out.toml` with a reason (e.g. `secrets.get =
  "secret material; out-of-band only"`).

Same check, same source list, runs against the CLI's clap subcommand
tree. This is the "parity is a CI check, not a convention" rule from
SCOPE.md "MCP surface". A proc-macro on `RpcServer` would also work
and is a free upgrade later; starting from the route table is the
smallest viable mechanism.

## How a job actually flows, with all three clients

A job submitted from Claude, watched in the browser, cancelled from
the CLI — same job row, same event stream, same daemon:

```
T0  Claude Code conversation:
       "submit a job to add dark-mode to settings"
    └─► codeless-mcp / codeless.job.create
        └─► HttpSseClient.submit_job(template)
            └─► POST /rpc/submit_job
                └─► codeless-runtime: writes Job row, queues it
                    └─► returns job_id = "abc123"

T1  Browser UI (already subscribed to /api/events):
       SSE pushes `job-queued { id: "abc123" }`
       UI lights up the new card

T2  Claude polls:
    └─► codeless-mcp / codeless.job.status { job_id: "abc123" }
        └─► HttpSseClient.get_job
            └─► POST /rpc/get_job
                └─► returns Job { state: Running, current_stage: 2, ... }

T3  You decide to cancel:
       $ codeless job stop abc123
    └─► CLI's HttpSseClient.stop_job
        └─► POST /rpc/stop_job
            └─► runtime flips state to Cancelling, kills the runner
                └─► SSE emits `job-stopped`

T4  Both Claude (on its next status poll) and the browser UI
    (via SSE) see the new state. Same row, same truth.
```

## Tool catalogue (first slice → full parity)

### Slice 1 (smallest useful loop — ship this first)

| MCP tool | RPC method | What an agent does with it |
|---|---|---|
| `codeless.job.create` | `submit_job` | start a job |
| `codeless.job.get` | `get_job` | check state and current stage |
| `codeless.job.list` | `list_jobs` | enumerate active jobs |

Three tools. Enough for "submit, wait, see state."

`codeless.events.tail` was tempting to slot here as a debug primitive,
but `subscribe` returns an open `Stream`, not a bounded list — it has
no shape that fits into a single MCP tool result. Rather than ship a
tool with "TBD semantics," `events.tail` is gated on a new
`list_events` RPC method that returns a bounded slice:

```rust
// new on RpcServer
async fn list_events(&self, args: ListEventsArgs) -> RpcResult<ListEventsResult>;

struct ListEventsArgs {
    job_id: Option<JobId>,         // None = global
    since: Option<EventCursor>,    // None = "from the start"
    limit: u32,                    // server caps at 1000
}
struct ListEventsResult {
    events: Vec<EventEnvelope>,
    next_cursor: Option<EventCursor>,
}
```

This lands as Slice 1's blocking dependency — RPC method + REST route
+ `codeless events list` CLI subcommand + MCP tool, in that order.

### Slice 2 (control)

`codeless.job.start`, `codeless.job.stop`, `codeless.job.rerun`,
`codeless.job.diff`, `codeless.stages.list`.

### Slice 3 (repos + files)

`codeless.repo.{add,list,remove}`, `codeless.job.files.{list,read,write}`,
`codeless.job.template.update`, `codeless.handover.write`.

### Slice 4 (reviews + chat)

`codeless.review.{list,approve,comment,stop}`, `codeless.agent.chat`.

### Never exposed

The opt-out list lives in `opt_out.toml` next to the parity check;
secrets and auth are the seed set, more entries land as the surface
grows. The rule of thumb: if a single approved tool call could
exfiltrate or rotate something irreversible, it belongs out-of-band.

Seed list (per SCOPE.md "What never crosses the MCP boundary"):

- `codeless.secrets.*` — secret material; out-of-band only.
- `codeless.auth.*` — bearer-token lifecycle.

Candidates to add as the surface grows: any future `runtime.shutdown`
or `runtime.gc_db`, `fs.write` outside a job worktree (the existing
`fs.*` RPCs are worktree-scoped — verify before wiring), and
potentially `repo.remove` (destructive, but reversible by re-add —
keep exposed unless a real incident motivates removal).

## REST and CLI parity — what's still needed

REST is essentially complete: every RpcServer method has a route. The
CLI covers the common path (`job submit`, `list`, `get`, `stop`,
`rerun`, `start`, `tail`). Two gaps worth closing while the parity
check is being added:

- `add_job_note` RPC (called out in PROGRESS.md as a prerequisite for
  the re-run-with-feedback UI flow) — needs to exist on the trait
  before `codeless.job.rerun` can carry a note. Slice 2 work.
- `subscribe` does not yet have a bounded "tail last N events for job
  X" mode suitable for an MCP tool result. Either add a new method
  (`list_events`) or document a convention on top of `subscribe` with
  an explicit `take_until` / event count. Slice 1 work.

## Why MCP needs a running daemon (and why that's fine)

Option-1 ("in-process runtime per MCP child") was rejected because:

- Each spawned MCP child would carry its own SQLite handle. SQLite is
  a single-writer database; concurrent MCP children racing on the
  same `codeless.db` is a data-corruption story, not a feature.
- Children would not see each others' jobs. The whole appeal of MCP
  driving codeless is that Claude and Codex are working *the same
  queue* — Option-1 hands them three disjoint queues.
- The MCP binary would have to link `codeless-runtime` and
  `codeless-adapters-host`. R1 doesn't strictly forbid this — MCP is
  host-side, not mobile-safe — but the same architectural reason
  mobile doesn't link those crates (the runtime is heavy, owns
  process spawn, owns SQLite) is the reason MCP shouldn't either.
  The MCP binary's job is *translation*, not running the system.

Option-2 ("MCP is an RPC client") is the chosen design. The cost is
that the daemon must be running. That's fine — the daemon must be
running for the UI to work anyway. The startup model is:

```
codeless serve &                      # once, in a tmux pane or a service
codeless-mcp                          # spawned by each agent host,
                                      # connects to the daemon
```

This mirrors how every other dev daemon works (LSP, Docker,
PostgreSQL). Agents come and go; the runtime stays.

## Single trust boundary (R5)

Local dev: `codeless serve` listens on `127.0.0.1`. The bearer token
from `~/.config/codeless/auth.toml` authorises every client
identically — browser, CLI, MCP. Per SCOPE.md "Asymmetry between stdio
and HTTP MCP", a stdio MCP server inherits the user's local trust and
does not need a bearer; an HTTP MCP server (Phase 2.5+) uses the same
bearer as REST.

Phase 7 swaps the bearer for OIDC. The MCP transports pick up the
change for free because they only know `RpcClient`.

## Implementation order

Slice numbers in this section refer to the tool catalogue above
(Slice 1 = create/get/list, Slice 2 = control verbs, etc.).

1. **`codeless-tools-jobs` crate skeleton.** Empty crate, registered
   in workspace `Cargo.toml`. Trait imports, no tools yet.
2. **`list_events` lands.** New RPC method + `*Args` / result structs
   + REST route + CLI `codeless events list` subcommand. **Blocks
   Slice 1's `events.tail`** (and replaces today's "no bounded tail"
   gap entirely).
3. **Slice 1 tools** — `job.create`, `job.get`, `job.list`,
   `events.tail`. Wire them onto the registry in
   `codeless-mcp/main.rs`. Stdio handshake test extended to call each
   tool against a mock RPC server.
4. **End-to-end smoke** — start the daemon, spawn `codeless-mcp`
   under `claude --mcp-config`, submit a real toy job from a Claude
   conversation, watch it in the browser. Document in
   [`START-SERVER-UI.md`](./START-SERVER-UI.md).
5. **`add_job_note` lands.** RPC method + REST route + CLI subcommand
   (`codeless job note <id> <text>`) + driver-side prompt folding (the
   `TODO` already sitting around line 205 of `job_driver_loop.rs` per
   PROGRESS.md). **Blocks Slice 2's `codeless.job.rerun`** carrying a
   note — Slice 2 ships without note support otherwise.
6. **Slice 2** — control verbs (start/stop/rerun/diff/stages).
7. **Parity CI check.** `RPC_METHODS` const exported from
   `codeless-server`; test in `codeless-mcp` and `codeless-cli`
   asserts every entry has a tool / a subcommand / an opt-out reason.
8. **Slice 3 + 4.** Files, templates, reviews, chat.
9. **HTTP MCP transport.** `codeless-mcp --listen :PORT` for hosted
   agent hosts that prefer HTTP over stdio.

## What stays out of scope

- A "Codeless agent SDK." Agents talk to the daemon through the same
  client interface as everything else.
- Per-agent permission scoping. Single trust boundary in MVP (R5).
- Re-introducing in-process MCP runtimes for any reason. The daemon
  is the runtime.

## Pointers

- The transport rule: [`SCOPE.md` — Rule 1](./SCOPE.md#rule-1--one-transport-interface-many-implementations)
- MCP surface design: [`SCOPE.md` — MCP surface](./SCOPE.md#mcp-surface-phase-2--headless-control-plane-for-ai-agents)
- Existing MCP server: [`codeless/crates/codeless-mcp/`](../codeless/crates/codeless-mcp/)
- Tool trait: [`codeless/crates/codeless-tools/`](../codeless/crates/codeless-tools/)
- RPC trait: [`codeless/crates/codeless-rpc/src/server.rs`](../codeless/crates/codeless-rpc/src/server.rs)
- HTTP routes: [`codeless/crates/codeless-server/src/routes.rs`](../codeless/crates/codeless-server/src/routes.rs)
- RPC client: [`codeless/crates/codeless-client/`](../codeless/crates/codeless-client/)
- Tool porting context: [`TOOLS-PORTING.md`](./TOOLS-PORTING.md)
