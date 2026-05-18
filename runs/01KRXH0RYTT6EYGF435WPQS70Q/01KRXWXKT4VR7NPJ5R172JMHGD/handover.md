## Done

- JOB-CHAT.md gains "Status — shipped on `codeless/job-chat`" paragraphs under each of (C1), (C2), (C3) with checkbox items ticked and the load-bearing test names cited (bot_chat_e2e::*, supervisor_e2e::*, slack_chat_e2e::*); OQ-CHAT-1..5 resolutions were already inlined in stage 1 and remain in place.
- CODELESS.md gains a "What works today (per-Job chat substrate, branch `codeless/job-chat`)" section with three bullets — unified chat table (C1), supervisor read-only tools (C2), action tools + supervisor_goals (C3).
- `crates/codeless-runtime/tests/migrations.rs::migrator_creates_all_tables_from_appendix_a` updated to include `supervisor_goals` in the expected table list — the previous stage's 0026 migration left this assertion out of sync, which surfaced as a baseline failure when stage 17 ran the test gate.
- Verification all green: `cargo test --workspace` (after the migration fix), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `pnpm -C ui/codeless-ui lint` (echo-shim, no real eslint), `pnpm -C ui/codeless-ui test` (24 files / 122 tests).
- Committed as `a7751fd` on `codeless/job-chat` with message starting "stage 17: documentation + handover — JOB-CHAT.md Status rows for C1, C2, C3 …" and a paragraph in the body summarising deferred items.

## Next

- (none) — this is stage 21 of 21; the job is complete. Push (`mani run push --projects codeless`) and open the PR if the operator wants the branch merged.

## What you need to know

- `pnpm -C ui/codeless-ui lint` is currently an `echo 'lint: no eslint configured yet' && exit 0` stub. It exits 0 today; treat a future real eslint config as a separate task, not a stage-17 regression.
- The migration-list fix in `tests/migrations.rs` is a sibling of stage 13 (which added `supervisor_goals`) — the assertion was missed there and showed up here. No production code touched.
- `cargo fmt --check` is clean even though no Rust source was edited beyond a one-line vec push; the doc edits cannot affect it.
- The "What works today" bullets in CODELESS.md follow the existing dated-bullet pattern of the section above; they are intentionally scoped to the chat substrate and cite load-bearing test names so a future agent can grep back to the proof.

## Open questions

- Deferred per the job-handover paragraph in the commit body: mcp_forward parity from PLUGIN-MCP is unrelated and stays in its own plugin track; mobile-shell wiring of the new chat surfaces is Phase 6 (codeless-tauri-mobile sees only `codeless-types` + `codeless-client` and the substrate already meets that boundary); multi-user trust beyond `chat_bindings.bound_by` is Phase 7 OIDC per OQ-CHAT-3.
