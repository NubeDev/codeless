# JOB-DIR kickoff prompt

Paste the fenced block below into a fresh Claude Code session pointed at
`/home/user/code/rust/codeless-workspace`. Spec: [JOB-LOOP.md](./JOB-LOOP.md).
Design intent for the work: [JOB-DIR.md](./JOB-DIR.md).

The prompt resets the inner `codeless/` repo to a clean `master` before
the loop starts, so any uncommitted scratch from prior sessions is
discarded. The new files left over from the destroyed session
(`crates/codeless-runtime/src/job_dir.rs`,
`crates/codeless-runtime/tests/job_dir_workflow.rs`,
`ui/codeless-ui/src/modules/jobs/JobFilesPane.tsx`) are **not** kept —
the loop rebuilds them from the design in JOB-DIR.md as part of its
own stages, so the agent's diff matches its commit log.

Date for the status file: `2026-05-13`. Slug: `job-dir`.

## The prompt

```
You are running JOB-LOOP per DOCS/JOB-LOOP.md.

Repo:        codeless
Branch:      feat/job-dir
Status file: DOCS/sessions/2026-05-13-job-dir.md
Spec:        DOCS/JOB-DIR.md   # the design for this loop; the source of truth
                               # for layout, RPC surface, UI shape, migration
Rules file:  CLAUDE.md         # repo root; rules an agent must follow
Phase:       Phase 3 — browser MVP iterate-loop (extends JOB-WORKFLOW.md)
Goal:        Land the job-as-directory feature end-to-end: directory
             layout for .codeless/jobs/<name>/, four file-surface RPCs,
             flat→dir migration on first write, scope/workflow/extras
             folded into the per-stage prompt, and a Spec pane in the
             UI that lets the user edit every file with CodeMirror.

Before tick 1 — clean slate setup (do this once, then start the loop):
  - cd codeless
  - git fetch origin
  - git checkout master
  - git reset --hard origin/master    # discard any uncommitted scratch
  - git clean -fdx                    # nuke untracked files too
  - git checkout -b feat/job-dir
  - cd ..                             # back to workspace root for mani
  - Confirm `git -C codeless status` is clean and on feat/job-dir.

Stages (ordered, each tagged [S|M|L]):
  1. [S] Add codeless-runtime/src/job_dir.rs — resolver, sanitiser,
         read_docs_for_prompt — with 7 unit tests covering resolve
         (none/flat/directory + flat-preferred-when-both),
         list_markdown (md-only filter), sanitise (path traversal,
         dotfiles, template.yaml reserved, .md auto-append).
         Wire it into crates/codeless-runtime/src/lib.rs.

  2. [S] Add the JOB-DIR wire types to codeless-rpc/src/methods.rs:
         ListJobFilesArgs / ListJobFilesResult / JobFileEntry /
         ReadJobFileArgs / ReadJobFileResult /
         WriteJobFileArgs / WriteJobFileResult / DeleteJobFileArgs.
         Re-export from src/lib.rs. Add the four method signatures
         to RpcServer in src/server.rs. Register the new types in
         crates/codeless-rpc/tests/specta_snapshot.rs and regen the
         wire-rpc.ts.snap with SPECTA_UPDATE=1.

  3. [M] Implement the four RPCs in codeless-runtime/src/rpc.rs:
         - list_job_files: resolve, scan, return template.yaml first
           then *.md in filename order with is_scope/is_workflow/
           is_template flags.
         - read_job_file: sanitise filename, refuse traversal, read.
         - write_job_file: sanitise filename, refuse "template.yaml"
           (reserved), promote flat→directory on first write
           (move flat YAML to <name>/template.yaml, delete the flat
           file, with separate commits for each migration step),
           write the new file, commit "update job-file: <name>/<filename>".
         - delete_job_file: sanitise, refuse "template.yaml",
           delete, commit "delete job-file: <name>/<filename>".
         Helpers: resolve_repo_and_template_name(job_id) →
         (repo_path, template_name), filename_err() → RpcError.
         Update update_job_template in the same file to:
         - write to <repo>/.codeless/jobs/<name>/template.yaml
         - if a flat <name>.yaml exists, delete it in a second commit
           "migrate template: <name> → directory layout"
         - keep the rename guard, keep the DB row refresh.

  4. [S] Wire the four routes into codeless-server/src/routes.rs.
         Wire the four methods into codeless-client/src/http_client.rs
         (HttpRpcClient).

  5. [S] Update codeless-runtime/src/job_driver_loop.rs prompt builder:
         after the handover prefix and before the notes/original, call
         crate::job_dir::read_docs_for_prompt(&repo_path, &template.name)
         and fold the result in. Only template-style jobs (template_yaml
         present + parseable) trigger this. Order matches JOB-DIR.md
         "How the agent reads the docs": handover, job docs, notes,
         original prompt.

  6. [M] Add crates/codeless-runtime/tests/job_dir_workflow.rs with
         6 integration tests:
         - list_job_files_reports_none_layout_until_first_save
         - update_job_template_creates_directory_layout
         - write_job_file_adds_scope_md_and_commits
         - write_job_file_migrates_flat_layout_in_place
         - delete_job_file_refuses_template_yaml
         - read_job_file_rejects_path_traversal
         Update the pre-existing job_workflow.rs test if it asserted
         the old flat path (it should now expect the directory).

  7. [S] TS surface: add the new wire types and method-map entries to
         ui/codeless-ui/src/lib/rpc/methods.ts. Re-export through
         src/lib/rpc/index.ts.

  8. [M] Add ui/codeless-ui/src/modules/jobs/JobFilesPane.tsx — the
         Spec pane with the two-pane layout from JOB-DIR.md "The UI":
         file list (left), CodeMirror editor (right), + file dialog
         with SCOPE.md / WORKFLOW.md presets when missing, delete
         button (× on hover, except for template.yaml), legacy-flat-
         layout hint at the top when layout === "flat". Use the
         existing InlineCodeEditor component for the editor surface.

  9. [S] Rewire ui/codeless-ui/src/modules/jobs/JobPage.tsx:
         - Rename the "yaml" section id to "spec".
         - Rename the label from "Template" to "Spec".
         - Swap TemplateEditor for JobFilesPane in the section render.
         - Drop the unused templatePathFor helper.
         - Delete ui/codeless-ui/src/modules/jobs/TemplateEditor.tsx
           if it still exists in the working tree.

 10. [S] Mock-client parity: add list_job_files / read_job_file /
         write_job_file / delete_job_file cases to
         ui/codeless-ui/src/lib/rpc/mock-client.ts. Add a private
         jobFiles: Map<string, Map<string, string>> field. Include
         a normaliseJobFilename helper mirroring the runtime rules.
         Reserve template.yaml for both write and delete.

Sizing reminder:
  S = mechanical, ≤ ~15 min, low risk.
  M = real thinking, one focused area.
  L = MUST be split into S/M sub-stages before being worked.

Scheduler: CronCreate one-shot, ~1 min between ticks
Max ticks: 30

Batching rule (do as much as fits in ONE tick — the user wants big
sessions, so batch aggressively where the rule allows):
  - up to 4 contiguous S stages in the same area, OR
  - 1 M stage (+ optionally 1 closely-related S), OR
  - the next sub-stage of an L.
  Stop the batch on any failure or if the diff exceeds the plan.

  Hints for this loop, given the stages above:
   - Tick 1: stage 1 (S) + stage 2 (S) — both touch the wire surface
     in adjacent crates; one verify pass.
   - Tick 2: stage 3 (M) alone — biggest stage, four RPCs + migration.
   - Tick 3: stage 4 (S) + stage 5 (S) — routes + client + prompt
     builder, all small wiring.
   - Tick 4: stage 6 (M) alone — the integration tests.
   - Tick 5: stage 7 (S) + stage 8 (M) — TS types then the Spec pane.
   - Tick 6: stage 9 (S) + stage 10 (S) — JobPage rewire + mock parity.
  These are hints, not commitments. The batcher decides per tick.

Procedure each tick:
  - Pre-flight: clean tree (in codeless/), parse status file,
    all-done check. NOTE the setup block above ran ONCE before tick
    1 — every later tick assumes a clean tree and the feat/job-dir
    branch.
  - Decide the batch from the next [ ] stages and their tags.
    Announce it in chat in one line ("batch: stages X, Y").
  - For EACH stage in the batch, in order:
      plan -> implement (minimum diff, follow CLAUDE.md and the
      design in JOB-DIR.md) ->
      verify:
        Rust:  cargo test --workspace, cargo clippy --workspace
               --all-targets -- -D warnings, cargo fmt --check
        TS UI: pnpm -C codeless/ui/codeless-ui tsc --noEmit
        Wire types: SPECTA_UPDATE not needed during verify; run the
                    specta snapshot test plain and confirm it passes.
                    If it fails, the snapshot regen was missed in
                    stage 2 — fix that, do not bypass.
      ->
      update the status file (check box, move ← next, bump
      Last tick + Current stage, Notes line if non-obvious) ->
      commit AND push via mani:
        ./bin/mani --config mani.yaml run commit --projects codeless MSG='stage N: <title>'
        ./bin/mani --config mani.yaml run push --projects codeless
      Push is required per stage, not just at end of tick.
  - If all stages [x] -> report DONE, do NOT reschedule.
  - Else: SCHEDULE THE NEXT TICK. Call CronCreate with
    recurring:false and a 5-field local-time cron expression for
    ~1 minute from now (pick an off-minute, not :00 or :30). Pass
    this exact prompt verbatim as the `prompt` arg.
    If CronCreate is unavailable or returns an error, DO NOT exit
    silently — follow JOB-LOOP.md "If you cannot schedule": say
    which stage finished, why scheduling failed, and how to re-kick.
  - Halt without rescheduling on any guardrail trip in JOB-LOOP.md
    (untagged stage, dirty tree, verify fail, push fail, cross-platform
    rule violation — UI imports of @tauri-apps/api/core, process-spawn
    in a non-host crate, mobile-unsafe code in a mobile-reach crate)
    and explain why in chat before exiting.

If the status file does not exist yet, create it at
DOCS/sessions/2026-05-13-job-dir.md, populate from the stages above
with [S|M|L] tags, include the AGENT REMINDER block from JOB-LOOP.md
"The status doc" section verbatim, commit and push as
"stage 0: init status", then begin tick 1.
```

## Re-kick hint

If the loop halts mid-way (verify fail, push fail, the operator
interrupts), paste the same prompt again. The pre-flight will see
the existing `feat/job-dir` branch and the existing status file, skip
the setup block (the branch already exists; `git checkout -b` would
fail), and pick up from the next `[ ]` stage.

If you need a hard reset back to master, run the setup block by hand
in a shell first, then paste the prompt — the loop will see the
clean tree and an existing status file in the previous branch's
worktree, which it should refuse to use. In that case delete the
status file before pasting:

```sh
rm DOCS/sessions/2026-05-13-job-dir.md
```

and the loop will recreate it from stage 0.
