# Plugin Substrate — Process runtime flavour (deferred)

Status: deferred design
Owner: ap@nube-io.com
Created: 2026-05-18

Companion to [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md). This doc
fills in item 11 — the *seam* for an out-of-process plugin host. The
substrate doc owns the *what* and *why*; this doc records the design
so it doesn't drift when the time comes to land it.

**This is not MVP work.** The substrate ships with builtin + WASM
flavours; process lands when one of the following becomes true:

- A plugin needs a polyglot runtime (Python, Go, TS) for ecosystem
  reasons that WASM can't satisfy (model-specific libraries, mature
  bindings).
- A plugin needs crash isolation the WASM sandbox cannot provide
  (long-running drivers, large native dependencies, plugins that
  link to native libraries we can't recompile to WASM).

If neither is true, this design stays a doc. The point of writing it
now is to record the rubix-lifted shape while the context is fresh
and to reserve the `[[runtimes]] kind = "process"` slot in
`plugin.toml` so an existing manifest doesn't have to change when
the flavour lands.

If anything below contradicts [`PLUGIN-SUBSTRATE.md`](./PLUGIN-SUBSTRATE.md)
or [`../SCOPE.md`](../SCOPE.md), those win.

## One-line summary

A plugin may ship a standalone binary that speaks gRPC over a Unix-
domain socket to the codeless server, supervised by a per-plugin
state machine with a circuit breaker. The contract is fixed by
`tool.proto`; the SDK side (`codeless-plugin-sdk` with `feature =
"process"`) hides the gRPC plumbing so authors implement the same
`Tool` trait as builtin and WASM flavours.

This is the codeless analogue of rubix's process-block design ([rubix
EXTENSIONS.md § Process-block wire in detail](../../../rubix-workspace/rubix-agent/docs/design/extensions/EXTENSIONS.md#process-block-wire-in-detail)),
narrowed from "node kinds with slots and ports" to "tools with input/
output JSON schemas."

## What gets reused from rubix

| Rubix asset | Reuse | Notes |
|---|---|---|
| `extension-sdk/extensions-sdk/src/process.rs` (757 LoC) | **Reference**, then port the supervisor + `BehaviorProxy` shape. | The runtime model carries; the trait shape (NodeBehavior vs ToolBehavior) is new. |
| `block.proto` | **Pattern lift**, write `tool.proto` from scratch matching the codeless tool trait. | gRPC service has five methods: `Describe`, `Health`, `Call`, `Subscribe` (events), `Cancel`. No `Discover` (codeless tools are not instance-bearing). |
| `extensions-host` supervisor state machine (`Discovered → Starting → Running → Restarting | Failed | Stopped`) | **Pattern lift, copy code shape.** Circuit breaker, exponential backoff, `Failed` opt-in sticky semantics. | Documented at length in rubix EXTENSIONS.md §  Supervisor lifecycle. Same model fits codeless. |
| `process-wrap` (process-group leader on Unix, job object on Windows) | **Direct dep.** | Same crate. Same rationale: a plugin that spawns helpers must not leak zombies on agent restart. |
| `HostPolicy` (socket timeout, health interval, failure threshold/window, optional cooldown) | **Direct port.** | Knobs are domain-free; lift the struct verbatim, rename fields if codeless conventions demand. |

What's not reused: the rubix `BehaviorProxy` routes `OnMessage` /
`OnTimer` / `OnConfig` events to nodes; codeless's proxy routes
exactly one verb: `Call(tool_id, args_json) → ToolResult`. The
proxy itself is smaller.

## The proto contract — `tool.proto`

Lives in `crates/codeless-tools/proto/tool.proto`. Versioned: add-
only within a major version (per
[`../../codeless/CLAUDE.md`](../../codeless/CLAUDE.md) on stability).
CI diffs the proto against the previous release and fails on
removed/renamed fields.

```proto
syntax = "proto3";
package codeless.tool.v1;

service ToolPlugin {
  // Identity + declared tools. Single attempt with a deadline.
  rpc Describe(DescribeRequest) returns (DescribeResponse);

  // Liveness ping at HostPolicy.health_interval.
  rpc Health(HealthRequest) returns (HealthResponse);

  // Synchronous tool invocation. The client (codeless-server) is
  // responsible for enforcing the persona allowed_tools list before
  // ever calling this — the plugin trusts the server.
  rpc Call(CallRequest) returns (CallResponse);

  // Optional: server-stream of structured progress events for long-
  // running calls. Plugins that don't need it ignore Subscribe.
  rpc Subscribe(SubscribeRequest) returns (stream SubscribeEvent);

  // Cooperative cancel of an in-flight Call. The plugin SHOULD
  // honour it within HostPolicy.cancel_grace; otherwise the
  // supervisor terminates the process.
  rpc Cancel(CancelRequest) returns (CancelResponse);
}

message DescribeResponse {
  string plugin_id = 1;
  string version   = 2;
  repeated ToolManifest tools = 3;
}

message ToolManifest {
  string id            = 1;
  string description   = 2;
  string input_schema  = 3;   // JSON schema serialised
  string output_schema = 4;
  Tier   tier          = 5;
}

enum Tier {
  TIER_UNSPECIFIED = 0;
  READ             = 1;
  WRITE            = 2;
  DESTRUCTIVE      = 3;
}

message CallRequest {
  string call_id   = 1;       // for Cancel correlation
  string tool_id   = 2;
  string args_json = 3;
  string thread_id = 4;       // for attachment scoping
}

message CallResponse {
  oneof outcome {
    string ok_json = 1;
    ToolError err  = 2;
  }
}

message ToolError {
  string code      = 1;
  string message   = 2;
  bool   retryable = 3;
}
```

**Why thread-id, not session-id.** Codeless's attachment scope is
the thread; the plugin uses `thread_id` to mint or read attachments
through host calls. The plugin does *not* see the user identity —
R5 is single-tenant anyway, and plugins should not encode user-
specific behaviour.

**Why no `Discover`.** Rubix nodes have instances (e.g. BACnet
devices discovered on a network); codeless tools do not. A plugin
that wants "list X" exposes that as a tool (`catalog.list`), not as
a discovery stream.

## Supervisor lifecycle

The state machine. Lifted from rubix EXTENSIONS.md verbatim because
the same shape applies; differences are noted inline.

```
Discovered → Starting → Running ──┬─→ Restarting → Running
                                  ├─→ Failed (sticky by default)
                                  └─→ Stopped (operator-driven)
```

Per-plugin startup:

1. **Spawn** the binary as a process group leader (Unix) / job
   object (Windows) via `process-wrap`. The supervisor owns the
   wrapped child; on shutdown or restart the whole tree is
   signalled.
2. **Await the UDS** the binary binds, up to
   `HostPolicy::socket_ready_timeout` (default **5s**, codeless-
   configurable like every other timeout).
3. **Identity check on `Describe`.** The reported `plugin_id` must
   equal the directory name; the declared tools must be a subset
   of what the manifest claims for this plugin. A stale binary
   cannot impersonate another plugin; a binary that grew a new tool
   without updating the manifest cannot silently start serving it.
4. **Register proxies.** For every tool the plugin owns, a
   `ProcessToolProxy` is inserted into `codeless-tools::Registry`.
   `Call(args)` from the runtime routes over the UDS. On shutdown
   the proxy entries are removed atomically — the runtime never
   holds a route to a dead channel.
5. **Health-tick** on `HostPolicy::health_interval` (default
   **10s**), in parallel with supervising the child handle. A
   crash is observed by whichever fires first.

Crash / Health failure:

- A sliding window of `failure_times` is kept in memory.
- If the count reaches `HostPolicy::failure_threshold` within
  `HostPolicy::failure_window`, the plugin moves to `Failed` and is
  left alone until an operator calls `enable()`.
- Otherwise restart with exponential backoff
  (`HostPolicy::backoff_initial`, `backoff_max`, jittered).
- Headless deployments can opt out of "Failed is sticky" by setting
  `HostPolicy::failed_cooldown = Some(d)`: the supervisor sleeps
  `d`, clears the window, retries. Same semantics rubix offers.

Operational edges (lifted verbatim from rubix; the concerns are
identical):

- **Circuit-breaker state is in-memory.** Agent restart resets the
  window. Acceptable for crew-attended deployments; persist into the
  codeless DB if it stops being acceptable.
- **`Failed` is opt-in sticky.** Default is "wait for an operator";
  headless edge ops can flip to "retry forever."
- **`Health` is a gRPC ping, not application liveness.** A plugin
  that deadlocks inside its own runtime keeps answering Health.
  Plugin authors with real concurrency expose their own liveness
  probe and fail Health themselves when it trips.
- **Slow startup is tunable, not retried.** `Describe` is single-
  attempt against `socket_ready_timeout`; heavy plugins should bump
  the timeout rather than relying on restart backoff to absorb cold
  starts.

## Capability surface — what the plugin can do back to the host

The plugin is a separate process; "imports" from it are gRPC calls
back over the same UDS, multiplexed on the same connection. The
v0.1 reverse interface:

```proto
service ToolHost {
  // Read an attachment owned by the plugin's current thread.
  rpc AttachmentRead(AttachmentReadRequest)
      returns (stream AttachmentChunk);

  // Mint a new attachment under the plugin's current thread.
  // Returns the attachment_id the plugin then references in its
  // CallResponse.
  rpc AttachmentWrite(stream AttachmentWriteChunk)
      returns (AttachmentWriteResponse);

  // Plugin-owned SQLite table access — same as WASM's kv interface
  // open question (OQ-WASM-3), not in v0.1 of process either.
  // Reserved for v0.2.
}
```

Same default-deny posture as WASM: a process plugin gets no
filesystem-on-the-host, no network beyond its own outbound (which
the OS allows; we don't try to firewall it from inside the
supervisor), no codeless-runtime side-channels. Attachment R/W is
the only reverse path in v0.1.

A process plugin that needs the host's filesystem reads / writes
through `AttachmentWrite` (the host already owns scoped storage in
SQLite). A process plugin that needs the network does it itself
through its own runtime. The codeless server doesn't proxy network
egress.

## Authoring — the SDK side

`codeless-plugin-sdk` with `feature = "process"` builds a binary
that:

1. Binds a UDS at `$CODELESS_PLUGIN_SOCK` (set by the supervisor
   before spawn).
2. Implements `ToolPlugin::Describe`, `Health`, `Call` from the
   author's `Tool` impls via a single `run_plugin()` call:

   ```rust
   fn main() -> Result<()> {
       codeless_plugin_sdk::run_plugin(|reg| {
           reg.register::<NotesAppend>();
           reg.register::<NotesList>();
       })
   }
   ```

3. The author's `Tool` impl is the same trait as builtin and WASM
   flavours. The runtime adapter is the only thing that changes.

Non-Rust authors get a generated client SDK from the same proto:
`tonic-go`, `betterproto` (Python), `nice-grpc` (TS). The SDK
surface is language-idiomatic; the wire is uniform.

This is exactly rubix's "block authors never hand-write gRPC"
stance (`run_process_plugin()`), narrowed to the tool trait.

## Mobile safety

`codeless-plugin-host-process` is host-only. Gated behind a Cargo
feature the mobile build does not enable, exactly like
`codeless-adapters-host`. The proto crate (`codeless-tool-proto`)
is mobile-safe (it's just generated message types), but no one in
the mobile-safe dependency graph imports it.

A future mobile shell that wants to "run" process plugins simply
cannot. That's the right answer: iOS doesn't let you `fork`.

## Manifest

```toml
[[runtimes]]
kind     = "process"
binary   = "bin/notes"                  # path under the plugin dir
                                        # supervisor spawns this

[runtimes.policy]
socket_ready_timeout = "5s"
health_interval      = "10s"
failure_threshold    = 3
failure_window       = "60s"
failed_cooldown      = false            # or "60s" for headless retry
```

Same mutually-exclusive rule as the WASM flavour: a plugin declares
at most one *active* runtime per server process. Builtin + WASM +
process can coexist in the manifest as build artefacts; only one is
loaded.

## Acceptance (for the day this lands)

The process flavour is done when:

1. The `notes` plugin builds as a third flavour: same `Tool` source,
   `cargo build` produces `plugins/notes/bin/notes`.
2. `plugin_substrate_e2e::notes_plugin_loads_and_seeds_persona...`
   passes with the runtime flavour swapped to `"process"` via
   config, no test code change.
3. A circuit-breaker integration test
   `plugin_process_e2e::circuit_breaker_trips_after_threshold`
   proves a plugin that crashes on every Call moves to `Failed`
   after `failure_threshold` and stays there until manual `enable`.
4. An identity-check test
   `plugin_process_e2e::stale_binary_rejected` proves a binary that
   reports a `plugin_id` mismatch is refused at `Describe`.
5. A cancellation test
   `plugin_process_e2e::cancel_terminates_slow_call` proves
   `Cancel(call_id)` followed by `HostPolicy::cancel_grace` elapsing
   yields process termination, not a hung supervisor.

## Open questions (record for later)

- **OQ-PROC-1.** Do we ship the proto in a separate `codeless-tool-
  proto` crate, or fold it into `codeless-tools`? **Lean: separate.**
  Non-Rust SDKs need the `.proto` file independently of the Rust
  crate, and a dedicated crate is the cleanest publishing surface.
- **OQ-PROC-2.** Streaming progress events (`Subscribe`) — does the
  Assistant UI render them as a progress card, or as in-place
  updates to the action card? **Defer**; design when a real
  long-running plugin needs it.
- **OQ-PROC-3.** Resource limits on the child (cgroups on Linux,
  job-object quotas on Windows). Rubix has this on the roadmap.
  **Lean: do not implement at first landing.** Process plugins are
  trusted-author in MVP (R5 single tenant); cgroup limits are a
  Phase-7 multi-tenant question.
- **OQ-PROC-4.** Per-process supervisor or one global supervisor
  task with N children? Rubix uses per-process. **Lean: same.** One
  tokio task per plugin, owned by the registry.
- **OQ-PROC-5.** Should the proto live next to `tool.wit` (one
  cross-language contract source) or stay separate? **Lean:
  separate.** They serialise different boundaries — WIT is in-
  process typed FFI, proto is out-of-process gRPC. Conflating them
  would force one to compromise.

## Decisions locked (for the day this lands)

1. **gRPC over UDS, not stdin/stdout JSON.** Streaming, deadlines,
   error model, and code generation across languages are all
   solved by gRPC; reinventing them on stdio is busywork.
2. **Supervisor with circuit breaker, lifted from rubix.** Same
   state machine, same opt-in-sticky `Failed`, same
   `socket_ready_timeout` shape.
3. **`process-wrap` for process-group leadership.** Same crate as
   rubix.
4. **Mobile builds do not include the process host crate.** Cargo
   feature gate, exactly like `codeless-adapters-host`.
5. **Plugin authors never hand-write gRPC.** `run_plugin()` from
   `codeless-plugin-sdk` with `feature = "process"` wraps it; non-
   Rust SDKs are generated from `tool.proto`.
6. **No `Discover` RPC.** Codeless tools are not instance-bearing.
   A plugin that wants to enumerate things exposes a `list` tool.

## Reserve-the-seam (what changes today)

The only thing that changes in MVP, before any of the above lands:

- `plugin.toml` manifest parser **accepts** `[[runtimes]] kind =
  "process"` and strict-validates the policy block.
- A plugin that declares a process runtime but no other runtime is
  marked `Failed` at load with a structured reason (`"process
  runtime not yet supported; declare builtin or wasm or wait for
  process host to land"`).

This way an early plugin can declare a process runtime as a future
target without breaking an MVP install, and `plugin info` can list
the declared-but-unsupported runtime so operators see what's
coming.
