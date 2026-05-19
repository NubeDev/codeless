## Done

- Stage 5 review of stages 3-4 (mode column + setThreadMode RPC; read-only fs.list/fs.read/fs.search tools with workspace-root sandbox) against R1/R2/R4/R5 and wire compatibility

## Next

- (none — stage 6 will pick up write paths)

## What you need to know

- PASS: stages 3-4 keep R1 (host-only tools, no process spawn), R2 (RPC-only transport for setThreadMode), R4/R5 (server enforces mode read from SQLite, safe `read-only` default, strict `from_wire` reject), and add only additive, defaulted wire fields
- The `ui/codeless-ui/src/lib/shell/path-picker.ts` and `setup/SETUP.md` deltas in `git diff master..HEAD` are merge-base drift from master commit `8ef0768 fix pick path`, not introduced by stage-3/4 commits — verified via `git log master..HEAD -- <path>`
- Stage 3 appears twice in the log (commits `f84db5d` and `a562291`) with near-identical messages; harmless duplicate commit but worth a glance next stage

## Open questions

- (none for the gate)
