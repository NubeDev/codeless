## Done

- crates/codeless-types/src/scope_patch.rs (new): ScopePatchId, ScopePatchKind {Tighten,Loosen}, ScopePatchTarget {ClaudeMd, JobScopeMd, JobWorkflowMd, JobClaudeMd}, ScopePatch — mobile-safe, specta + serde derives, kebab-case wire labels
- crates/codeless-types/src/event.rs: new Event::ScopePatchProposed variant (rename "scope-patch-proposed") carrying stage_id, review_id, patch_id, kind, target, target_path, evidence_stage_id (Loosen), has_predicate (Tighten) per SESSION-MUTABLE-SCOPE-DECISIONS.md Q7
- crates/codeless-types/src/lib.rs: module + re-exports
- crates/codeless-types/tests/{serde_wire.rs,specta_snapshot.rs,wire.ts.snap}: wire-shape test + specta registration + regenerated snapshot
- crates/codeless-runtime/src/scope_patch_emit.rs (new): parse_blocks for SCOPE-PATCH-BEGIN/END mini-format (kind/target/target-path/rationale/predicate/evidence/body), append_to_proposals_file (creates DOCS/SCOPE-PROPOSED.md with header on first append, append-only after), emit_from_handover orchestration publishing Event::ScopePatchProposed. EmitOutcome enum {Emitted, NoBlock, MultipleBlocks, Malformed, SideEffectFailed} keeps shadow-mode failures observable-but-non-fatal.
- crates/codeless-runtime/src/lib.rs: pub mod scope_patch_emit
- crates/codeless-runtime/src/template_runner.rs: on REVIEW PASS, read the stage's handover and call emit_from_handover with a fresh ReviewId; map every EmitOutcome to a structured warn/info log; gate verdict unchanged (Step 5 promotes Malformed/Multiple to FAIL)
- crates/codeless-rpc/examples/wire_ts.rs + ui/codeless-ui/src/lib/rpc/generated/wire.ts: new types registered and regenerated so UI compiles against the new event variant
- committed as `stage 4: ScopePatch shadow mode …` on codeless/session-mutable-scope

## Next

- REVIEW gate before Step 5: confirm shadow-mode emission is visible end-to-end, then land parse-time guards in codeless-runtime/scope_patch_emit (kind/target mutable-set membership, one-patch-per-REVIEW promoted from a warn to a FAIL reason, evidence_stage_id required on Loosen, has_predicate required on Tighten). Cite SCOPE.md Step 5 invariants.

## What you need to know

- Shadow-mode policy is deliberate: Malformed / MultipleBlocks / SideEffectFailed all return without aborting the REVIEW gate, so the kill-criterion telemetry can count noise without the gate weaponising parse errors before Step 5 lands. Step 5 promotes these to FAIL by changing the match arms in template_runner.rs and tightening parse_one in scope_patch_emit.rs.
- The proposals-file format is markdown (header + per-patch `## <ulid>` heading + bulleted metadata + Rationale / Body sections). It is append-only; the Step 6 approval CLI is the only consumer that mutates entries (decisions Q1).
- ScopePatchTarget intentionally omits any wire-format variant (handover.rs, JOB-MODEL.md, JOB-LOOP.md) so Step 5 can reject those at parse time without a separate "blocked" enum.
- Mobile-safety: scope_patch.rs has no codeless-runtime dependency and no I/O; only serde + specta + ulid. Wire-ts regen confirms the types reach the UI.
- No telemetry sink beyond the events bus — decisions Q7. The kill-criterion query runs against the existing events table indexed by `event_type`.
- Pre-existing test flake `rpc_in_process::job_filtered_subscription_drops_unrelated_events` still failing on HEAD~; not introduced by this stage. cargo clippy --workspace --all-targets -- -D warnings and cargo fmt --check both clean. Per WORKFLOW.md the headless session committed but did not push — the outer JOB-LOOP harness handles the push.

## Open questions

- (none)
