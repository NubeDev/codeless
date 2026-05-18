## Done

- crates/codeless-runtime/src/diff_verify.rs: tightened the pre-check tokenizer. `verify_handover` now takes a `worktree: &Path` and filters every shape-passed token through a two-clause rule before the diff-presence check: (a) the token starts with a prefix derived from the current diff's file list (first segment, two-segment prefix when depth ≥ 3, and literal repo-root filenames), or (b) the token resolves under the worktree at check time. Tokens that satisfy neither are silently dropped. Absolute paths and `..` traversal are explicitly rejected in (b) so an absolute path that exists elsewhere on the host can't satisfy a worktree-relative claim.
- New helpers `derive_diff_prefixes`, `matches_diff_prefix`, `token_is_path_shaped_for_diff` with R2-compliant doc comments explaining the *why* (diff-driven so self-updates as the repo grows; (b) keeps no-op claims on unchanged-existing files visible).
- Five new unit tests: drops the four real-world false positives (`tool.call`, `rest_proxy.path`, `metadata_json.delivery.slack`, absolute ai-runner path), still flags real missing paths under a known prefix, admits an unchanged existing file via (b) and flags its no-op claim, derive_diff_prefixes shape pinning, absolute/parent-traversal rejection.
- Threaded `worktree` through the two callers (`template_runner::run_diff_verify_precheck`, `scope_patch_emit::verify_loosen_evidence`).
- Updated two pre-existing tests whose diffs no longer admitted the asserted miss under the tightened rule: `precheck_fails_when_handover_claims_a_path_no_commit_touched` (now seeds an `unrelated/other.md` so the `unrelated/` prefix is in the derived set) and `loosen_emits_only_when_evidence_diff_matches` first-rejection case (diff now contains a sibling `crates/codeless-predicates/...` file so the cited path survives shape filtering and is rejected on diff-presence, not on shape).
- `cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` all green. (A flaky parallel-git failure surfaced once in `codeless-adapters-host::git_diff::tests` but passes when rerun and is unrelated to this stage's changes.)
- Committed as `990ad16` on `codeless/auto-bypass-hardening`.

## Next

- Stage 8: extend `auto_bypass_policy::policy_comment` in `crates/codeless-runtime/src/auto_bypass_policy.rs` to append a fenced block carrying the prior stage's failure_class (wire name) and failure_detail (truncated to 400 chars with `…`) to the canned policy paragraph; widen the signature to `policy_comment(policy, prior: Option<&PriorFailure>) -> String` with a local `PriorFailure { class, detail }` struct; `None` reproduces today's bytes-for-bytes output so existing canned-string assertions stay green. Per SCOPE.md §Q4 for fence/truncation/ordering specifics.

## What you need to know

- Mani was not used to commit — this worktree is the inner codeless repo only; the workspace-level `./bin/mani` and `mani.yaml` aren't reachable. Raw git was used. The next session running from the workspace root should still use mani per CLAUDE.md.
- `verify_handover`'s signature changed: any future caller must pass the worktree root. Both existing callers (`template_runner`, `scope_patch_emit`) already do.
- The new shape filter is *additive* — it runs after the existing `looks_path_like` shape filter, not as a replacement. Existing `extract_paths_from_done` / `verify_paths_in_diff` callers and their tests are unchanged.
- `extract_paths_from_done` deliberately still returns broad shape-only candidates; the diff-context narrowing lives inside `verify_handover`. If a future caller wants the new behavior without going through `verify_handover`, they can call `derive_diff_prefixes` + `token_is_path_shaped_for_diff` themselves — both are crate-private today (`fn`, not `pub fn`); promote if needed.

## Open questions

- (none)
