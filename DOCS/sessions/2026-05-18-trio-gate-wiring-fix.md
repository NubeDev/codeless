# Trio gate wiring fix

Branch:      master (fix on a fresh `fix/trio-gate-wiring` branch)
Status file: this file
Related:    `.codeless/jobs/todos-recorder-and-gate/SCOPE.md` (the
            feature this fix completes)
Goal:       wire `verify_runner::run_verify` and
            `trio_emitter::commit_stage_changes` into the production
            stage-execution path in `template_runner.rs`, plus a
            terminal `Skipped` fallback when neither applies, so the
            `wait_for_trio_resolved` gate can actually close. Without
            this fix **every new job hangs forever on its first
            stage**.

## Why this exists (read first)

PR #24 (`todos-recorder-and-gate`, merged 2026-05-18 as commit
`1ad25ac`) shipped a stage-completion gate that refuses
`StageCompleted{ Passed }` until all three trio todos (`checks`,
`docs`, `git`) on the stage's terminal task resolve to a terminal
status (`Done` / `Skipped` / `Failed`).

The trio rows are **created** correctly at stage entry via
`state_machine.rs` injection. But only one of the three rails is
actually **driven to a terminal state** by production code:

| Trio rail | Resolution path | Production caller? |
|---|---|---|
| `docs` | `emit_trio_started/completed(TodoKind::Docs)` from `claude_runner.rs:409` (around the handover write) | yes |
| `checks` | `verify_runner::run_verify(..., Some(&store))` (emits the trio when `store` is `Some`) | **no — only called from `verify_runner.rs`'s own tests** |
| `git` | `trio_emitter::commit_stage_changes` (emits the trio around `git commit`) | **no — only called from `trio_emitter.rs`'s own tests** |

