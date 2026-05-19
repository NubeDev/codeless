## Done

- reviewed the stage-1 diff (commit 080857e): docs-only, adds four resolved open questions under DOCS/SCOPE.md §"Adapter registry, stage 1"
- confirmed no code, manifests, or wire types changed — Layer-1 invariants are not in scope to break here
- emitted gate sentinel below

## Next

- stage 3 should begin the WORK ramp; the runtime will resume on PASS

## What you need to know

- PASS: stage 1 was a docs-only resolution of the four open questions in WORKSPACE-ATTACH.md §"TODO — adapter registry"; R1/R2/R4/R5 and wire formats are untouched because no code changed.
- Decisions recorded: composite PK `(kind, instance_id)` for chat_adapters, single PK for runner_config; `--respawn-on-exit` opt-in; 5-min validate-cache TTL with write-invalidation; resumable iff `Resumable` capability AND last persisted transition <30 s.

## Open questions

- (none)
