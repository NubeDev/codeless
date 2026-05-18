## Done

- Reviewed stages 0–7 diff vs master for M3+M4 of WORKSPACE-ATTACH
- Verified Layer-1 invariants (R1 crate direction, R2 single transport, R4/R5 trust boundary, wire-format stability)
- PASS: UI-only diff plus additive wire-type registration; all four workspace RPCs go through RpcClient, server retains path-validation authority, no host-only imports leak into mobile-safe crates, no existing wire shapes mutated.

## Next

- Follow-up job: flag /workspaces top-level route + sidebar group (deferred per M4 phasing note)

## What you need to know

- Sentinel line above (`PASS: ...`) is what the runtime parses
- Rust-side change is limited to crates/codeless-rpc/examples/wire_ts.rs registering new codeless-types entries; no transport or adapter edits
- All UI work confined to ui/codeless-ui (settings tab, modals, store, shell PathPicker)

## Open questions

- (none)
