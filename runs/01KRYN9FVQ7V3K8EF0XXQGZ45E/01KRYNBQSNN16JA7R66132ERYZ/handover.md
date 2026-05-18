## Done

- resolved the four open questions in `.codeless/jobs/auto-bypass-hardening/SCOPE.md` inline (Q1 SQLite error-code set, Q2 WAL pragma timing, Q3 path-shaped tokenizer rule, Q4 bypass-comment failure_detail thread-through wording), each with a chosen answer plus a *why* paragraph
- replaced the "Open questions (resolve in stage 1, before any code)" section with "Open questions (resolved in stage 1)" containing the locked-in decisions the rest of the job builds on
- added `DOCS/sessions/2026-05-19-auto-bypass-hardening.md` as the per-stage running log; it carries a short summary of all four resolutions and a concrete stage-2 handover (file paths, what to add, what NOT to touch)
- committed as `stage 1: resolve open questions` (SHA `9f0dd1f`) on branch `codeless/auto-bypass-hardening`

## Next

- stage 2: add `FailureClass::InfrastructureError` in `crates/codeless-types/src/stage.rs` (one-line doc comment per CLAUDE.md R2 — explains *why* infra failures halt, not what the variant stores); round-trip `failure_class_label` / `parse_failure_class` in `crates/codeless-runtime/src/store/codec.rs`; regenerate the specta wire snapshot consumed by `ui/codeless-ui/src/lib/rpc/wire.ts`; `cargo test -p codeless-types` green
- do NOT touch the sqlx error mapper (stage 3) or the state-machine halt branch (stage 4) in the stage-2 commit — keeping the diff to the variant alone is what makes the M-INFRA REVIEW gate at stage 5 easy to inspect and revert

## What you need to know

- the Q1 set widens the bias by one code: `8 SQLITE_READONLY` joins the four codes the bias listed, because the real-world trigger (filesystem remount RO under disk pressure, container CoW quota) lands in the same operator-intervention bucket as `SQLITE_FULL`. `26 SQLITE_NOTADB` is deliberately *not* in the infra set — wrong-file-at-DB-path is a deployment-config error, not a transient host condition
- the Q1 matcher reads the primary `code()` only — extended `IOERR_*` codes flow through naturally because they share the primary; a non-`Database` `sqlx::Error` (`Io`, `Tls`, `PoolClosed`, `PoolTimedOut`) stays in `RunnerError` for now
- the Q3 rule is *self-updating*: prefixes are derived from the current diff's file list (first segment + two-segment prefixes + literal repo-root filenames that appear in the diff), so a new top-level directory works automatically without a follow-up patch. The new gate layers *after* the existing `looks_path_like` shape filter, not as a replacement — stage 6 widens `extract_paths_from_done`'s call site to pass the diff's file list down
- the Q4 fence is a triple-backtick fence with **no language tag** (SCOPE.md uses `~~~` in the example so the doc itself renders). Detail is truncated at the prompt boundary only (400 Unicode scalars, U+2026 marker); the stored row value is unchanged so audit trail keeps the full string. `policy_comment` widens to `(policy: &AutoBypassPolicy, prior: Option<&PriorFailure>) -> String` where `PriorFailure { class: FailureClass, detail: String }` is local to `codeless-runtime`; `None` reproduces today's bytes so existing string-pin tests stay green
- commit was made directly via `git` rather than `mani` — this worktree has no `../bin/mani` available, and the codeless-agent system prompt requires `git add` + `git commit` so changes survive worktree cleanup. The branch `codeless/auto-bypass-hardening` is local-only; a later session running from the workspace root will push it via mani

## Open questions

- (none) — stage 1's whole purpose was to close the four; if stage 2+ discovers a decision the resolved set does not cover, the workflow says halt and surface in chat rather than silently invent
