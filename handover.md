# scoped-pause-points — stage 8 → stage 9 (REVIEW: UI landed)

Stage 8 (UI) landed. Stage 9 is the final REVIEW gate; the reviewer
reads this handover when deciding whether to approve.

## What landed in stage 8

### New RPC: `list_scheduled_pause_points`

A read-only lookup so the UI can render the operator-declared
schedule. Resume still goes through the existing `resume_job` RPC —
no new pause primitive landed.

- `crates/codeless-rpc/src/methods.rs` — `ListScheduledPausePointsArgs
  { job_id }` and `ListScheduledPausePointsResult { points:
  Vec<PausePoint> }`. Re-exported through `crates/codeless-rpc/src/
  lib.rs`.
- `crates/codeless-rpc/src/server.rs` — trait method on `RpcServer`.
  Returns the schedule in YAML order; `NotFound` for an unknown
  `job_id`; empty list when the job carries no schedule (predates
  the feature or template's `pause_points:` block is empty).
- `crates/codeless-runtime/src/rpc/{mod.rs,jobs.rs}` — runtime
  implementation. `jobs::list_scheduled_pause_points` runs `get_job`
  for the existence check (so the call shape mirrors `list_stages`'s
  not-found semantics) then forwards to
  `store::list_scheduled_pause_points` from stage 5.
- `crates/codeless-server/src/routes.rs` — axum route
  `POST /rpc/list_scheduled_pause_points`, behind the same bearer
  layer as every other RPC (R5).
- `crates/codeless-client/src/http_client.rs` —
  `HttpRpcClient::list_scheduled_pause_points` calls the route. The
  iOS / Android shells reach this through `codeless-client` only, so
  the mobile-safe graph still admits the new method.

### Wire types regenerated

- `crates/codeless-rpc/examples/wire_ts.rs` — registers the four new
  `PausePoint*` / `TodoSelector` types so they emit into the UI's
  generated `wire.ts`. The freshly-regenerated
  `ui/codeless-ui/src/lib/rpc/generated/wire.ts` now carries:
  - `PausePoint`, `PausePointId`, `PausePointPosition`,
    `PausePointTarget`, `TodoSelector` (specta-derived).
  - `StopReason` grows the `{ "scoped-pause-point": { "point-id":
    PausePointId } }` object variant alongside the existing string
    union members.

  **The specta/serde divergence noted in the stage-6 handover
  applies here:** specta TS spells the inner field `point-id`
  (hyphen); the runtime emits `point_id` on the wire because serde's
  enum-level `rename_all = "kebab-case"` does not rename struct
  fields inside variants. The new UI helper handles both spellings
  so the divider lookup works regardless of which producer shaped
  the payload.

### UI: `StagesOverview` planned-pause chips

`ui/codeless-ui/src/modules/jobs/StagesOverview.tsx`:

- Loads the schedule once on mount via the new RPC; resets on
  `jobId` change. Failures fall through to "no chips" silently so
  pre-recorder jobs and tests that don't seed the schedule keep
  rendering the rest of the overview.
- New `PlannedPauseChip` component: dashed border, "planned" tag,
  the point's operator-authored `reason` text (or a structural
  fallback like "pause after stage 2"). `data-testid` /
  `data-pause-point-id` / `data-pause-position` / `data-pause-fired`
  attributes pin the chip for the new vitest. Chips group per stage
  via 1-based ordinal lookup; `before` chips render above the stage
  row, `after` chips below. Stage-todo targets collapse onto their
  parent stage's chip slot (per-todo placement is a refinement
  follow-up).
- When the job is currently paused on a scoped point
  (`scopedPausePointId(job.stop_reason) === point.id`), the chip
  switches to amber and shows a `Resume` button that calls
  `resume_job` with all caps at `null` / no operator comment — the
  same surface the run-strip button uses, no new RPC. The local
  busy / error state stays on the chip so a failed call doesn't
  bleed into the stages list.

### UI: chat divider for `JobPaused { reason: ScopedPausePoint }`

`ui/codeless-ui/src/modules/chat/feed.ts`:

- `scopedPausePointId(reason)` reads the `point_id` out of the
  `StopReason` object variant, accepting both the serde-wire form
  (`point_id`) and the specta-TS form (`point-id`).
- `stopReasonLabel(reason)` formats a `StopReason` (string union or
  object) into a safe string for JSX interpolation; every existing
  site that wrote `{job.stop_reason}` directly into JSX
  (`JobChatPage`, `JobDetail`, `JobTimeline`, `RunPane`'s status
  strip) routes through this helper now — without it the new object
  variant trips TS's `ReactNode` check.
- `liveItemFromEvent` `case "job-paused"`: when the reason resolves
  to a scoped point id, emits a `lifecycle` item labelled `paused at
  scoped point <id>` (warn tone). String reasons keep their existing
  formatting.

