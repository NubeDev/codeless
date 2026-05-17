## Done

- Ran the four gate greps from .codeless/jobs/telegram-integration/WORKFLOW.md against this worktree:
- `pub bypass` on ResumeJobArgs — FAIL (still `pub bypass_failing_stage: bool` at crates/codeless-rpc/src/methods.rs:142)
- `pub next_stage_comment` on ResumeJobArgs — FAIL (no matches)
- `actor: Option<String>` on JobResumed — FAIL (no matches in crates/codeless-types/src/event.rs)
- `comment: Option<String>` on JobResumed — FAIL (no matches)
- Verified `master` (this repo's main; `origin/HEAD -> origin/master`) does not contain the slack-integration stage-1 commit. The commit `245a228 stage 1: bypass + next_stage_comment on ResumeJobArgs, actor on JobResumed` is reachable only from `codeless/slack-integration` (`git branch --contains 245a228` returns just that branch).
- Stage 1 marked `[!]` (blocked / gate closed). No code changed; no commit produced, per WORKFLOW.md's "stage genuinely produced no change" carve-out.

## Next

- Wait. Do not advance to stage 2.
- Re-run telegram-integration stage 1 after slack-integration stage 1 (commit 245a228 or its merged equivalent) has landed on `master`. The four greps in WORKFLOW.md lines 33–40 must all pass before stage 2 may start.
- When the gate opens, stage 2 is "Decide adapter shape" — extract `codeless-slack` parser + event subscriber + rate-limiter into a shared `codeless-bot-core` crate (Approach 1), or create a fresh `codeless-bot` crate with `transport/{slack,telegram}.rs` (Approach 2). Read SCOPE.md, the slack-integration job dir's current state, and DOCS/SCOPE-TELEGRAM-INTEGRATION.md before picking.

## What you need to know

- Branch: `codeless/telegram-integration`, based on `master` (commit 952eb13). Working tree clean.
- This repo's main branch is named `master`, not `main`. WORKFLOW.md says "main"; treat that as a synonym for `master` (origin/HEAD points at it).
- Do NOT borrow the fields by merging `codeless/slack-integration` into this branch. WORKFLOW.md is explicit: "Do not branch-merge from the slack-integration branch into this worktree to 'borrow' them — they must come from `main`." Doing so would force this job to carry the slack scaffolding and would conflict at merge time.
- The slack stage-1 commit on `codeless/slack-integration` renames `bypass_failing_stage` → `bypass` with a serde alias for backward compat, adds `ResumeJobArgs.next_stage_comment: Option<String>`, and adds `JobResumed.actor: Option<String>`. The commit message claims `actor` only; WORKFLOW.md additionally requires `JobResumed.comment: Option<String>`. If slack stage 1 lands without `comment`, the gate still fails — flag this upstream rather than papering over it here.
- Slack-integration is currently at stage 4 on its own branch (`ffa11bd stage 4: Wire parsed commands to RpcClient calls...`). The RPC fields exist there but have not been promoted to master yet.

## Open questions

- Who/what merges slack-integration stage 1 into master, and on what timeline? Without a merge cadence the telegram-integration loop is parked indefinitely.
- WORKFLOW.md requires `JobResumed.comment: Option<String>` for the gate, but the slack stage-1 commit (245a228) appears to only add `actor` on `JobResumed`. Either (a) the slack adapter encodes the operator comment elsewhere (e.g. piggybacking on the existing prefix mechanism) and `JobResumed.comment` will arrive in a later slack stage, or (b) WORKFLOW.md is over-specifying. The next telegram-stage-1 session should re-check this against whatever actually lands on master and, if needed, escalate to amend WORKFLOW.md rather than silently relaxing the gate.
