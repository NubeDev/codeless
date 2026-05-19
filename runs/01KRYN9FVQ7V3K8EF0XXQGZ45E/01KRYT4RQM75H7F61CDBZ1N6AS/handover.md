## Done

- DOCS/AUTO-BYPASS-DECISIONS.md gained Q8 (InfrastructureError rule, sqlx primary-code set, the 7-step classify_stage_failure layering, NOTADB carve-out, Q2-rejection-rule extension, Q7 cross-link)
- DOCS/JOB-WORKFLOW.md "TODO precheck rules reference" flipped to a documented "Precheck rules" section with Rule #1 (Done<->diff cross-check, three-layer path-shape filter, four killed false positives named, fuller overhaul deferred), plus Rules #2 and #3 carried forward as numbered rules
- CODELESS.md "What works today" gained an auto-bypass-hardening bullet (infra halt + WAL pool + tokenizer hardening + bypass thread-through + ~ glyph)
- pnpm -C ui/codeless-ui lint (placeholder echo) and pnpm -C ui/codeless-ui test green (27 files / 135 tests)
- single commit 537d301 on codeless/auto-bypass-hardening

## Next

- (none) — this is the final stage of the job

## What you need to know

- The Rust verifications (`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`) could not be run cleanly in this worktree because a concurrent worktree (job-01KRYQJVK0G60MEZVFQ6KW3Y1F) is actively rewriting `../ai-runner/Cargo.toml`'s `workspace = "..."` pointer and reshaping the shared `target/` directory mid-build. Each time we re-pin the pointer at ours and start a build, the other job re-pins it and removes target/ fingerprint files mid-test. The single observed test failure (`codeless-adapters-host::git_commit::tests::git_revert_undoes_an_earlier_commit_and_returns_new_sha`) is an `Os { code: 2, NotFound }` on a `git` subprocess from inside `init_repo`, not a regression caused by this stage — this stage is docs-only (three markdown files).
- Stage commit is `537d301`. The four green Rust checks should be re-run from a worktree that is not concurrent with another active job.
- The precheck rule wording deliberately calls out that the tokenizer was *hardened, not rewritten* — a fuller diff-verify overhaul (Markdown-aware parser, glob-shaped Done bullets) is a separate job.
- AUTO-BYPASS-DECISIONS.md Q8 pins the layering order load-bearingly: cancel -> no-store -> row-load -> stop_reason -> infra -> policy -> default-halt. Any future PR that moves the infra branch above stop_reason or below the policy match needs a new Q in the decisions doc first.

## Open questions

- (none)
