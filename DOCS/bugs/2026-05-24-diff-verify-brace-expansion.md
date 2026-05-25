# Bug: `diff-verify` pre-check rejects valid handovers when `Done` doc uses brace-expansion path syntax

**Filed:** 2026-05-24
**Severity:** moderate — silently fails review stages even when the code work is correct; misleads the operator into thinking the job failed
**Component:** runtime / handover pre-check (the path that emits `failure_class: pre-check-failed` with detail `diff-verify pre-check failed: handover \`Done\` claims paths not in the diff: …`)

## Symptom

A multi-stage job lands real code via `cargo`, commits it on the
job's branch, but every REVIEW stage downstream is marked
`failed` with status code `pre-check-failed`. The
`failure_detail` strings include paths that **do** exist in the
worktree's diff but are spelled with shell brace expansion
(`{a,b}`) rather than as separate paths.

## Reproduction

Job `01KSBHCFBTGGMG4CYB5T9E6ZZZ` (rubix-thin-slice v2):

```
$ curl -sX POST http://127.0.0.1:7777/rpc/list_stages \
  -H 'content-type: application/json' \
  -d '{"job_id":"01KSBHCFBTGGMG4CYB5T9E6ZZZ"}' \
| jq -r '.stages[] | select(.stage.status=="failed")
       | "\(.stage.ordinal): \(.stage.failure_detail)"'

1: diff-verify pre-check failed: handover `Done` claims paths not in the diff: ./rubix/scripts/lint-doc-refs.sh
3: diff-verify pre-check failed: handover `Done` claims paths not in the diff: rubix/crates/rubix-agent/migrations/0002_history/{up,down}.sql, rubix/0002_history/up.sql, ./rubix/scripts/lint-doc-refs.sh
5: diff-verify pre-check failed: handover `Done` claims paths not in the diff: rubix/crates/rubix-agent/src/routes/{mod.rs,tools.rs}
```

The real diff contains:

```
$ cd /home/user/.codeless/worktrees/job-01KSBHCFBTGGMG4CYB5T9E6ZZZ
$ git diff --name-only ad393ba..HEAD | grep -E "0002_history|routes/"
rubix/crates/rubix-agent/migrations/0002_history/down.sql
rubix/crates/rubix-agent/migrations/0002_history/up.sql
rubix/crates/rubix-agent/src/routes/mod.rs
rubix/crates/rubix-agent/src/routes/tools.rs
```

The files **are** in the diff. The pre-check matches on the
literal string `0002_history/{up,down}.sql`, which is never a
real path, so the match fails and the stage is rejected.

## Three distinct cases inside `failure_detail`

1. **Brace expansion** — `0002_history/{up,down}.sql` and
   `routes/{mod.rs,tools.rs}`. The pre-check should expand these
   the way `sh` / `bash` would, or refuse to accept them with a
   clearer error and require the agent to list them individually.

2. **Leading `./`** — `./rubix/scripts/lint-doc-refs.sh` shows up
   in the failure even though it wasn't modified by the stage. The
   agent listed it because it *ran* the script. The pre-check
   appears to treat any path mentioned in `Done` as "claimed to be
   in the diff," with no distinction between "I modified this" and
   "I executed this." Worth tightening the contract on what `Done`
   means.

3. **Truncated path** — `rubix/0002_history/up.sql` (note the
   missing `crates/rubix-agent/migrations/`). The agent appears
   to have abbreviated the path mid-sentence. Pre-check
   correctly flags this as a missing file; the issue is the
   prompt encouraging the abbreviation, not the check.

## Suggested fixes

In priority order:

1. **Expand brace patterns in the pre-check.** A handover that
   says `{a,b}.sql` clearly means two paths. The check could:
   - shell-style expand the brace pattern, then verify each
     resulting path is in the diff;
   - OR fail with a clearer error: `"handover path '{x,y}.ext'
     uses brace expansion; list each path on its own line"`.

   Either way the agent gets actionable feedback instead of a
   stage-failed wall.

2. **Distinguish "modified" from "executed."** The agent should
   be able to write `Ran: ./rubix/scripts/lint-doc-refs.sh` (or
   similar) without the pre-check interpreting that as a diff
   claim. Today the only signal seems to be path syntax, which
   conflates the two.

3. **Document the `Done`-doc grammar** in the runtime prompt or
   `JOB-MODEL.md` so the agent knows the pre-check is strict
   about path formats. Tell the agent: "list every modified file
   on its own line; no brace expansion; no globs; no leading
   `./`."

## Workaround for operators today

When writing job `SCOPE.md` or `template.yaml`, instruct the
agent explicitly to **list paths individually** in any handover
or run note. A line in the per-job `WORKFLOW.md` such as:

> **Handover paths.** In the `Done` block of any handover or run
> note, list every touched file on its own line. Do NOT use shell
> brace expansion (`{a,b}.sql`), globs (`*.rs`), or leading `./`.
> The runtime's diff-verify pre-check is strict and will reject
> the stage with a misleading `failed` status if it can't match
> the path literally.

…would have prevented two of the three review failures on the
rubix-thin-slice-v2 job.

## Impact

In the rubix-thin-slice-v2 job ($24.07 spent), **all three work
stages passed and produced correct code** (commits `c6f457d`,
`90facb5`, `e0bb855` on branch `codeless/rubix-thin-slice-v2`).
**All three REVIEW stages were marked `failed` by this bug.**
The operator (me) had to manually verify the work was sound,
adding ~30 min of review-the-review-failure to a job that should
have been clean.

For a long autonomous job this confuses the operator into
thinking the work is broken when it isn't, and may cause them
to discard a successful run.
