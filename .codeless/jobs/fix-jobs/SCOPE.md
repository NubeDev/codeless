# Scope — fix-jobs (multi-jobpage tab regression)

## Bug

Open two job-detail tabs at once (e.g. `/jobs/<A>` and `/jobs/<B>`
both in the workspace tab strip). Switching to the **second** tab
leaves the right pane **completely blank** — no page header, no inner
tab strip, no SPEC / Stages / CHAT content. Only one job-detail tab
can render content at a time; the other is a dead tab.

Reproduction:

1. Open the running `assistant` job at
   `/jobs/01KRMM8EVDP2AACA3Y9Q7WR244` — renders fine.
2. Open any second job at `/jobs/<other-id>` (a draft works) in a
   second workspace tab.
3. Click between the two tabs. One renders, the other is blank. The
   URL bar of the blank tab still reads its path; the workspace tab
   strip still highlights the right tab.

## Goal

Both (or N) `JobPage` instances mounted by `JobDetailStack` render
their own content correctly when activated. Switching is instant per
the `JobDetailStack` comment:

> "render every job-detail tab simultaneously and toggle visibility,
> so switching back is instant and per-job event subscriptions don't
> tear down"

## In scope

- The failing RTL test that exercises two `JobPage` instances against
  different jobIds simultaneously, with `MockRpcClient`.
- The fix at the actual shared-state site. Minimal, scoped, one commit
  per distinct root cause if there are several.
- Manual verification recorded in this file before the final REVIEW.

## Out of scope

- Anything in `DOCS/WORKSPACE-ATTACH.md`. Different job.
- Draft-job UX (start button, default tab, planned-stages preview).
  Different job.
- Backend / Rust changes. The bug reproduces against `master`'s live
  backend; if you find a server-side cause, document it and stop —
  do not expand scope.

## Investigation entry points (verify before fixing)

Don't trust any of these. The bug is one of them, possibly more than
one.

1. **SSE / event stream singleton.** `useEventStream` /
   `useEventStreamWithState` in
   `codeless/ui/codeless-ui/src/lib/rpc/`. If the underlying
   `EventSource` or subscription manager is module-level and keyed by
   *connection* rather than *(connection, jobId)*, the second mount's
   subscribe could swap the server-side scope and starve the first.
2. **Chat / session stores.** `useChatStore` and any zustand store
   with a "current job" or "current session" slice not keyed by
   `jobId`. A second mount would clobber the first's session pointer.
3. **`useJob(jobId)` cache.** Any shared in-flight-request map or
   global cache not keyed by `jobId` could resolve one mount's
   response into the other's setState.
4. **`activeTab` URL mirror in `JobPage`.** Each `JobPage` keeps its
   own `activeTab` state, but the URL `?tab=` mirror effect fires on
   both mounts. The inactive one is meant to bail on
   `if (!active) return;` — confirm that gate is sound. If it's
   wrong, the inactive `JobPage` rewrites the URL and confuses the
   parent's URL-to-tab logic.
5. **Outer wrapper in `App.tsx`.** The
   `<div className={cn("absolute inset-0", !isJobDetailTab && "invisible pointer-events-none")}>`
   wrapper around `JobDetailStack`. If a non-detail tab kind sneaks
   in and `isJobDetailTab` flips false mid-render, both `JobPage`s
   get hidden. Less likely but verifiable in minutes.

## Constraints

- **R2** — UI imports only `RpcClient`. No new `@tauri-apps/*` imports.
- **R3** — one file per concept. No per-shell `Foo.web.tsx`.
- **No drive-by refactors.** Regression fix, not a tab-system rewrite.
- **No global "only one JobPage at a time" hack.** The whole point of
  `JobDetailStack` is parallel mounted JobPages. Reverting that is
  not acceptable.
- **Don't disable strict-mode double-mount.** If the bug surfaces
  under React strict-mode in dev, that's a symptom of a real bug, not
  the bug itself.
- **Don't `key={jobId}`-remount as the fix.** That hides the
  underlying state-sharing bug and breaks the "instant switch"
  property.
- `pnpm typecheck` + `pnpm lint` green from
  `codeless/ui/codeless-ui/` before each commit.

## Deliverables

1. Branch `codeless/fix-jobs` off `master` (already set on the job).
2. Failing RTL test on `master`, passing after the fix.
3. One fix commit per distinct root cause.
4. **Manual verification recorded in this file** under §"Manual
   verification": two tabs open, both render, switching instant, both
   receive their own live events.
5. PR to `master` linked from the final REVIEW handover.

## Reproduction

[Stage 1 fills this in: which tab went blank, console errors, network
state for both jobIds, screenshots if useful.]

## Root cause

[Stage 2 fills this in after reading the rpc/SSE/chat-store code.
Name the exact module-level state that is shared across JobPage
instances, with file:line references.]

## Manual verification

[Stage 6 fills this in: dated, with the two job IDs used, the
sequence of clicks, and a one-line note on whether both jobs continue
to receive their own SSE events.]

## References

- Bug originally reported live during the
  `01KRMM8EVDP2AACA3Y9Q7WR244` / `01KRMRHE7NX75PR1YJ8K49GV41`
  session — two tabs were both in the strip, one rendered, the other
  blank with `?tab=spec` in URL.
- `codeless/ui/codeless-ui/src/modules/jobs/JobDetailStack.tsx` — the
  load-bearing mount-all-toggle-visibility comment.
- `codeless/ui/codeless-ui/src/modules/jobs/JobPage.tsx` — the
  per-instance state owner. The `active` prop is what gates the
  URL-mirror effect.
