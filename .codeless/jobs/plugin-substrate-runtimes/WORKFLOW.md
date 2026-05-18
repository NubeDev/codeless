# Workflow — plugin-substrate-runtimes

How to drive the stages in `template.yaml`. Read this before every
stage, alongside `SCOPE.md` and the four plugin docs (substrate,
WASM, UI-federation, MCP). Process is design-only in this job —
treat its doc as the seam contract, not a build target.

## Sequencing

- **Stage 1 is non-negotiably alone.** It resolves the open
  questions and propagates them into the plugin docs. No code work
  starts until the resolutions are committed and pushed. The biases
  in `SCOPE.md` §"Open questions" are *starting points*, not
  pre-decided answers — record the chosen answer + one-line *why*
  under each.
- **Stages 2-4 (SDK + WIT + host) form the WASM-A milestone.** They
  may be sequenced in this exact order. Stage 2 ships the
  authoring surface, stage 3 the ABI contract, stage 4 the host —
  reversing introduces type churn on every commit. The plugin keeps
  compiling unchanged across all three via a thin shim that stage 5
  retires.
- **REVIEW M-WASM-A** gates stages 5-7. Do not start WASM
  capability or limits work without the SDK + WIT + host crates
  visible in the workspace and clippy-clean.
- **Stages 5-7 (WASM-B) build linearly.** Stage 5 swings the
  notes plugin onto the SDK and proves the e2e test passes under
  both flavours. Stage 6 adds the capability sandbox and the
  attachment WIT. Stage 7 adds fuel + memory + wall-clock caps.
  Each stage's tests gate the next.
- **REVIEW M-WASM-B** gates the entire UI surface (stages 8-12).
  No UI work before WASM is green end-to-end.
- **Stages 8-12 (UI) build linearly.** Stage 8 is the fork +
  rename. Stage 9 is the SDK API surface. Stage 10 is the host
  wiring. Stage 11 is the server REST surface. Stage 12 is the
  notes plugin's `ui/` subtree and the three Playwright tests.
- **REVIEW M-UI** gates stages 13-14. No server-side manifest /
  MCP work before the UI surface is green.
- **Stages 13-14 (server + MCP) may overlap in code but ship as
  separate commits.** Stage 13 is the manifest extension and the
  process-seam test. Stage 14 is the MCP contribution surface and
  the three MCP tests. Stage 13 must commit first because the MCP
  manifest extension extends the same parser.
- **REVIEW M-MCP** gates stage 15. The final stage is documentation
  + the all-greens check.

## Per-stage discipline

Before writing any code in a stage:

1. Re-read `SCOPE.md` §"In scope" and §"Constraints". If the stage
   demands something not in §"In scope", **stop and surface it** in
   the job chat — do not silently expand scope. The estimator
   plugin, the deferred items, mobile MF wiring, and runtime-table
   writers for `NotesAppend::call` are the most likely scope-creep
   attractors; resist all four.
2. Re-read the **relevant section** of the relevant plugin doc:
   - Stages 2-4: `PLUGIN-WASM.md` §"The crate" + §"The authoring
     SDK" + §"The WIT contract".
   - Stages 5-7: `PLUGIN-WASM.md` §"Capability sandbox" + §"Limits".
   - Stages 8-12: `PLUGIN-UI-FEDERATION.md` end-to-end (it's
     shorter than the WASM doc and dense; read it whole each time).
   - Stages 13-14: `PLUGIN-SUBSTRATE.md` item 6 (manifest) +
     `PLUGIN-MCP.md` end-to-end.
3. Re-read `codeless/CLAUDE.md` §"Hard rules" — R1/R2/R3/R4/R5 are
   load-bearing here. **R1 is the one most likely to silently
   break:** the WASM host crate is host-only, but the runtime-
   adapter table in `codeless-tools` must stay mobile-safe.
   Confirm via:
   ```
   cargo check -p codeless-client --target aarch64-apple-ios
   cargo check -p codeless-client --target aarch64-linux-android
   ```
   Either fails → stop, do not commit, fix the leak.
4. Check the reuse rule. Every file you copy from rubix gets a
   `// codeless-ported-from: rubix-workspace/<path>@<sha>` header
   on its first commit. No file appears in this job without the
   header if it came from rubix; no file appears with the header
   if it did not. Verify before the stage's `checks` todo:
   ```
   rg 'codeless-ported-from' --files-with-matches | wc -l
   ```
   The count must match the stage's intent.