Result: every new job runs the first stage's claude session → emits
`Docs` trio start+complete → enters `wait_for_trio_resolved` at
[`template_runner.rs:1425`](../../crates/codeless-runtime/src/template_runner.rs#L1425) →
polls forever because the `checks` and `git` rows stay `Pending`.

The feature was unit-tested in isolation and the integration test
(`plugin_substrate_e2e`-style) didn't cover the full
runner→verify→commit→gate sequence. That gap merged cleanly because
the merging job (`01KRW4JNFRKPWPN2CEZ2AND2S9`) ran on the **old
binary without the gate** — it completed before the gate code was
even live.

## Evidence (so future-you doesn't repeat the diagnosis)

```sh
# 1. Both production-side caller searches return empty:
grep -rE "verify_runner::run_verify|trio_emitter::commit_stage_changes" \
    crates/codeless-runtime/src --include='*.rs' \
  | grep -v '/tests/' | grep -v 'verify_runner.rs:\|trio_emitter.rs:'
# (no output)

# 2. The only Docs caller is the only working rail:
grep -rE "emit_trio_started\(.*TodoKind::Docs|emit_trio_completed\(.*TodoKind::Docs" \
    crates/codeless-runtime/src --include='*.rs'
# crates/codeless-runtime/src/claude_runner.rs:409   (production)

# 3. Server log proof from 2026-05-18 03:09 UTC, job 01KRWGYB96G8G0DHEZEB6GAZ9H:
#   "stage trio gate waiting for checks/docs/git resolution"  -> hangs forever.

# 4. SSE stream for a hung job shows only 3 todo-added + 1 todo-updated +
#    1 todo-completed (all docs). Never any checks/git events:
JOB=<id>
timeout 3 curl -sN "http://127.0.0.1:7777/events?scope=job&job_id=$JOB&since=0" \
  | python3 -c "
import json,sys
counts={}
for line in sys.stdin:
    line=line.strip()
    if not line.startswith('data:'): continue
    try: e=json.loads(line[5:].strip()).get('event',{})
    except: continue
    t=e.get('type','')
    if t.startswith('todo-'):
        k=e.get('kind') or ''
        counts[(t,k)]=counts.get((t,k),0)+1
for (t,k),v in sorted(counts.items()): print(f'  {v} {t} kind={k!r}')"
```

## The fix — concrete touch points

Three changes in [`crates/codeless-runtime/src/template_runner.rs`](../../crates/codeless-runtime/src/template_runner.rs)
to land between the runner returning and `wait_for_trio_resolved`
(currently line 1424). Apply in this order:

### 1. Run verify (drives the `checks` trio)

Just before `wait_for_trio_resolved`:

- If the stage has any `VerifyStep`s (template's `verify:` list or
  the `verify_cmd:` sugar that wraps a one-shot step), call
  `verify_runner::run_verify(&ctx, task_id, stage_id, &steps,
  &exec, Some(&store)).await`. The function already emits
  `Checks` trio start + complete when `store` is `Some`.
- If the stage has **no** verify steps, emit
  `Event::TodoCompleted{ todo_id: checks_id, status: Skipped }`
  directly (find the existing checks-trio todo via
  `find_trio_id(&store, task_id, TodoKind::Checks)` from
  `trio_emitter.rs:33`). This is the path my smoketest hit.

The `exec` impl is the same one used elsewhere in the runtime —
`HostVerifyExec` or whichever production wrapper exists; if there
isn't one yet, add a thin `tokio::process::Command`-based exec in
`codeless-adapters-host` (R1: process spawn lives there only) and
inject through `RunnerContext`.

### 2. Commit stage changes (drives the `git` trio)

After verify, before the gate:

- Call `trio_emitter::commit_stage_changes(&ctx, &store, task_id,
  stage_id, &worktree_path, &commit_subject, &paths).await`.
- `worktree_path` is on the `RunnerContext` (or derive from `ctx.job_id`
  → `worktree_path` on the `Job` row).
- `commit_subject` matches the JOB-LOOP convention: `"stage <ordinal>:
  <stage.name>"` — see the existing handover writer for the formatting.
- `paths` is the changed-file set; the existing diff machinery
  (`diff_verify::extract_paths_from_done` consumers, or
  `git status --porcelain` via the host adapter) produces this.
- The function already routes the outcome (Ok(true) / Ok(false) /
  Err) to `Done` / `Skipped` / `Failed`.

### 3. Defensive `Skipped` fallback

After verify + commit, before entering `wait_for_trio_resolved`,
emit a `TodoCompleted{ Skipped }` for **any trio row still
`Pending`**. This is the belt-and-braces guard against a future
runner that doesn't go through `claude_runner` (e.g. `mock`, future
`codex`, `anthropic`) and so never emits the `Docs` rail. Without
this, adding a new runner re-introduces the hang.

Helper to add in `trio_emitter.rs`:

```rust
/// Skip any trio row that is still Pending at the stage's terminal
/// gate. Belt-and-braces so a runner that doesn't drive one of the
/// rails (mock, codex, anthropic) does not hang the gate forever.
pub async fn skip_pending_trio_rows(
    ctx: &RunnerContext,
    store: &SqliteStore,
    task_id: TaskId,
    stage_id: StageId,
) {
    for kind in [TodoKind::Checks, TodoKind::Docs, TodoKind::Git] {
        let Some(todo_id) = find_trio_id(store, task_id, kind).await else { continue };
        let row = store.get_todo(todo_id).await.ok().flatten();
        if !matches!(row.and_then(|r| r.status.into()), Some(TodoStatus::Pending)) {
            continue;
        }
        publish(
            ctx, stage_id, task_id,
            Event::TodoCompleted { todo_id, status: TodoStatus::Skipped },
        ).await;
    }
}
```

(Adjust to whatever the store's todo-row accessor actually exposes;
`get_todo` is illustrative.)

### Tests to add in the same diff

1. **`template_runner::tests`** — full driver test where a stage
   has no `verify` block and the runner emits no docs trio. With
   the fix, the stage's three trio rows all flip to `Skipped` /
   `Skipped` / `Skipped` and `StageCompleted{ Passed }` publishes
   within a poll cycle. Without the fix, the test hangs (use
   `tokio::time::timeout` with a 5s budget; pre-fix this expires).

2. **`trio_emitter::tests::skip_pending_trio_rows_only_skips_pending`** —
   pre-populate three rows in mixed states (Pending / InProgress / Done),
   call the helper, assert only Pending rows flipped.

