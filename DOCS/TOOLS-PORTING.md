# TOOLS-PORTING — proposal

**Status:** proposal. Not yet accepted into [`SCOPE.md`](./SCOPE.md).
Successor to the withdrawn [`MOXXY-INTEGRATION.md`](./MOXXY-INTEGRATION.md)
(see that doc's "Phase 0 audit findings" for why we are not vendoring
moxxy as crates).

**Author intent:** build codeless's LLM-callable tool surface
(filesystem, shell, HTTP, headless browser, and beyond) in a new
codeless-owned crate `codeless-tools`. Where moxxy already has a good
impl, **port the file** — copy it in, rewrite its `moxxy_core` /
`moxxy_storage` types to use codeless equivalents, own the result.
Where moxxy doesn't help or codeless already has it, write native.
Treat moxxy as a reference implementation, not an upstream — no
patch log, no upstream tracking, no shared types.

If anything below contradicts [`SCOPE.md`](./SCOPE.md), **SCOPE.md
wins** until this proposal is merged into it.

## One-line summary

A new host-only crate `codeless-tools` owns every LLM-callable tool.
Tools reach the LLM through the codeless MCP server. Some tool impls
are ported from moxxy (copied + rewritten); some are native. The line
between "ported" and "native" is decided per file, not per crate.

## Why this instead of vendoring

