## Done

- DOCS/JOB-CHAT.md §Open questions rewritten as resolutions for OQ-CHAT-1..5, in SCOPE.md's "Settled" style, with the numbering preserved for cross-doc citation
- v0.1 transport set (Web, Cli, Telegram, Slack, Supervisor) and the lowercase-ASCII wire-name convention for ChatTransport added as a settled block right under the wire types in §Data model
- Stale "See open Q #5" comment on chat_messages.run_id retargeted to OQ-CHAT-4
- Committed on branch codeless/job-chat as 4514ff4 with subject "stage 1: resolve open questions"

## Next

- Stage 2 picks up the next item in the kickoff sequence (the next stage of the 21 in this job); see DOCS/JOB-CHAT.md C1 punch list for the implementation order — migrations + wire types are the natural first code stage

## What you need to know

- This stage is doc-only by directive; no Rust, no SQL, no tests touched
- The five resolutions are load-bearing for later stages:
- OQ-CHAT-1 / -2: the metadata_json key table is the contract the C1 RPCs and the C2/C3 supervisor will write against; do not invent new top-level keys without amending the table
- OQ-CHAT-3: any auth/ACL code in C1-C3 should explicitly cite "single-tenant v0.1, see OQ-CHAT-3" rather than re-deriving the policy
- OQ-CHAT-4: list_job_messages must default to per-Job (no run_id filter); a run_id parameter, if added later, is opt-in only
- OQ-CHAT-5: every metadata key the substrate sets must appear in the §Open questions §OQ-CHAT-5 table; adapter-only keys live under metadata_json.<transport>.*
- ChatTransport wire-name rule: the Rust enum serializes to lowercase ASCII; SQLite transport column values already match. Specta-derived TS bindings inherit the casing — verify when wire-types are generated in a later stage
- I did not run mani for the commit. The repo has no mani.yaml at this level (the workspace one lives outside this worktree); standard git commit is what the JOB-LOOP harness wraps from outside. If a later stage runs mani directly, point it at the workspace root per CLAUDE.md
- cargo test / clippy / fmt were not run because nothing under crates/ or ui/ changed

## Open questions

- (none)
