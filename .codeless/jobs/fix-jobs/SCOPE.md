# Scope — fix-jobs (multi-jobpage tab regression + spec-mode chat persistence)

## Bug 1 — blank second job-detail tab

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

## Bug 2 — spec-mode chat persistence (verify, may already work)

**Status: unconfirmed.** This may already work correctly. The stage is
to verify rather than assume it is broken.

When a job's CHAT panel is open while the job is in spec mode, confirm
that messages sent in that chat survive a page reload. Normal chat
history is known to persist correctly; spec-mode turns should behave
identically.

If verification shows persistence is broken, investigate in order:

1. **Backend not writing spec-mode turns to SQLite.** The spec-mode
   handler may respond in-memory and never call `job_event` / the
   chat-persist path. Check `codeless-runtime` event handling for
   a spec-mode branch that bypasses the store.
2. **Frontend sending to a different RPC method.** The spec-mode chat
   component may call a different `RpcClient` method that the server
   processes but doesn't persist.
3. **SSE event arrives but is not written to the DB.** The message
   appears live (via SSE) but the "persist this turn" write is
   skipped.

If verification shows it already works, record that in
§"Spec-mode chat fix verification" and close the stage — no fix
needed. If broken and the root cause is Rust-side, document it in
§"Spec-mode chat root cause" and stop; a narrowly targeted persistence
fix is in scope but a chat-architecture overhaul is not.

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
- Investigation of spec-mode chat persistence: trace the message
  lifecycle from the UI send through to SQLite. If the root cause is
  in the Rust backend, a narrowly targeted persistence fix is in
  scope; a full chat-architecture rewrite is not.

## Out of scope

- Anything in `DOCS/WORKSPACE-ATTACH.md`. Different job.
- Draft-job UX (start button, default tab, planned-stages preview).
  Different job.
- Backend / Rust changes for Bug 1. The blank-tab bug reproduces
  against `master`'s live backend; if you find a server-side cause,
  document it and stop — do not expand scope.
- A full rewrite of chat persistence or the runtime event system for
  Bug 2. The fix must be the minimum change that makes spec-mode turns
  durable.

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
2. Failing RTL test on `master`, passing after the fix (Bug 1).
3. One fix commit per distinct root cause (Bug 1).
4. **Manual verification recorded in this file** under §"Manual
   verification": two tabs open, both render, switching instant, both
   receive their own live events.
5. Root cause documented in §"Spec-mode chat root cause" with
   file:line references (Bug 2).
6. Fix commit (or backend-only note if Rust-side) for Bug 2.
7. PR to `master` linked from the final REVIEW handover.

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

## Spec-mode chat root cause

[Stage 8 fills this in: trace the message from UI send → RPC method →
runtime handler → SQLite write. Name the exact call site where
persistence is skipped, with file:line references. If the root cause
is in the Rust backend, record it here and note whether it is in
scope to fix or needs a separate job.]

## Spec-mode chat fix verification

[Stage 9 fills this in: send a message in spec-mode chat, reload the
page, confirm the message reappears. Dated, with the job ID used.]

## References

- Bug 1 originally reported live during the
  `01KRMM8EVDP2AACA3Y9Q7WR244` / `01KRMRHE7NX75PR1YJ8K49GV41`
  session — two tabs were both in the strip, one rendered, the other
  blank with `?tab=spec` in URL.
- `codeless/ui/codeless-ui/src/modules/jobs/JobDetailStack.tsx` — the
  load-bearing mount-all-toggle-visibility comment.
- `codeless/ui/codeless-ui/src/modules/jobs/JobPage.tsx` — the
  per-instance state owner. The `active` prop is what gates the
  URL-mirror effect.
- Bug 2 reported by user: spec-mode chat messages disappear on reload;
  normal chat history persists. Start investigation at
  `codeless-runtime` event/persist path and the UI RPC method used by
  the spec-mode chat panel.