5. Check R2 / R3 / R6 grep gates before committing. For UI stages
   (8-12):
   ```
   rg '@tauri-apps' ui/codeless-ui/src --glob '!src/shells/desktop/**'
   rg '@tauri-apps' plugins/notes/ui --glob '!**/node_modules/**'
   ```
   The first set must not grow. The second set must stay empty.
   R3 forbids per-shell `.tsx`; grep:
   ```
   rg --files ui/codeless-ui/src plugins/notes/ui | \
     rg '\.(web|desktop|android|ios)\.tsx?$'
   ```
   The match set must stay empty.

Before committing a stage:

1. `cargo test --workspace` green (or the targeted package's tests
   for early-stage work — stage 15 catches the workspace gate).
2. `cargo clippy --workspace --all-targets -- -D warnings` green.
3. `cargo fmt --check` green.
4. For UI stages: `pnpm -C ui/codeless-ui lint` and
   `pnpm -C ui/codeless-ui test` green; the Playwright happy-path
   test landed in that stage actually exercises the new behaviour,
   not just renders markup.
5. The stage's snapshot tests, if any, are updated intentionally —
   review every diff line; no blind `-u`. Snapshot drift on the
   plugin manifest, the slot vocabulary, or the MCP listing is the
   canary for a contract change you didn't mean to make.
6. Update `SCOPE.md` §"Deliverables" with a `[x]` against anything
   completed in the stage.

Commit + push via **mani** from the workspace root:

```
./bin/mani --config mani.yaml run commit --projects codeless \
  MSG='stage N: <one-line title from template.yaml>'
./bin/mani --config mani.yaml run push --projects codeless
```

No `--force`, no `--no-verify`. If a hook fails, fix the cause.

## Closing trio — the last three todos of every stage

Every stage's todo checklist ends with the same three items, in
order. The user watches these tick over in the `Stages` overview;
they are how the user confirms a long-running stage actually
landed instead of just looking like it did. Do **not** rename or
reorder them.

1. `checks` — run the stage's `verify:` list (or `verify_cmd`).
   Every step must pass. On failure: stop, fix, re-run; do not
   advance to `docs`.
