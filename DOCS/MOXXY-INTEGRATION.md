# MOXXY-INTEGRATION — proposal (SUPERSEDED)

**Status:** **SUPERSEDED on 2026-05-13** by
[`TOOLS-PORTING.md`](./TOOLS-PORTING.md). The Phase 0 spike below
fired kill criteria K1 and K2; the vendoring approach in this doc is
not viable. The successor proposal ports specific files from moxxy
into a new codeless-owned crate (`codeless-tools`) rather than
vendoring `moxxy-runtime` as a crate.

**See also:** the MCP / agent-driven half of what this proposal would
have covered is now [`AGENT-CONTROL-PLANE.md`](./AGENT-CONTROL-PLANE.md)
— the concrete plan for exposing job operations (create / debug /
status / control) through MCP, REST, and CLI against the existing
`codeless-server` daemon, with no in-process runtime per agent. Anyone
who came here looking for "how do Claude / Codex / Copilot drive
codeless jobs?" should read that doc.

This doc is kept, not deleted, because the audit findings are useful
context: anyone asking "why didn't we just fork moxxy?" should read
the [Phase 0 audit](#phase-0-audit-findings) before re-opening the
question.

**Author intent:** vendor a curated subset of [moxxy-ai/moxxy](https://github.com/moxxy-ai/moxxy)
into codeless so coding jobs gain a large tool surface (filesystem,
git, shell, HTTP, headless browser; moxxy claims 940+ tests, to be
verified in the spike) and a real third-party extension story (WASI
plugins), without inheriting moxxy's agent loop, swarm orchestration,
channels, or HTTP gateway — all of which fight codeless's existing
scope.

If anything below contradicts [`SCOPE.md`](./SCOPE.md), **SCOPE.md
wins** until this proposal is merged into it.

## One-line summary

Vendor moxxy's `moxxy-runtime` (the 85 primitives) and `moxxy-plugin`
(the WASI plugin host) into `codeless-workspace/moxxy/`, patched in
place, exposed to the codeless runtime through a new host-only crate
`codeless-primitives`. Delete moxxy's agent loop, swarm, channels,
gateway, MCP server, and SQLite layer.

## Why this scope (the constraints that pick what we keep)

The whole point of this proposal is that **moxxy and codeless solve
different problems** but moxxy already built three pieces codeless
needs and would otherwise have to build from scratch.

Codeless's load-bearing constraints from
[`SCOPE.md`](./SCOPE.md#why-this-scope-the-key-constraints):

- **C1** — many concurrent coding jobs across many repos from a
  browser. Headless Rust core, SQLite as truth.
- **C2** — coder loop must run unsupervised for hours. Fresh session
  per tick, handover.md as memory. See [`LOOP-CODER.md`](./LOOP-CODER.md).

Moxxy's load-bearing constraints (inferred from its README and crate
split):

- **M1** — many agents, each with private workspace, memory, and
  scoped vault. Agents run autonomously.
- **M2** — agentic execution loop drives the LLM, with stuck detection
  and recovery. Sub-agent spawning. Hive swarm orchestration.

C2 and M2 are **mutually exclusive**. You cannot have "fresh session
per tick, no in-memory continuity" *and* "agentic loop with stuck
detection and recovery state" in the same binary. One of them has to
go, and for codeless it's M2.

What survives that filter is exactly the moxxy parts that don't carry
loop state: **primitives** and **plugin host**.

## What we keep, what we drop, why

| Moxxy piece | Decision | Why |
|---|---|---|
| `moxxy-runtime` primitives (fs, git, shell, HTTP, headless browser, memory, webhooks, MCP, skills) | **Keep** | Pure tool surface. Each primitive is "called with args, returns a result." No loop assumptions. Exactly what codeless tasks invoke. |
| `moxxy-plugin` (WASI plugin host, capability-scoped permissions) | **Keep** | Gives codeless a third-party extension story we don't have today and don't want to design from scratch. |
| `moxxy-vault` (OS-keychain backend, scoped secrets) | **Keep partial** | Reuse the keychain backend; drop the per-agent scoping model. Codeless is single-tenant ([SCOPE R5](./SCOPE.md#r5-single-tenant-trust-boundary)). |
| `moxxy-core` (agent loop, stuck detection, recovery, sub-agent spawning) | **Drop** | Fights LOOP-CODER's fresh-session-per-tick model. Cannot coexist. |
| Hive swarm orchestration (queen/worker, task boards, voting) | **Drop** | Competes with codeless's `Repo → Job → Stage → Task`. Two scheduling models in one binary is a maintenance bomb. |
| `moxxy-channel` (Discord, Telegram, email, voice) | **Drop** | Out of scope for MVP. Revisit in Phase 7+ if "notify me on Telegram when a stage hits review" becomes real demand. |
| `moxxy-gateway` (Axum HTTP + SSE server) | **Drop** | `codeless-server` already owns the HTTP/SSE surface. |
| `moxxy-mcp` (MCP server binary) | **Drop** | `codeless-mcp` already owns the MCP surface, and parity with the CLI is enforced in CI ([SCOPE.md](./SCOPE.md)). |
| `moxxy-storage` (SQLite WAL backend) | **Drop** | [SCOPE R4](./SCOPE.md#r4-sqlite-is-the-source-of-truth) — codeless owns the SQLite schema. Two schemas in one binary is forbidden. |
| Moxxy's TUI / Node CLI / interactive wizards | **Drop** | Codeless owns its surfaces. |

## End-state architecture

### Repo layout (additions only)

The vendored moxxy code lives at the **workspace** level (alongside
`ai-runner/`). The new bridge crate `codeless-primitives` lives
**inside the inner `codeless/` repo** as a workspace member, alongside
the other `codeless-*` crates. This is the same split as today: vendored
upstream code sits in the outer workspace, codeless-owned crates sit
in the inner repo.

```
codeless-workspace/
├── ai-runner/                ← existing, vendored, patched
├── ai-runner.PATCHES.md      ← existing
├── moxxy/                    ← NEW, vendored from moxxy-ai/moxxy
│   ├── crates/
│   │   ├── moxxy-runtime/    ← the primitives
│   │   ├── moxxy-plugin/     ← WASI plugin host
│   │   └── moxxy-vault/      ← keychain backend only (scoping stripped)
│   └── (moxxy-core, moxxy-channel, moxxy-gateway, moxxy-mcp,
│        moxxy-storage are deleted at vendor time, not kept and ignored)
├── moxxy.PATCHES.md          ← NEW, same model as ai-runner.PATCHES.md
└── codeless/                 ← inner repo
    └── crates/
        └── codeless-primitives/  ← NEW, bridge crate (codeless-owned)
```

Same vendoring model as `ai-runner/`: no inner `.git`, every codeless-
side edit lands a row in `moxxy.PATCHES.md` with a
`// codeless-patch-NNN` source marker. Upstream is **not tracked** —
this is a one-way copy.

### New crate

One new crate is added to the [SCOPE.md crate layout](./SCOPE.md#crate-layout-load-bearing-not-aspirational):

| Crate | Contains | iOS-safe | Android-safe | Notes |
|---|---|---|---|---|
| `codeless-primitives` | Wraps `moxxy-runtime` and `moxxy-plugin`. Exposes each primitive behind a codeless-native trait (`Primitive::call(ctx, args) -> Result<Value>`). Owns WASI plugin loading and capability enforcement. The single seam through which codeless tasks reach moxxy code. | ❌ | ❌ | Host-only. Process spawn and FS access live here, gated behind the same Cargo feature as `codeless-adapters-host` (see [SCOPE R1](./SCOPE.md#r1-crate-dependency-direction-rust)). Lives inside `codeless/crates/codeless-primitives/`. |

Dependency direction (additions are the two host-only nodes; both are
Cargo-feature-gated identically to `codeless-adapters-host`):

```
codeless-types ← codeless-rpc ← codeless-runtime
                                       ↑
                                       │ (trait calls)
                                       │
                       codeless-adapters-host
                       codeless-primitives  ← moxxy-runtime,
                                              moxxy-plugin,
                                              moxxy-vault (keychain only)
```

`codeless-runtime` calls `codeless-primitives` through a trait; it
never imports moxxy types directly. This is the same discipline used
for `codeless-adapters-host` today.

### Integration seam — how primitives reach the LLM

Primitives are exposed to coding-runner subprocesses (Claude Code,
Codex, Copilot CLI, future runners) **through the codeless MCP
surface**, not by patching each runner's tool list inline.

The path:

1. `codeless-primitives` registers each primitive (and each loaded
   WASI plugin) with `codeless-mcp` at runtime startup.
2. `codeless-mcp` exposes them as MCP tools alongside its existing
   codeless-native tools.
3. When a coding job starts, its runner subprocess is configured to
   talk to `codeless-mcp` over stdio (or Streamable HTTP for remote
   runs). The runner sees the primitives as ordinary MCP tools.
4. Tool calls come back to `codeless-primitives` via the MCP server
   and dispatch to moxxy code.

Why MCP and not the ai-runner tool list:

- **Runner-agnostic.** A new runner (Gemini CLI, whatever) gets the
  full primitive surface for free with no `ai-runner` patch per
  primitive. Adding a primitive is one commit in one crate.
- **Matches SCOPE.** The MCP server is already a load-bearing peer
  surface ([SCOPE.md](./SCOPE.md) — CLI/MCP/UI parity is enforced in
  CI). Routing primitives through it reinforces that surface rather
  than building a parallel inline path.
- **WASI plugins land in one place.** A user-installed `.wasm` plugin
  becomes an MCP tool the same way a built-in primitive does. No
  per-runner plugin-discovery code.

The trade-off accepted: each tool call is an MCP round-trip rather
than an in-process call. On coding-job timescales (seconds-to-minutes
per task) this is noise. If a future profiler shows it isn't, the
seam can be reopened in-process for a specific runner without
disturbing the trait surface.

### New hard rule

This proposal adds one rule to [SCOPE.md](./SCOPE.md#hard-rulesviolating-any-of-these-halts-work):

**R6 — Moxxy types are crate-local to `codeless-primitives`.**
No other crate may import from `moxxy-*` directly. Everything goes
through the codeless-native primitive trait. This keeps the vendor
boundary maintainable: when we patch moxxy, the blast radius is one
crate, not the whole runtime.

Enforceable by grep. The pattern covers `use`, `extern crate`, and
mid-expression qualified paths (`moxxy_runtime::foo()`); re-exports
are caught by the `pub use` form of the same pattern:

```bash
grep -rE '(\<use\s+|\<pub\s+use\s+|\<extern\s+crate\s+|\bmoxxy_[a-z_]+::)moxxy_' \
  --include='*.rs' codeless/ \
  | grep -v 'codeless/crates/codeless-primitives/'
# must return zero lines
```

CI runs this; a non-zero exit blocks merge.

### Surface area additions

Coding jobs gain the following primitive families (subject to Phase 0
audit confirming each is extractable):

- **Filesystem** — read, write, glob, watch (codeless has its own
  worktree FS access today; moxxy's adds breadth)
- **Git** — beyond worktree management (which codeless owns):
  blame, log queries, diff parsing
- **Shell** — sandboxed execution with output capture
- **HTTP** — outbound requests with redaction
- **Headless browser** — supervised Playwright sidecar. **The
  highest-value single piece** — building it from scratch is weeks.
- **Memory store** — short-term scratch usable inside a single task
  (not a replacement for handover.md across ticks; see LOOP-CODER)
- **Webhooks** — outbound only in MVP
- **MCP client** — codeless tasks can call other MCP servers as tools.
  Note the direction: codeless tasks act as MCP *clients* here, while
  `codeless-mcp` itself is an MCP *server*. The two roles share no
  code; they're peer concerns.

WASI plugins extend this list at runtime: a user drops a `.wasm` file
in the global plugin directory (see [Decisions](#decisions) below),
declares its capabilities in a manifest, and it appears as a callable
primitive in every coding job — exposed through the same MCP seam as
built-in primitives.

### What this does not change

- `Repo → Job → Stage → Task → Review` state machine — untouched.
- Fresh-session-per-tick loop ([LOOP-CODER](./LOOP-CODER.md)) — untouched.
- The two-tier scheduler (job-queue scheduler + session scheduler) —
  untouched.
- SQLite schema — untouched. Codeless owns it.
- The `RpcClient` trait and the one-React-UI-for-four-shells rule —
  untouched.
- All five existing R-rules — untouched.

The new R6 is additive.

## Phase 0 — the spike (mandatory before vendoring)

This proposal cannot be accepted without first answering one question:
**can `moxxy-runtime` (and `moxxy-plugin`) compile after we delete
`moxxy-core`, `moxxy-storage`, `moxxy-channel`, `moxxy-gateway`, and
`moxxy-mcp`?**

If yes, the plan above is straightforward — a long vendoring exercise
but no architectural surprises. If no, the answer flips to one of two
fallbacks (see [Phase 0 fallbacks](#phase-0-fallbacks) below); the
default is "drop this proposal entirely."

### Day budget

**One engineer, one day.** No vendoring, no commits to `codeless-workspace/`
during the spike. Output is a written audit (added as a section to
this doc) and a go/no-go recommendation.

### What to read

1. `moxxy/crates/moxxy-runtime/Cargo.toml` — list every internal
   moxxy-* dependency. The cheap ones are `moxxy-types` analogs
   (pure-data crates). The expensive ones are `moxxy-core` and
   `moxxy-storage`.
2. `moxxy/crates/moxxy-runtime/src/lib.rs` and module roots — grep for
   `use moxxy_core::`, `use moxxy_storage::`, `use moxxy_channel::`.
   Count how many primitive impls reach into the agent loop or the
   storage layer.
3. `moxxy/crates/moxxy-plugin/Cargo.toml` and lib.rs — same exercise.
   WASI hosts often reach into the agent loop for capability checks;
   if moxxy-plugin's capability model is tangled with moxxy-core's
   agent identity, the cost goes up.
4. `moxxy/crates/moxxy-vault/Cargo.toml` and lib.rs — confirm the
   keychain backend can be lifted without the scoping model.
5. Tests — moxxy claims 940+ tests. Identify which test suites cover
   only `moxxy-runtime` and `moxxy-plugin`. Those are the ones we keep
   and run in codeless CI.

### Green-light criteria (all must hold)

- **G1** — every internal moxxy-* dependency of `moxxy-runtime` is
  either (a) on the keep list (`moxxy-plugin`, `moxxy-vault`) or (b)
  a pure-data crate with no async runtime, no I/O, no SQLite, no
  channel/gateway types. The count doesn't matter; the *kind* does.
- **G2** — `moxxy-plugin`'s capability model can be reduced to a
  static manifest check, with no reach into moxxy-core's agent
  identity.
- **G3** — `moxxy-vault`'s keychain backend module compiles in
  isolation after the scoping types are stripped.
- **G4** — Three required primitive families compile in the spike
  branch after the deletions: **filesystem**, **shell**, **headless
  browser**. Headless browser is required because the proposal names
  it the highest-value single piece (see Surface area additions);
  losing it makes the whole vendor unattractive. Git, HTTP, memory,
  webhooks, and MCP-client are bonus families — nice if they survive
  the spike, not blockers.
- **G5** — Moxxy is licensed under MIT, Apache-2.0, BSD-3-Clause, or
  a dual-license that includes one of those (confirmed via the
  `LICENSE` file in repo root and `Cargo.toml` `license` fields).
  Verify before vendoring; if it's AGPL, GPL, SSPL, or has a
  non-OSI custom clause, this whole proposal is dead.

### Kill criteria (any one kills the proposal)

- **K1** — `moxxy-runtime` transitively requires `moxxy-storage`'s
  SQLite schema for primitive bookkeeping (e.g. "every tool call
  writes to a moxxy table"). That schema cannot coexist with codeless
  SQLite ([R4](./SCOPE.md#r4-sqlite-is-the-source-of-truth)).
- **K2** — `moxxy-plugin`'s WASI host expects the moxxy agent loop
  to be running (capability checks ask "what is the current agent's
  scope?"). Stripping that is a rewrite, not a patch.
- **K3** — The headless browser primitive is tangled with
  `moxxy-channel` or `moxxy-gateway` (e.g. results are pushed through
  channels rather than returned synchronously). The single
  highest-value piece becomes the most expensive to extract.
- **K4** — License is incompatible.

If green-lit, the spike writer **appends an "Audit findings" section
to this doc** and changes status to "accepted." Then vendoring proceeds
under the normal JOB-LOOP rules.

If killed, this doc is deleted and a short note is added to
[`SCOPE.md`](./SCOPE.md) under "Considered and rejected" explaining
which K-criterion fired.

### Phase 0 fallbacks

If Phase 0 fails its green-light criteria, two fallbacks exist. The
default is **(B) drop the proposal**; (A) is documented so the choice
is conscious, not reflexive.

**(A) Moxxy as a sidecar process.** Run unmodified upstream moxxy as
a separate process; `codeless-primitives` becomes a thin client of
moxxy's HTTP/SSE gateway.

- Gets us: the primitive surface, no source coupling, no patch log,
  free upstream updates.
- Loses us: the WASI plugin *trust boundary* (plugin permissions are
  enforced by moxxy's identity model, not codeless's), one extra
  process to supervise, two SQLite files (moxxy's + codeless's),
  IPC latency on every tool call, and a deploy story that requires
  shipping the moxxy binary separately.
- When to pick: only if K1 or K2 fires and the headless browser
  primitive is *individually* worth the operational cost. Probably
  not.

**(B) Drop the proposal entirely.** Build the few primitives codeless
actually needs (probably: shell, fs, HTTP — already mostly present
via ai-runner) as codeless-native code. Skip WASI plugins for MVP.

- Gets us: a smaller, simpler codeless; no upstream surface area at
  all beyond ai-runner.
- Loses us: the headless browser (rebuild from scratch, weeks), the
  WASI extension story, the breadth of moxxy's primitive set.
- When to pick: default for any spike failure. Easy to revisit later
  if a specific primitive becomes a real bottleneck.

## Phase 0 audit findings

**Date:** 2026-05-13. **Auditor:** automated spike, results
hand-verified.
**Upstream commit:** `master` HEAD at time of audit (shallow clone,
SHA not recorded in this session).
**Method:** clone moxxy to `/tmp/moxxy-spike/moxxy`, read `Cargo.toml`
+ `lib.rs` for each candidate keep crate, grep for cross-crate symbol
use. No vendoring performed.

### Verdict

**HALT.** Two kill criteria fire (**K1** and **K2**) plus G1 fails on
inspection. The proposal as written is not viable.

### G5 — license check (PASSED)

- `LICENSE-MIT` present in repo root, standard MIT text, "Copyright (c)
  2026 Moxxy Contributors."
- No `license = ` field in workspace `Cargo.toml` — recommend asking
  upstream to add one if any revised proposal proceeds, but not
  blocking.
- **G5 holds.** License is compatible.

### G1 — moxxy-runtime internal deps (FAILED)

`moxxy-runtime/Cargo.toml` declares hard `[dependencies]` on:

| Dep | On keep list? | Pure-data? | Verdict |
|---|---|---|---|
| `moxxy-types` | yes (implicit, would need to add) | yes | OK |
| `moxxy-core` | **no — explicit drop** | **no** (event bus, embeddings, settings, allowlist store, skill loader, webhook loader, heartbeat scheduler) | **FAIL** |
| `moxxy-storage` | **no — explicit drop** | **no** (SQLite DAOs and row types) | **FAIL** |
| `moxxy-vault` | yes (keychain-only keep) | n/a (transitive Database use) | conditional |
| `moxxy-mcp` | **no — explicit drop** | **no** (McpManager, config loader) | **FAIL** |

Three of the five internal deps are on the explicit drop list, and
each is used as a non-trivial type (not just a re-export). G1 says
every dep must be on the keep list or pure-data; three are neither.

### K1 — moxxy-runtime requires moxxy-storage's SQLite schema (FIRES)

`moxxy-runtime` reaches directly into `moxxy_storage::Database` and
named row types in at least these primitive impls:

- `primitives/session.rs` — `Database`, `SessionSummaryRow`
- `primitives/memory_ltm.rs` — `Database`, `MemoryIndexRow`,
  `AgentRow` (test)
- `primitives/vault.rs` — `Database` (test)
- `primitives/webhook.rs` — `Database`, `AgentRow`
- `context.rs` — `Database`, `VaultSecretRefRow`, `VaultGrantRow`
- `executor/reflection.rs` — `Database`, `MemoryIndexRow`,
  `SessionSummaryRow`
- `agent_kind/mod.rs` — `Database`

These are not bookkeeping calls that we could route through a trait —
they are concrete `rusqlite`-backed row types. **K1 fires:**
`moxxy-runtime` cannot compile without `moxxy-storage`, and
`moxxy-storage`'s schema cannot coexist with codeless's SQLite ([SCOPE
R4](./SCOPE.md#r4-sqlite-is-the-source-of-truth)).

### K2 — moxxy-plugin transitively pulls the agent loop (FIRES)

`moxxy-plugin/Cargo.toml` declares `moxxy-runtime = { path = ... }`
as a hard dependency. moxxy-plugin's source is clean (no direct
`moxxy_core` or `moxxy_storage` use), but its compile graph requires
`moxxy-runtime`, which in turn requires the entire agent-loop world
above.

The capability model in moxxy-plugin is not the issue (which is what
K2 originally feared). The issue is structural: the plugin host
cannot be lifted without lifting `moxxy-runtime`'s full dep set with
it. **K2 fires in a different shape than written.** Updating the K2
text in any revised proposal: "moxxy-plugin's compile graph requires
moxxy-runtime, and moxxy-runtime cannot be detached from moxxy-core +
moxxy-storage."

### K3 — headless browser entanglement (NOT EVALUATED, moot)

K1 + K2 are sufficient to halt. Did not finish the K3 audit (browser
primitive lives at `moxxy-runtime/src/primitives/browser/` as a clean
submodule; on first read it looks tractable in isolation, but it
shares `moxxy_core::NetworkMode` and `moxxy_core::AllowlistFile`
gates with other primitives so it inherits the K1 problem).

### G2, G3, G4 — not evaluated (moot)

Once K1 and K2 fire, the question is moot. Not worth the time.

### Surface area moxxy-runtime depends on from moxxy-core

For the revised-proposal conversation, here is the full set of
`moxxy_core::` symbols used by `moxxy-runtime`:

- `EventBus`, `EmbeddingService`, `MockEmbeddingService`,
  `NetworkMode`, `PathPolicy`
- `AllowlistFile`, `allowlist_path`, `settings_path`, `heartbeat_path`
- `LoadedSkill`, `SkillSource`, `LoadedWebhook`, `WebhookLoader`,
  `HeartbeatEntry`
- `SttError`, `SttProvider`

This is not a thin border. It's threaded through every primitive
family. A "patch out moxxy_core" exercise would touch dozens of
files and rewrite the path-policy, allowlist, event-bus, embedding,
and webhook subsystems — at which point we have rewritten
moxxy-runtime, not vendored it.

### Recommendation

Three live options. Bringing this back to the human (per spike
instructions) rather than picking one.

1. **Fallback (B) — drop the proposal entirely.** Default. Build the
   few primitives codeless actually needs (shell, fs, HTTP) as
   codeless-native code; skip WASI plugins for MVP; if the headless
   browser turns out to be load-bearing, build a minimal Playwright
   bridge from scratch when the need is concrete. Costs weeks of
   browser-bridge work *if* we end up wanting it; costs nothing today.
2. **Fallback (A) — moxxy as a sidecar.** Run unmodified upstream
   moxxy as a separate process, talk to it over its HTTP/SSE gateway.
   Gets us the full primitive surface and the headless browser without
   any vendor coupling, at the cost of: two processes to supervise,
   two SQLite files, IPC on every tool call, and a deploy story that
   ships moxxy as a separate artefact. The WASI plugin trust boundary
   becomes moxxy's, not ours.
3. **Revised proposal — vendor only the leaf primitives we want.**
   Don't try to vendor `moxxy-runtime` as a crate. Instead, identify
   the specific primitive impls we want (e.g. the `browser/` subtree,
   maybe `shell.rs`, maybe `git.rs`) and **port them, not vendor
   them** — copy the file, replace `moxxy_core` and `moxxy_storage`
   types with codeless equivalents (we already have an event bus,
   allowlist, path policy in adapters-host), and own the result.
   Treat moxxy as a reference implementation, not an upstream. This
   is a meaningful amount of work but it's bounded and aligned with
   codeless's design.

My read: option 3 is the right call **if** the headless browser
primitive is the actual prize. The Playwright sidecar is real
engineering and porting that file is much cheaper than rebuilding
it. For everything else (fs, shell, HTTP) codeless either already has
equivalents via ai-runner or can write them faster than it can port
them.

Option 1 is the right call **if** the headless browser is a "nice to
have, not a must." That's a product question, not an engineering
question.

Option 2 is worth analysing in writing but I'd push back on it. The
operational cost of a second process + second SQLite is
disproportionate to what we're getting.

### Cleanup performed

- Clone left in `/tmp/moxxy-spike/moxxy` for follow-up reads.
  Delete when the decision lands.
- No commits made. No files outside this doc were modified.

## End-state — what vendoring looks like (only if Phase 0 green-lights)

This section is conditional and is written for the agent who picks up
the work after the spike, not for the spike itself.

### Vendor commits (three ticks, not one)

[JOB-LOOP](./JOB-LOOP.md) requires one logical batch per tick. The
vendor work is split into three sequential `M`-sized commits, each its
own tick:

**Tick 1 — vendor drop.**
Adds `moxxy/` directory with the curated subset (keep crates only,
dropped crates physically deleted, not just gitignored). Creates
`moxxy.PATCHES.md` with the policy header copied from
`ai-runner.PATCHES.md`. No entries yet. No codeless-side changes.
Reviewable as "did we copy the right files and delete the right
ones."

**Tick 2 — bridge crate and rules.**
Adds the new `codeless-primitives` crate at
`codeless/crates/codeless-primitives/` with a skeleton trait
(`Primitive::call`) and one wired-through primitive as proof-of-life
(suggest: `read_file`, simplest). Updates [SCOPE.md](./SCOPE.md) to
add the `codeless-primitives` row to the crate table and R6 to the
hard rules. Adds the R6 grep to CI. Reviewable as "does the seam
compile and is the rule enforceable."

**Tick 3 — workspace doc update.**
Updates [CLAUDE.md](../CLAUDE.md) repo-layout block to mention
`moxxy/` alongside `ai-runner/`. Updates the index doc to flip this
proposal's status from "proposal" to "accepted" with a link to the
spike audit findings. Small commit, easy review.

Each tick is reviewable on its own; together they are the structural
prerequisite for the primitive-family commits below. None of them
change runtime behavior — the first behavior change is Tick 4+.

### Subsequent commits

Each primitive family lands in its own commit:

- `feat(primitives): expose moxxy fs primitives via codeless-primitives`
- `feat(primitives): expose moxxy git primitives`
- ...

Each commit must:

- Land at least one integration test that drives the primitive from a
  fake codeless task (no real LLM, no real runner).
- Add a row to `moxxy.PATCHES.md` for every moxxy-side edit it
  required, with a `// codeless-patch-NNN` source marker.
- Leave `cargo check` green on every workspace member, including the
  iOS/Android-safe ones (which must still not depend on
  `codeless-primitives`).

### Things we will not do during vendoring

- **No drive-by refactors of moxxy code.** If moxxy's primitive impl
  has style we'd write differently, leave it. The patch log gets
  unreadable if it includes taste-driven diffs.
- **No "while we're here" feature additions.** If a primitive is
  missing a feature codeless wants, that's a separate commit on top
  of the vendored code, written as a codeless patch.
- **No deletion of moxxy tests that pass.** If they pass, they keep
  passing. We delete tests only when they reference dropped crates
  (moxxy-core, etc.) and cannot be made to compile.

### Risk acknowledged

- **Patch log will grow faster than ai-runner's.** Moxxy is larger and
  reaches further. Budget for 10–20 patches in the first month, not
  the 2 ai-runner has accumulated.
- **Upstream divergence is permanent.** This is the explicit choice:
  one-way copy, no branch tracking. We do not get upstream bug fixes
  for free; we re-vendor only if we deliberately want a specific
  upstream change, and we re-apply patches by hand.
- **The headless browser primitive carries a Playwright sidecar.**
  This adds a Node.js runtime dependency to any codeless deploy that
  enables the feature. Gate it behind a Cargo feature so the minimum
  codeless deploy stays Rust-only.

## Decisions

These are settled in this doc and do not need to be re-litigated
during the spike or vendoring. The spike can challenge them only if
it surfaces evidence that flips the trade-off.

1. **WASI plugin location is global, not per-repo.** Plugins live in
   `~/.codeless/plugins/`. Per-repo plugins would match the "repo is
   first-class" model but open a supply-chain hole: cloning a repo
   would auto-install code that runs in every job for that repo.
   Global is safer for MVP; revisit if a real per-repo use case
   emerges.
2. **Primitives reach the LLM through the MCP server**, not through
   inline patches to `ai-runner`. See [Integration seam](#integration-seam--how-primitives-reach-the-llm)
   above for the reasoning.

## Open questions

These are flagged for resolution before or during the spike, not
blockers for writing the spike itself.

1. **Does codeless need the moxxy "skills" system** (Markdown files
   with YAML frontmatter declaring agent capabilities)? Codeless
   already has the YAML/TOML job template surface. Two declarative
   surfaces may be redundant. Lean: drop skills, keep templates —
   but the spike should confirm `moxxy-runtime` doesn't require the
   skills loader to function.

## Pointers

- Scope this is layered on top of: [`SCOPE.md`](./SCOPE.md)
- Loop design this proposal explicitly preserves: [`LOOP-CODER.md`](./LOOP-CODER.md)
- Vendoring precedent (model we copy): [`../ai-runner.PATCHES.md`](../ai-runner.PATCHES.md)
- Upstream moxxy (do not link from code, only docs): https://github.com/moxxy-ai/moxxy
