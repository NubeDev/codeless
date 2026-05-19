## Done

- Reviewed the full stage 3–8 diff against R1 (crate dependency direction), R2 (single transport), R4/R5 (trust boundary + SQLite source of truth), and the wire-format-untouched invariant. All hold.
- Verified the only new `process::Command` call site outside pre-existing code is `crates/codeless-adapters-host/src/respawn.rs::supervise` — the runtime's `restart_server` uses a `tokio::sync::Notify` (`RestartTrigger`) and does not spawn.
- Verified all six new RPCs (`list_chat_adapters`, `set_chat_adapter_enabled`, `validate_chat_adapter_secrets`, `list_runners`, `set_runner_enabled`, `restart_server`) are routed inside `rpc_routes` and live behind the existing `bearer_layer` in `crates/codeless-server/src/routes.rs`.
- Verified `crates/codeless-types/tests/wire.ts.snap` diff is additive-only with zero removals.
- Ticked stage-1 checkboxes 1–5 in `DOCS/WORKSPACE-ATTACH.md` §"TODO — adapter registry"; reworded the stage-2 paragraph to name this job as the seam-provider; reworded the Gmail paragraph from "separate milestone" to "separate follow-up job".
- Marked SCOPE.md deliverable #6 as `[x]` with a one-line landing note.
- Rewrote `handover.md` so the next session opens with the gate verdict, the new RPC inventory, and the deferred-to-follow-up list.
- Committed and left the change on `codeless/adapter-registry` for mani-driven push.

## Next

- Emit gate sentinel for the runtime: `PASS: R1/R2/R4/R5 invariants hold across the stage 3-8 diff — process spawning stays in codeless-adapters-host, every new RPC rides the bearer-gated HTTP+SSE transport, the new SQLite tables are the source of truth with the cache as a session-lifetime gate, and wire.ts.snap is additive-only.`
- Push the stage-9 commit via `./bin/mani --config mani.yaml run push --projects codeless` from the workspace root.
- Schedule the two follow-up jobs the stage title flagged: (a) the Settings → Adapters UI job that consumes the six new RPCs, (b) the `codeless-gmail` adapter job that adds the `Gmail` variant to `ChatAdapterKind` and the OAuth PKCE host wiring + long-poll. Stage 2 hot-reload stays deferred until a documented trigger fires.

## What you need to know

- Gate sentinel line is in the prose above the handover block: `PASS: R1/R2/R4/R5 invariants hold ...`. The runtime parses for `PASS:` / `FAIL:`.
- No code patches were proposed at this gate — per the stage instructions, that lands in a later ramp step.
- `cargo test --workspace` was not re-run inside stage 9; the green status is trusted from stage 8's handover. The REVIEW gate's contract is diff inspection against invariants, not test re-execution.
- `ChatAdapterKind` deliberately does not include `Gmail` yet — that variant lands paired with the `codeless-gmail` crate. The composite `(kind, instance_id)` PK absorbs the addition without a schema change.
- The validate cache is process-lifetime in-memory by design (decision OQ#3 in SCOPE.md). After a restart the operator must re-validate before re-enabling — that is the load-bearing UX consequence, recorded in `crates/codeless-runtime/src/rpc/adapters.rs` doc comments.

## Open questions

- (none)