The [Phase 0 audit](./MOXXY-INTEGRATION.md#phase-0-audit-findings)
established that `moxxy-runtime` cannot be lifted as a crate without
also lifting `moxxy-core` and `moxxy-storage`, which fight codeless
constraints C2 (fresh-session-per-tick) and R4 (single SQLite).

Porting is bounded the way vendoring is not:

- **Bounded blast radius.** Each ported file is a one-time copy with a
  clear rewrite checklist. No patch log to maintain. No upstream
  divergence to track. The work is done when the file compiles.
- **Codeless owns the result.** Once ported, the file is codeless
  code by every standard — same conventions, same comment rules
  ([CLAUDE.md](../CLAUDE.md)), same test discipline. A new contributor
  reading it doesn't need to know it came from moxxy.
- **Per-file decision.** We don't commit to porting all of moxxy. We
  port the files where moxxy's design is better than what we'd write
  from scratch and skip the rest.

Cost paid: a few days per non-trivial tool. Cost avoided: months of
patching a vendored crate against an architecture it wasn't designed
for.

## The new crate

| Crate | Contains | iOS-safe | Android-safe | Notes |
|---|---|---|---|---|
| `codeless-tools` | Every LLM-callable tool impl. Each tool implements the `Tool` trait (see [Tool surface](#tool-surface)). Tools are registered with `codeless-mcp` at runtime startup and exposed as MCP tools to runner subprocesses. | ❌ | ❌ | Host-only. Process spawn, FS access, network calls all live here. Lives at `codeless/crates/codeless-tools/`. |

### Host-only enforcement (R1 teeth)

`codeless-tools`' `Cargo.toml` declares **no default features**.
Everything that requires host capabilities (process spawn, FS, network)
is behind a single workspace-level feature `host` (the same feature
that gates `codeless-adapters-host` today; if that feature is not yet
in workspace `Cargo.toml`, Phase 1 adds it). Mobile-safe crates
(`codeless-types`, `codeless-rpc`, `codeless-client`,
`codeless-tauri-mobile`) do **not** enable `host` and do **not**
declare `codeless-tools` as a dependency at all.

The R1 grep ([SCOPE](./SCOPE.md#r1-crate-dependency-direction-rust))
is extended to cover this crate:

```bash
# No mobile-safe crate may depend on codeless-tools.
grep -E 'codeless-tools' \
  codeless/crates/codeless-types/Cargo.toml \
  codeless/crates/codeless-rpc/Cargo.toml \
  codeless/crates/codeless-client/Cargo.toml \
  codeless/crates/codeless-tauri-mobile/Cargo.toml 2>/dev/null
# must return zero lines
```

CI runs this; non-zero exit blocks merge.

`codeless-tools` does not depend on any `moxxy-*` crate. The earlier
proposal's R6 (a grep rule banning `moxxy_*` imports) is no longer
needed — there are no moxxy crates to import. Ported files have
their `moxxy_core::Foo` rewritten to a codeless-side equivalent, and
that's the end of the linkage.

### Dependency direction

```
codeless-types ← codeless-rpc ← codeless-runtime ← codeless-server
                                       │                  │
                                       │                  │
                                       ▼                  ▼
                              codeless-adapters-host  codeless-mcp
                                                          │
                                                          │ (registers tools at startup,
                                                          │  dispatches MCP calls into them)
                                                          ▼
                                                    codeless-tools
```

Notes on the diagram:

- `codeless-mcp` has a **direct** dep on `codeless-tools` (it owns
  the registration call site). This is the only crate that does.
- `codeless-runtime` does **not** depend on `codeless-tools`. It
  reaches tools transitively through MCP — runner subprocesses make
  MCP calls that land in `codeless-mcp`, which dispatches to
  `codeless-tools`. This keeps `codeless-runtime` free of host-only
  code, matching its row in the [SCOPE crate table](./SCOPE.md#crate-layout-load-bearing-not-aspirational).
- All `host`-feature-gated crates (`codeless-adapters-host`,
  `codeless-mcp`, `codeless-tools`) are compiled by
  `codeless-server` and `codeless-tauri-desktop`. None are compiled
  by `codeless-tauri-mobile`.

### Tool surface

The `Tool` trait is the contract every ported and native tool
implements. It is defined in Phase 1 and frozen as cheaply as
possible afterwards (changes ripple through every tool). The
load-bearing decisions, made now:

```rust
#[async_trait::async_trait]
pub trait Tool: Send + Sync + 'static {
    /// Stable identifier exposed to MCP. Dotted convention
    /// (`browse.fetch`, `browser.click`). Must be unique across the
    /// registry.
    fn name(&self) -> &str;

    /// JSON Schema describing the args object. Used by codeless-mcp
    /// to advertise the tool to runners; runners validate against
    /// it before calling.
    fn schema(&self) -> &serde_json::Value;

    /// Invoke the tool. Async so I/O-bound tools (HTTP, browser,
    /// shell) don't block the executor. Cancellation is delivered
    /// through `ctx`, not via a separate future drop — tools must
    /// poll `ctx.cancelled()` at every await point that could be
    /// load-bearing.
    async fn call(
        &self,
        ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError>;
}
```

Why these choices:

- **Async.** Non-negotiable for Phase 3 (browser sidecar, long-running
  navigation). Inventing it in Phase 1 costs nothing; retrofitting
  it in Phase 3 would touch every tool.
- **JSON in, JSON out.** Matches MCP's wire format directly. No
  per-tool serialization layer to debug.
- **Schema on the trait, not optional.** MCP requires it; runners
  validate against it. Phase 1's port writes a schema as a JSON
  literal in the file; a derive macro is a later refinement, not a
  Phase 1 invention.
- **Cancellation via ctx, not future drop.** Future drop on a tool
  mid-call leaves the runner with no completion event and leaves
  child processes orphaned. Cancellation through `ctx.cancelled()`
  gives the tool a chance to clean up (kill the sidecar, abort the
  HTTP request) and return a structured `ToolError::Cancelled`.

**Streaming output** (e.g. shell stdout chunks, browser screenshot
progress) is deferred. When a tool needs it, we add a second method
(`call_streaming`) that yields `Stream<Item = ToolEvent>`, and
non-streaming tools keep the cheap `call` path. Phase 1's port
(`browse.fetch`) is non-streaming.

### ToolCtx

`ToolCtx` is the per-call context every tool receives. Pinned now
because it's the second-most-reused type in the crate after `Tool`.
Carries:

| Field | Purpose |
|---|---|
| `worktree_root: &Path` | Active job's worktree (from [SCOPE worktree manager](./SCOPE.md)). FS-touching tools resolve relative paths against this. |
| `network_mode: NetworkMode` | The job's network policy (`None`, `Allowlist(file)`, `Open`). Network-touching tools enforce. |
| `allowlist: &AllowlistFile` | Parsed allowlist for the job. Net/HTTP tools consult before egress. |
| `cancel: CancellationToken` | `tokio_util::sync::CancellationToken`. Tools call `cancel.is_cancelled()` or select on `cancel.cancelled()` at await points. |
| `tracing: &Span` | Structured-log span. Tools log into it; correlated with the runner subprocess's task ID. |
| `mcp_session: McpSessionHandle` | Reference to the MCP session that invoked this tool. Used by tools that need to call *back* into the runner (e.g. ask-for-clarification flows). Phase 1's port doesn't use it; included so Phase 2+ doesn't need to reshape `ToolCtx`. |

`ToolCtx` is borrowed for the call's duration — tools do not own it.
It is constructed once per tool invocation in `codeless-mcp`'s
dispatch path.

## How tools reach the LLM

Same seam as the previous proposal: **through `codeless-mcp`.** Each
tool is an MCP tool; runner subprocesses (Claude Code, Codex, etc.)
talk to `codeless-mcp` over stdio or Streamable HTTP and see the
tools as native MCP tools.

Why MCP not inline `ai-runner` patches:

- **Runner-agnostic.** A new runner (Gemini CLI, whatever) gets every
  tool for free with zero `ai-runner` patches.
- **Matches SCOPE.** MCP is already a load-bearing peer surface; this
  reinforces it.
- **Plugins land here too.** When we add a plugin system later (WASI
  or otherwise), plugins register the same way as built-in tools.

Trade-off accepted: each tool call is an MCP round-trip. On coding-
job timescales (tasks measured in seconds-to-minutes) the cost is
noise — an MCP call over stdio is sub-millisecond serialise +
sub-millisecond deserialise plus one process pipe write. If a future
profiler shows it isn't, the seam can be reopened in-process for a
specific runner without disturbing the trait surface.

## Porting policy

A file qualifies for porting if **all three** hold:

1. moxxy's impl handles a real edge case codeless will need (browser
   sidecar lifecycle, allowlist parsing, request redaction, etc.).
2. Building the equivalent native would take longer than porting
   would.
3. The file's reach into `moxxy_core` / `moxxy_storage` is shallow
   enough that the rewrite checklist (below) is one engineer-day or
   less.

If any of the three fails, write native. Build only what codeless
needs, not what moxxy happens to ship.

### Per-port rewrite checklist

Every ported file:

1. **Replace `moxxy_core::*` and `moxxy_storage::*` imports** with
   codeless equivalents. Where no equivalent exists yet, write the
   minimum codeless-side type to satisfy this file (and add a note
   so a follow-up port can reuse it).
2. **Strip moxxy-specific concepts.** Agent identity, scoped vault,
   per-agent paths — none of these exist in codeless. Replace with
   single-tenant equivalents ([SCOPE R5](./SCOPE.md#r5-single-tenant-trust-boundary)).
3. **Re-style.** Match codeless conventions:
   [CLAUDE.md](../CLAUDE.md) on comments (why-only, no task-status,
   no decorative banners), one concept per file.
4. **Re-test.** Move moxxy's tests for this file into the port if
   they make sense in the codeless context; rewrite or delete those
   that don't. Every ported tool ships with at least one codeless
   integration test in `codeless/crates/codeless-tools/tests/`,
   driven from a "fake task" — a `ToolCtx` constructed by the test
   harness (`codeless-tools::testing::fake_ctx()`, written in
   Phase 1 sub-tick T2) with a tempdir worktree, an in-memory
   allowlist, and a cancellation token the test controls. No real
   LLM, no real runner subprocess, no real MCP server. The test
   calls `tool.call(&ctx, args).await` directly.
5. **Log provenance.** A single line at the top of each ported file:
   `// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/primitives/<file>`
   then nothing else from moxxy in the file header. This is for
   humans navigating the codebase; it carries no legal weight.
6. **License preservation.** MIT requires the copyright notice and
   permission text be preserved "in all copies or substantial
   portions." A port is a substantial portion. We satisfy this with
   a single workspace-level `NOTICE` file (see below) that
   reproduces moxxy's MIT notice in full. The per-file provenance
   line points at moxxy; the NOTICE file carries the legal text.
   This is the standard split — Rust ecosystem projects like
   tokio-rs and serde-rs use the same pattern.

Codeless's own license is **not yet decided** (see [Open question
1](#open-questions)). The relicensing question doesn't bite until
that decision lands — until then, ported files inherit moxxy's MIT
implicitly via NOTICE preservation and the workspace having no
declared license of its own. The day codeless picks a license, the
proposal that introduces it must state how it relates to incoming
MIT ports (almost certainly: codeless's license applies to codeless-
authored code; ported files keep MIT via NOTICE; both coexist).

### NOTICE file placement

`NOTICE` lives at **`codeless/NOTICE`** — inside the inner repo, not
the workspace. The inner repo is what ships as binaries and what
downstream sees; the workspace is a build harness. Phase 1 sub-tick
T4 creates this file. Contents: header line ("This product includes
software developed by third parties. See entries below."), one
section per upstream, each section reproducing the upstream's MIT
text in full plus a line naming which files we derived from it.

## Phase plan

This is incremental on purpose. Each phase ends with code that
compiles and tests that pass. No phase is a prerequisite for the next
in terms of scope — the next phase happens because we want more
tools, not because the current phase is incomplete.

### Phase 1 — `codeless-tools` skeleton + one ported tool

Goal: prove the porting pattern works end to end with the smallest
possible surface.

**Tick sizing:** `L` per [JOB-LOOP](./JOB-LOOP.md), split into five
sub-ticks. Each sub-tick ends with `cargo check` green and a
reviewable commit. They run in order; T2 cannot land before T1.

| Sub-tick | Size | Scope | Done when |
|---|---|---|---|
| **T1** | S | New crate `codeless/crates/codeless-tools/` as workspace member. `host` Cargo feature added to workspace `Cargo.toml` if not present. Empty `lib.rs` with crate-level docs. Workspace builds. | `cargo check -p codeless-tools` green; R1 grep returns zero lines. |
| **T2** | M | `Tool` trait, `ToolCtx`, `ToolError`, registration mechanism, `codeless-tools::testing::fake_ctx()` test harness. No tool impls yet. | Trait compiles; harness usable from a unit test; doc comments explain every field of `ToolCtx`. |
| **T3** | M | `codeless-mcp` integration. `codeless-mcp` declares a dep on `codeless-tools`, accepts a `ToolRegistry` at startup, exposes registered tools as MCP tools, dispatches MCP calls into `tool.call(&ctx, args)`. | Existing `codeless-mcp` tests still pass; new integration test wires a no-op tool and verifies the MCP advertise + dispatch round-trip. |
| **T4** | M | Codeless-side `NetworkMode` and `AllowlistFile` types in `codeless-tools` (probably under a `policy` module). `codeless/NOTICE` created with moxxy's MIT notice. `SCOPE.md` crate table updated to add `codeless-tools`; R1 enforcement note added. | Both types unit-tested in isolation; NOTICE present; SCOPE.md PR-ready. |
| **T5** | M | Port `browse.fetch` from `moxxy-runtime/src/primitives/browse.rs` (the `fetch` primitive only — not the HTML extractor, not the JS-rendering browser). Provenance comment, MIT preserved via NOTICE, schema declared inline. At least one integration test using the T2 harness. ai-runner naming collision resolved (see [ai-runner overlap](#ai-runner-overlap) below). | Tool callable end-to-end through the MCP seam in a test; `cargo check -p codeless-server` green. |

The split exists so a sub-tick failure halts narrowly. T2 failing
(trait design wrong) doesn't waste T3 work. T5 failing (port harder
than expected) is a single-file undo, not a phase rollback.

Why `browse.fetch` for T5, not the headless browser:

- Headless browser carries a Playwright sidecar — a separate Node.js
  process to supervise, lifecycle to manage, IPC to design. Wrong
  forcing function for "validate the porting pattern."
- Shell is tempting but ai-runner already exposes shell; the port
  doesn't earn its keep.
- `browse.fetch` is small (~200 LOC), useful (coding jobs fetch docs,
  preview URLs, gh API responses), and forces us to build the
  network-mode and allowlist primitives that every later port
  will reuse.

#### ai-runner overlap

`ai-runner` already exposes some tools (read_file, edit_file, shell,
and Claude Code's built-in `WebFetch`) to the runner subprocess
directly, without going through MCP. Once `codeless-tools` lands,
the same surface is reachable a second way (MCP). Phase 1's port
(`browse.fetch`) sits next to Claude Code's `WebFetch`. Two
considerations:

1. **Naming.** Use a dotted, codeless-prefixed name to avoid
   collision: `codeless.browse.fetch` rather than `browse.fetch` or
   `fetch`. Claude Code's `WebFetch` stays as-is; the codeless tool
   is reachable to runners that don't have a native equivalent
   (Codex, future runners) and to MCP clients that want codeless's
   specific allowlist behaviour.
2. **Migration of native-runner tools.** Moving ai-runner's existing
   `read_file`/`edit_file`/`shell` into `codeless-tools` is a
   **separate proposal** (touches ai-runner patches, runner
   subprocess configuration, every running job). Out of scope for
   this doc. Phase 1 does *not* migrate them. Phase 1's port lives
   alongside them.

The end-state — single tool registry, runners pick from it via MCP
only — is a goal, not a Phase 1 deliverable. Phase 1 establishes the
crate so a future migration has somewhere to migrate *to*.

### Phase 2 — second port (shell or HTTP-with-allowlist)

Goal: shake out the abstractions Phase 1 invented under load.

The abstractions written in Phase 1 (`NetworkMode`, `AllowlistFile`,
`ToolCtx`) were designed for one caller. The second port is where we
find out which of them were over-specified to that caller, under-
specified, or just wrong. Pick the second port to *force* that
exercise:

- If Phase 1's allowlist abstraction was the hard part: port
  `http.rs` next (heavier allowlist use, redaction, more HTTP
  knobs).
- If Phase 1's `ToolCtx` was the hard part: port `shell.rs` (path
  policy, network mode, working directory — different `ToolCtx`
  shape, will surface gaps).

Choice is made when Phase 1 lands, based on what hurt. Don't predict
which it'll be.

**Tick sizing:** `M`. One tick. Smaller than Phase 1 because the
abstractions exist.

### Phase 3 — headless browser

Goal: the prize.

Port `primitives/browser/` (4 files: `core.rs`, `crawl.rs`,
`interact.rs`, `session.rs`) plus the Playwright sidecar harness.
Gate behind a Cargo feature so the minimum codeless deploy stays
Rust-only.

This is the largest port and the riskiest. By Phase 3 the
abstractions are stable (Phase 1 invented them, Phase 2 tested them)
and the team has a feel for moxxy's code shape, so this is the right
order. Doing it first would mean inventing the abstractions in the
middle of a 1000-LOC port, which is exactly how vendoring proposals
get stuck.

**Tick sizing:** `L`. Multiple ticks, each landing a sub-piece
(sidecar lifecycle → navigate/read → screenshot/eval → interact →
crawl). Each tick green on its own.

### Phase 4+ — opportunistic ports

After Phase 3, ports happen as the product needs them. No fixed list.
Candidates worth eyeing: `git.rs` (blame, log queries), webhook
emission, the skill loader (if/when we want one).

## What lives in `codeless-tools` from day one — native, not ported

Some tools are easier to write than to port:

- **Worktree manager bridge.** Codeless already owns this in
  `codeless-adapters-host`; `codeless-tools` exposes it as a tool
  but delegates to the existing impl.
- **The codeless-native primitives ai-runner already wires up**
  (read_file, edit_file, etc.). These move from "implicit ai-runner
  tools" to "explicit codeless-tools entries" at some point — but
  *not* in Phase 1. That migration is a separate proposal because
  it touches ai-runner's tool surface.

Don't conflate "port from moxxy" with "build the codeless-tools
crate." The crate hosts both kinds of tool.

## Risks acknowledged

- **The first port will be slow.** Inventing the codeless-side
  `NetworkMode` / `AllowlistFile` abstractions while porting one
  file is harder than porting a file in a crate that already has
  those abstractions. Budget Phase 1 generously; the speedup shows
  up in Phase 2.
- **Moxxy will evolve. We won't follow.** A bug fix upstream is not
  ours unless someone reads moxxy's changelog and decides to apply
  it by hand. Accept this. The whole point of porting over vendoring
  is that we *don't* track upstream.
- **License hygiene must not slip.** The NOTICE file is small but
  load-bearing. If we port a file and forget to add moxxy to NOTICE,
  we have a licensing problem. Phase 1's sub-tick order is set so
  T4 (NOTICE + policy types) lands strictly before T5 (the first
  port). For Phase 2+ ports, the rewrite checklist's "License
  preservation" step is mandatory CI: a port that adds a
  `// ported from` comment without a matching NOTICE entry should
  fail the build.
- **"Port vs native" is a judgment call per file.** That judgment can
  be wrong. If a port turns out to need more rewriting than a from-
  scratch implementation would have, throw it out and write native.
  No sunk-cost.

## Decisions

1. **NOTICE file.** Single `codeless/NOTICE` file in the inner repo,
   plain MIT text per upstream. See [NOTICE file placement](#notice-file-placement).
2. **Tool registration is tool-led.** `codeless-mcp` accepts a
   `ToolRegistry` constructed by the host binary (`codeless-server`,
   `codeless-tauri-desktop`). The host calls `codeless-tools`'s
   builder, which pushes built-in tool registrations into the
   registry. Reason: matches how plugins will work later (a plugin
   loader pushes its tools the same way), and keeps `codeless-mcp`
   ignorant of which tools exist. Server-led pull would force
   `codeless-mcp` to know about plugin discovery, which is the wrong
   direction.
3. **Tool naming convention.** Dotted, codeless-prefixed:
   `codeless.browse.fetch`. The `codeless.` prefix prevents
   collisions with native runner tools (e.g. Claude Code's
   `WebFetch`); the dotted form keeps reading parity with moxxy
   source.

## Open questions

1. **Codeless's overall license.** Not decided. Until decided,
   ported MIT files live in the codebase under MIT via the NOTICE
   file and the workspace has no declared license of its own. The
   proposal that picks codeless's license must state how it relates
   to incoming MIT ports. Not blocking for Phase 1 — the NOTICE
   file does the legal work today, regardless of what codeless picks
   later.
2. **Status of the predecessor doc.** [`MOXXY-INTEGRATION.md`](./MOXXY-INTEGRATION.md)
   stays in the tree, marked SUPERSEDED, as the source of truth for
   the Phase 0 audit findings. This is *not* an open question in
   the live-design sense — it is a documentation choice — but
   flagged here because the predecessor doc is linked from this doc
   for the audit content, so it cannot be deleted without breaking
   those links.

## Pointers

- Predecessor (rejected): [`MOXXY-INTEGRATION.md`](./MOXXY-INTEGRATION.md) — especially its [audit findings](./MOXXY-INTEGRATION.md#phase-0-audit-findings).
- Scope this layers on top of: [`SCOPE.md`](./SCOPE.md)
- Loop design preserved: [`LOOP-CODER.md`](./LOOP-CODER.md)
- Upstream moxxy (reference only, not tracked): https://github.com/moxxy-ai/moxxy