2. `docs` — update `handover.md` for the next stage and the active
   session doc, in the same worktree, so the fresh agent that opens
   the next stage has the context it needs (per SCOPE Constraint:
   anything that must survive a stage boundary is on disk, not in
   the agent's head).
3. `git` — stage the changes (`git add -A` from the worktree root,
   or specific paths if the stage was surgical), commit with the
   message `stage N: <one-line title from template.yaml>` so the
   history mirrors the template stages one-for-one, and push to
   the job's branch (`codeless/plugin-substrate-runtimes`) so the
   work is recoverable even if the worktree is wiped.

A stage is not "done" until all three todos are green and the push
succeeds. If `checks` or `git` fails, fix the cause and retry — do
not mark the stage `[x]`, do not advance, and never `--force` or
`--no-verify`. If a stage genuinely produced no change, say so in
the handover and mark `git` as `skipped — no diff`, but the next
stage's commit must include any side-effect files the investigation
touched.

## REVIEW gates

Four gates: M-WASM-A (after stage 4), M-WASM-B (after stage 7),
M-UI (after stage 12), M-MCP (after stage 14).

At each gate, write a handover comment in the job chat with:

- One bullet per item the gate is checking.
- For **M-WASM-A**: confirm the three crates compile, the runtime-
  adapter table is in `codeless-tools`, and the iOS/Android cargo
  check matrix passes (paste the matrix's tail). Confirm no
  workspace-wide clippy regression. Confirm every ported file has
  its `// codeless-ported-from:` header.
- For **M-WASM-B**: paste the `plugin_substrate_e2e::*` +
  `plugin_wasm_e2e::*` test output. Confirm `cargo test
  --workspace` runs both flavours of the notes plugin and both
  pass. Confirm the WASM capability set ships *as documented in
  PLUGIN-WASM.md* — call out any deviation explicitly and update
  the doc in the same gate, not after.
- For **M-UI**: paste `pnpm -C ui/codeless-ui test` tail, the
  Playwright report summary, and a screenshot (or markdown render)
  of the Assistant in the `notes` persona with the plugin's
  `AssistantPanel` mounted. Confirm `codeless/CLAUDE.md` has R6
  and the workspace `CLAUDE.md` has the four-doc pointer set.
- For **M-MCP**: paste the three MCP tests' output. If a real MCP
  client is available locally (Claude Desktop), paste the
  `tools/list` showing `notes.notes_append` (mock-client output
  is acceptable for CI).

Do not proceed past a REVIEW gate without explicit approval in
chat. REVIEW gates still commit + push the stage that *led* to
the gate; they pause only the *next* stage.

## Anti-patterns specific to this job

- **Do not** depend on `rubix-extensions-sdk`, `rubix-block-client`,
  or `spi` as Cargo dependencies. Same anti-pattern
  `TOOLS-PORTING.md` rejected for moxxy. The reuse story is
  *port files with attribution*, not *vendor crates*. Confirm via:
  ```
  rg 'rubix-extensions-sdk|rubix-block-client|spi\s*=' \
    crates/*/Cargo.toml
  ```
  The match set must stay empty.
- **Do not** import `@tauri-apps/*` outside
  `ui/codeless-ui/src/shells/desktop/`. R2 and R6 both forbid it.
- **Do not** create a `WorkspacesPage.web.tsx` /
  `WorkspacesPage.desktop.tsx` style split. R3 is non-negotiable;
  the slot vocabulary is the only contribution surface for plugin
  UI. If a plugin needs per-shell behaviour, it injects through
  the existing `ui/codeless-ui/src/lib/shell/` pattern.
- **Do not** add a fourth dispatch kind to MCP (`wasm_direct`,
  `process_direct`, etc.). PLUGIN-MCP.md fixes the dispatcher at
  three kinds; a WASM plugin's MCP tool dispatches via
  `tool_call`, not via a runtime-specific path. This is the load-
  bearing simplification.
- **Do not** template the `description_md` in MCP-tool manifests at
  runtime. Static files only — prompt-injection defence. The
  parser rejects any `description_md` whose path resolves to
  something outside the plugin bundle.
- **Do not** invent slot ids in the host. Adding a slot is a host-
  side change, documented in `PLUGIN-UI-FEDERATION.md` §"Slot
  vocabulary". Plugins cannot self-declare new slots. If a stage's
  work needs a new slot, raise it in chat — do not add it silently.
- **Do not** raise WASM fuel/memory/wall-clock caps from inside the
  plugin manifest. OQ-WASM-5 resolution: global defaults, per-
  plugin overrides via codeless config only.
- **Do not** implement `mcp_forward` in this job. Parse-and-fail is
  the v0.1 stance per OQ-MCP-1; the supervisor / upstream-schema
  cache is a follow-up.
- **Do not** wire the runtime-table writer in `NotesAppend::call`.
  It is explicitly deferred in `SCOPE.md` §"Out of scope"; the
  substrate e2e tests already cover the seams without the body.
- **Do not** start the estimating plugin or any of substrate items
  2 + 4 (CommonChat extraction, chat state moves server-side). They
  are explicit blockers for the estimator and out of scope here.
- **Do not** treat the rubix `block-client`, `spi`, `node.rs`,
  `ctx.rs`, `subscribe.rs`, or `settings.rs` files as port
  candidates. They are graph-node vocabulary; codeless tools are
  flat. Porting any of them would silently import the rubix graph
  SPI. The reuse table in `PLUGIN-SUBSTRATE.md` is the canonical
  list — adhere to it.
- **Do not** drive-by-refactor `codeless-tools` while adding the
  runtime-adapter table. R4 (codeless/CLAUDE.md): three similar
  lines is better than a premature abstraction.

## When to halt

- **iOS or Android cargo check fails after a stage's changes.**
  Stop, do not commit. The runtime-adapter table or a downstream
  type leaked a host-only dep. Fix the leak before continuing.
- **A typed-wire snapshot or RPC schema mismatch you cannot
  explain.** Do not regenerate the snapshot. Snapshot drift on the
  plugin manifest, the slot vocabulary, or the MCP tool listing is
  the canary for a contract change you didn't mean to make. Surface
  in chat.
- **R2 / R3 / R6 grep regression.** New `@tauri-apps/*` outside
  `src/shells/desktop/`, a new `*.{web,desktop,android,ios}.tsx`,
  or a plugin source importing forbidden modules. Halt and rework.
  Both are non-negotiable.
- **A clippy warning the stage's changes introduced cannot be
  fixed without an `#[allow(...)]`.** Surface in chat. The only
  pre-approved `#[allow(unsafe_code)]` block is at the
  WASI FFI boundary in `codeless-plugin-sdk/src/wasm.rs` (per the
  rubix-ported pattern); any other allow needs explicit sign-off.
- **A WASM e2e test wedges instead of failing fast.** The fuel /
  wall-clock caps exist precisely so this can't happen; if it
  does, the cap implementation is wrong. Halt and rework before
  the next stage uses the cap surface.
- **Stage 1 resolutions force a doc change larger than the
  resolution text itself.** A bias's resolution implying a
  redesign of the plugin doc is a signal the bias was wrong;
  surface in chat, do not silently rewrite the doc to match an
  unconsidered alternative.
- **The job runs out of stages without all of `SCOPE.md`
  §"Deliverables" green.** Mark the unfinished items `[!]` in
  `SCOPE.md` (codeless/CLAUDE.md R4) and stop. Do not commit a
  partial implementation with a TODO.