3. **Existing `wait_for_trio_resolved` test at template_runner
   line ~2804** — extend to cover the `mock` runner path so the
   bug doesn't sneak back via a future template-runner refactor.

### Touch point list summary

| File | Change |
|---|---|
| `crates/codeless-runtime/src/template_runner.rs` | Add verify + commit + skip-pending block at line ~1423, before `wait_for_trio_resolved` |
| `crates/codeless-runtime/src/trio_emitter.rs` | Add `skip_pending_trio_rows`, export, test |
| `crates/codeless-runtime/src/verify_runner.rs` | (Maybe) add a `HostVerifyExec` impl if none exists, behind the `process-spawn` feature gate — R1 says spawn must live in `codeless-adapters-host`, so the actual `Command::new` lives there and `verify_runner` calls in via a trait. Check the existing impl before adding. |
| `crates/codeless-runtime/src/lib.rs` | Re-export the new helper if other crates need it |

## Out of scope for this fix

- Changing the trio-row schema or event shape (already shipped via
  `0021_todos.sql`; do not migrate again).
- Folding `claude_runner.rs:409`'s docs-trio emission into the
  `template_runner` site. Keep that where it is; the new code only
  wires the missing two rails plus the defensive skip.
- The diff-verify pre-check false-positive class (separate fix
  already in the working tree; commit separately).
- Restoring `runner: mock` actually selecting the mock runner.
  The server currently coerces mock → claude silently because no
  mock runner is registered at boot; a separate ticket.

## Restart procedure (when fix is built)

The dev server runs **outside** make's pid-file lifecycle — make's
`stop`/`restart` looks at `.codeless-dev/codeless.db` while the
real server uses `/home/user/.codeless/codeless.db`. Use the manual
incantation that matches the production args:

```sh
# 1. Find + kill the running binary
pgrep -af "target/debug/codeless.*--db /home/user/.codeless/codeless.db.*serve"
kill <pid>   # SIGTERM; sleep 2; if still alive, kill -9

# 2. Rebuild (cargo run does this automatically; explicit form):
cargo build -p codeless-cli   # ~30s incremental

# 3. Relaunch with the production args (matches what was originally running):
cd /home/user/code/rust/codeless-workspace/codeless
nohup target/debug/codeless \
  --db /home/user/.codeless/codeless.db \
  serve \
  --bind 127.0.0.1:7777 \
  --worktree-root /home/user/.codeless/worktrees \
  --driver-concurrency 4 \
  --fs-root /home/user/code/rust/codeless-workspace/codeless \
  --enable-claude \
  > /tmp/codeless-server.log 2>&1 &
echo "started pid $!"

# 4. Sanity check
sleep 3
ss -tlnp 2>/dev/null | grep 7777
curl -s -o /dev/null -w 'HTTP %{http_code}\n' \
  -X POST http://127.0.0.1:7777/rpc/list_repos \
  -H 'Content-Type: application/json' -d '{}'
# expect: HTTP 200
```

UI dev server (`pnpm dev` on port 5173) does **not** need a restart
— it picks up the wire types through Vite HMR. Only the backend
binary needs the rebuild.

## End-to-end test job

A 2-stage mock-runner job is the cheapest signal: no LLM cost,
exercises the full stage→verify→commit→gate sequence end to end.
The template that hangs on the pre-fix binary and must pass on the
post-fix binary lives at
[`.codeless/jobs/todos-smoketest/template.yaml`](../../.codeless/jobs/todos-smoketest/template.yaml):

```yaml
name: todos-smoketest
goal: |
  Two-stage smoke test exercising the todo-event pipeline.
stages:
  - title: say hello in a fresh file under /tmp and confirm it
    verify_cmd: 'true'
  - title: count to three out loud and confirm
    verify_cmd: 'true'
```

### Submit it