### UI: mock client + resume-from-paused

`ui/codeless-ui/src/lib/rpc/mock-client.ts`:

- `seedScheduledPausePoints(jobId, points)` test seam; per-job map
  keyed on job id.
- `case "list_scheduled_pause_points"` arm — `not_found` for an
  unknown job, otherwise the seeded list (or empty).
- `case "resume_job"` accepts `status === "paused"` in addition to
  `stopped` / `failed`, matching the runtime's resume contract that
  the SCOPE Q4 calls out. Without this the chip's Resume click
  would 409 against the mock.

### Tests (vitest + RTL)

The project doesn't ship Playwright today — the "Playwright test"
the template names is the vitest+RTL surface every other UI test in
the tree uses (vitest browser-playwright transport is in the
lockfile but not configured). Coverage lands as two test files:

- `ui/codeless-ui/src/modules/jobs/StagesOverview.test.tsx`
  (extended):
  1. `renders a planned-pause chip per scheduled point in YAML
     order` — seeds two points (before stage 1, after stage 2),
     renders, emits `stage-started` for both stages, asserts both
     chips appear with the right `data-pause-point-id` /
     `data-pause-position`, the operator-authored reason wins as
     the label for chip 1, and the structural fallback ("pause
     after stage 2") wins for chip 2.
  2. `surfaces a Resume button when paused on a scoped point and
     clears the pause on click` — seeds the job into `paused` with
     `stop_reason = { "scoped-pause-point": { point_id } }`, asserts
     the matching chip's `data-pause-fired` flips to `true` and a
     `Resume` button appears, clicks it, asserts the mock's job row
     flipped back to `queued` and `stop_reason = null`.

- `ui/codeless-ui/src/modules/chat/feed.scopedPause.test.ts` (new):
  - `scopedPausePointId` reads both wire shapes and returns `null`
    for the string variants.
  - `liveItemFromEvent` projects the scoped reason into a distinct
    `paused at scoped point …` divider while keeping the legacy
    `user` / `cost-cap` labels unchanged.
  - `stopReasonLabel` formats the object variant (so the wire
    object never lands as raw JSX) and pass-throughs the string
    variants.

Plus the existing 118 vitest cases stayed green after the
`stop_reason`-JSX-shape refactor.

## Verify

- `cargo test --workspace` — green (one flake on
  `codeless-adapters-host`'s
  `git_revert_undoes_an_earlier_commit_and_returns_new_sha` when
  the lib tests run in parallel; passes deterministically with
  `--test-threads=1`, unrelated to this stage).
- `cargo clippy --workspace --all-targets -- -D warnings` — green.
- `cargo fmt --check` — green.
- `pnpm test` (from `ui/codeless-ui/`) — **23 files, 118 tests
  passed**. Eight tests are new (two for the chip rendering + resume
  click in `StagesOverview.test.tsx`, six across
  `feed.scopedPause.test.ts`).
- `pnpm run typecheck` — green.

## What stage 9 (final REVIEW) needs to assess

- New wire surface: `list_scheduled_pause_points` is the only new
  RPC; everything else routes through `resume_job` and `JobPaused`.
- The `stop_reason` JSX-shape refactor touches four pre-existing
  files (`JobChatPage.tsx`, `JobDetail.tsx`, `JobTimeline.tsx`,
  `RunPane.tsx`) — each call site now goes through
  `stopReasonLabel`, no behaviour change for the string variants.
- R2 check: a fresh `rg '@tauri-apps/api' ui/codeless-ui/src/ -g
  '!src/shells/**'` returns the same baseline as before the stage
  (no growth). The chip + divider read through `RpcClient` only.
- R1 check: no new `tokio::process` / `std::process::Command` in any
  crate; the UI work is server-side schedule lookup + DOM rendering
  only.
- specta/serde `point_id` vs `point-id` is still the known
  divergence; the UI absorbs both via `scopedPausePointId`. A
  follow-up that aligns the runtime's serde output to specta (or
  vice versa) would let us drop the fallback branch.

## Out-of-scope follow-ups (file as separate jobs per SCOPE §"Open
follow-ups")

- Edit-points-from-UI: operators still edit `template.yaml` (direct
  or chat-driven on-disk path). The chip is read-only.
- Recurring / count-based breakpoints, conditional / predicate
  breakpoints.
- Per-todo chip placement: today a `StageTodo` target renders as a
  chip on the parent stage row. Inline-with-the-todo placement is a
  layout refinement, not a wire change.
- `pause_points_updated` event for divider chips to refresh without
  re-reading the whole job state. The mount-time fetch is sufficient
  for v1 because resync edits land while the job is paused and the
  divider lookup uses the live `stop_reason`.
- True Playwright browser test once a Playwright harness lands in
  `ui/codeless-ui/`; vitest+RTL exercises the same surface today.