```sh
python3 -c "
import json, urllib.request
yaml = open('.codeless/jobs/todos-smoketest/template.yaml').read()
body = {
  'repo_id': '0000000000EM6BPA0YBTG8MR9X',  # codeless repo, list_repos to confirm
  'prompt': None,
  'template_yaml': yaml,
  'runner': 'mock',
  'branch': 'test/todos-smoketest-3',         # bump suffix on re-runs
  'cost_cap_cents': 1000,
  'wall_clock_cap_ms': 600_000,
  'start_immediately': True,
}
req = urllib.request.Request('http://127.0.0.1:7777/rpc/submit_job',
  data=json.dumps(body).encode(), headers={'Content-Type':'application/json'})
print(json.loads(urllib.request.urlopen(req).read().decode())['id'])
"
```

Note: the server currently coerces `runner: mock` → `claude` because
the mock runner is not registered. That's fine for this test — the
bug reproduces on claude too. If you'd rather drive a true mock for
speed, add `--enable-mock` to the server launch (or fix the runner
registration as a follow-up).

### Pre-fix expected (hangs)

- 3 × `todo-added` (checks / docs / git) on stage 0
- 1 × `todo-updated` (docs → in-progress)
- 1 × `todo-completed` (docs → done)
- Server log: `stage trio gate waiting for checks/docs/git resolution`
- Job status stays `running` indefinitely. Cost cap eventually trips.

### Post-fix expected (completes)

Within ~60s on the claude runner:

- 6 × `todo-added` (3 per stage × 2 stages)
- ≥ 6 × `todo-completed` (every trio row reaches a terminal state)
- 2 × `stage-completed` events
- 1 × `job-completed` event
- Job status `completed`, `stop_reason: null`

### Verify script (paste verbatim)

```sh
JOB=<paste-id-from-submit>
sleep 60
echo "--- status ---"
curl -s -X POST http://127.0.0.1:7777/rpc/get_job \
  -H 'Content-Type: application/json' \
  -d "{\"job_id\":\"$JOB\"}" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print('status:', d['status'], 'stop_reason:', d.get('stop_reason'), 'cost:', d['cost_cents'])"

echo "--- todo events ---"
timeout 3 curl -sN "http://127.0.0.1:7777/events?scope=job&job_id=$JOB&since=0" 2>/dev/null \
  | python3 -c "
import json,sys
rows={}
for line in sys.stdin:
    line=line.strip()
    if not line.startswith('data:'): continue
    try: e=json.loads(line[5:].strip()).get('event',{})
    except: continue
    t=e.get('type','')
    if not t.startswith('todo-'): continue
    k = e.get('kind') or '-'
    s = e.get('status') or '-'
    rows[(t,k,s)] = rows.get((t,k,s),0)+1
for (t,k,s),v in sorted(rows.items()): print(f'  {v}  {t:18s} kind={k:8s} status={s}')"

echo "--- visual: open in browser ---"
echo "  http://127.0.0.1:5173/jobs/$JOB"
```

Pass criteria:

1. Job status is `completed`.
2. Every trio row across both stages reaches a terminal state
   (`done` / `skipped` / `failed`) — count of `todo-completed`
   events ≥ count of `todo-added` events.
3. UI Stages tab renders both stages with the per-trio glyphs
   transitioning `○ → ● → ✓` over the run. Per
   [`StagesOverview.tsx:137-200`](../../ui/codeless-ui/src/modules/jobs/StagesOverview.tsx#L137-L200).

## Cleanup of stuck jobs from before the fix

Two hung-on-the-gate jobs exist on this machine:

```
01KRWGYB96G8G0DHEZEB6GAZ9H   (stopped manually 2026-05-18)
01KRWH3DB344S7RBTXF9BJ9B63   (stopped manually 2026-05-18)
```

Both already `stopped`. Safe to leave or `delete_job` them. Their
worktrees at `/home/user/.codeless/worktrees/job-<id>/` can be
reaped via the workspace gc affordance if disk pressure matters
(unlikely — they're tiny).

## Why not just remove the gate

Tempting (~5 lines: short-circuit `wait_for_trio_resolved` to
return `Ok(true)`), but the gate is the safety mechanism the
feature shipped to enforce. Reverting it leaves the trio events
firing but no enforcement that the closing steps actually ran —
back to the old "agent says done, nothing checked" failure mode the
job set out to fix. The wiring is the correct fix.

If you must ship a stopgap because the fix takes longer than
expected, the safe revert is a feature flag:

```rust
// At the gate site:
let trio_gate_enabled = std::env::var("CODELESS_TRIO_GATE")
    .map(|v| v != "0").unwrap_or(true);
if trio_gate_enabled {
    if !wait_for_trio_resolved(store, &ctx, stage_id, task_id).await {
        return RunnerOutcome::Failed { /* … */ };
    }
}
```

Set `CODELESS_TRIO_GATE=0` in the server systemd unit / launch
script while the wiring lands. Remove the flag in the same PR that
wires verify + commit.

## Stages

1. [ ] Diagnose + repro on a fresh worktree. Confirm both stuck jobs
   are in `stopped` state; the new branch starts from clean master.
2. [ ] Add `skip_pending_trio_rows` to `trio_emitter.rs` with the
   isolated unit test. Smallest commit; lands first.
3. [ ] Wire `run_verify` into `template_runner.rs` for the
   has-verify path. Add a `HostVerifyExec` if one doesn't exist
   (R1: spawn in `codeless-adapters-host`).
4. [ ] Wire `commit_stage_changes` into `template_runner.rs`
   immediately after verify.
5. [ ] Add the defensive `skip_pending_trio_rows` call before
   `wait_for_trio_resolved`. Together with steps 3+4, this closes
   every gate path.
6. [ ] Full driver test: 2-stage no-verify template completes within
   a 30s timeout (pre-fix: this test hangs).
7. [ ] E2E: submit the smoketest job, confirm status `completed`
   and the todo-event ledger per the verify script above.
8. [ ] REVIEW + PR. Title: `fix(runtime): wire verify + git trio
   rails into stage flow; skip pending todos on gate entry`.

## Halt conditions

- If `verify_runner::run_verify` requires a `VerifyExec` impl that
  doesn't exist in `codeless-adapters-host`, that's the load-
  bearing prereq. Stop and add the impl on its own commit before
  resuming the wiring work; don't inline it inside the wiring PR.
- If the `commit_stage_changes` call needs `paths` and the
  production code path doesn't have a clean way to enumerate
  changed paths (the existing diff-verify path is read-only),
  prefer `git status --porcelain` via the host adapter over
  re-implementing here.
- If a test in `template_runner::tests` already covers the no-verify
  path and is **passing today**, something is mocking the gate.
  Find that mock and remove it before adding the new test — the
  new test must use the same `with_store` path production uses.

## What you need to know

- The codeless server is running on a binary built from master
  commit `916e0e7` (the `feat: assistant parity` merge). The fix
  branches from there.
- The UI is on Vite at `http://127.0.0.1:5173`; the backend is on
  `http://127.0.0.1:7777`; SQLite DB is
  `/home/user/.codeless/codeless.db`.
- Repo ULID: `0000000000EM6BPA0YBTG8MR9X` (the codeless repo).
- No auth — single-user dev. Bearer token not required.
- The diff-verify false-positive fix (rejecting `self.X`,
  `MockRpcClient.method`, brace expansion) is also uncommitted in
  the working tree — commit it separately, not in this PR.
- Earlier session-side fixes to `set_job_policy` field naming and
  `EditJobDialog` reseed are believed to have landed via the
  assistant-parity PR's W3 work; spot-check before relying.

## Open questions

1. **Should `commit_stage_changes` enumerate paths itself**, or
   should the runner produce the path list and pass it in? Today's
   signature takes `&[PathBuf]`; the runner doesn't currently track
   what it touched. Bias: have the function `git status` itself
   and produce its own list; the runner can't be trusted to be
   honest about touched files, and we already pay a status call
   in the host adapter anyway.
2. **Should the trio gate emit an event when it skips a Pending
   row**, separate from the normal `todo-completed`? Bias: no —
   `todo-completed { status: Skipped }` is the existing terminal
   signal and the recorder already knows how to handle it.
3. **Mock runner registration** — currently the server silently
   coerces `runner: mock` → claude because no mock runner is
   registered. Out of scope for this fix, but worth a follow-up
   ticket because the smoketest would be 10x faster on a true mock.
