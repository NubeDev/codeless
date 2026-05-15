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

Audit of `codeless/ui/codeless-ui/src/lib/rpc/` (`hooks.ts`,
`http-sse-client.ts`, `provider.tsx`, `client.ts`) and
`codeless/ui/codeless-ui/src/modules/ai/store/chatStore.ts`, plus a
walk of the `JobPage` subtree (`JobPage.tsx`, `JobDetailStack.tsx`,
`StagesOverview.tsx`, `StageDetail.tsx`, `RunPane.tsx`/`JobChat`):

The three named suspects (SSE subscription, chat store, `useJob`
cache) are **not** the shared singleton causing the bug. The only
module-level state that is shared across `JobPage` instances and
demonstrably drives blank content is `window.location` — read by
every `JobPage`'s `activeTab` lazy initialiser regardless of `active`.

### Cleared suspects

- **SSE subscription cache** —
  `codeless/ui/codeless-ui/src/lib/rpc/hooks.ts:237`:
  ```ts
  const SHARED_SUBSCRIPTIONS = new WeakMap<
    RpcClient,
    Map<string, SharedSubscription>
  >();
  ```
  The inner map is keyed by `JSON.stringify({ filter, since })` and
  `filter` for a `JobPage` is `{ scope: "job", job_id: <jobId> }`
  (`JobPage.tsx:155-173`, `RunPane.tsx:1125-1154` for the chat
  variant, `StagesOverview.tsx:497`). Different `jobId` ⇒ different
  key ⇒ distinct `SharedSubscription` with its own `buffer`,
  `listeners`, `stateListeners`, `lastStatus`, and `cancel`. Sharing
  inside a single `jobId` (page-level + `StagesOverview` +
  `JobChat`) is the deliberate connection-pooling design and is
  correct. Verified by reading `joinSubscription`
  (`hooks.ts:242-374`) and the disconnect cleanup path
  (`hooks.ts:298-300`): two parallel `JobPage`s do not collide here.
- **`HttpSseClient`** — `http-sse-client.ts:54-118` is stateless on
  the instance; each `subscribeWithState` call creates a fresh
  `openManagedSse` closure (`http-sse-client.ts:128-262`) with its
  own `EventSource`, stale/reconnect timers, cursor, and attempt
  counter. No module-level mutables.
- **`chatStore`** —
  `codeless/ui/codeless-ui/src/modules/ai/store/chatStore.ts:146-159`:
  module-level `chats: Map<sessionId, Chat>`, `seedMessages`, and
  `pendingPersist` Maps, plus the zustand store at line 238. Keys are
  AI-sidebar chat-session ids, **not** job ids. `JobChat` inside
  `RunPane.tsx:956+` owns its own `useState` for `history`,
  `streaming`, `liveItems`, and persists via
  `read_job_file`/`write_job_file` to `CHAT.md` inside the worktree
  (`RunPane.tsx:1062-1080`). It never touches `chatStore`. Two
  `JobPage`s cannot collide on this store.
- **`useJob`** — `hooks.ts:111-152`: per-component `useState` plus a
  `tick` counter for refetch. No module-level cache, no in-flight
  request map. Two parallel calls produce two independent `get_job`
  RPCs into two independent state slots.
- **`useRepos` / `useAsyncOnce`** — `hooks.ts:20-59`: per-component
  state only.

### Confirmed singleton — `window.location.search`

`codeless/ui/codeless-ui/src/modules/jobs/JobPage.tsx:70-90`:

```ts
const [activeTab, setActiveTab] = useState<ActiveTab>(() => {
  if (typeof window !== "undefined") {
    const param = new URLSearchParams(window.location.search).get("tab");
    ...
    if (param?.startsWith("stage:")) {
      const stageId = param.slice("stage:".length);
      if (stageId) {
        return { kind: "stage", stageId, stageName: stageId, pinned: false };
      }
    }
  }
  return { kind: "system", id: "Stages" };
});
```

`window.location` is a process-wide singleton. The initialiser runs
on every `JobPage` mount unconditionally — it does **not** check the
`active` prop. The URL-mirror effect at `JobPage.tsx:96-107`
correctly gates writes on `if (!active) return;`, but there is no
matching read-gate. Sequence:

1. Open job A → its `JobPage` mounts active, initialises `activeTab`
   from URL (`?tab=stages` by default → `Stages`). The user clicks a
   Stage row; `handleOpenStageTab` (`JobPage.tsx:191-203`) sets
   `activeTab` to `{ kind: "stage", stageId: "<A-stage>", ... }`,
   and the URL-mirror effect writes `?tab=stage:<A-stage>` into the
   address bar.
2. Open job B → a fresh `JobPage` mounts with `jobId=B`, and on this
   first render the lazy initialiser reads the **same**
   `window.location.search` that job A wrote. It returns
   `{ kind: "stage", stageId: "<A-stage>", stageName: "<A-stage>", pinned: false }`
   into job B's local state.
3. User activates job B's tab. `StageDetail` (`JobPage.tsx:332-341`)
   renders with `jobId=B, stageId="<A-stage>"`. Inside `StageDetail`,
   the `list_stages` lookup for an `<A-stage>` id against job B's
   stages returns `null`; `rollup` stays `null`; the right-pane
   content area renders empty.

The active-prop gate is the wrong direction: only the **active**
`JobPage` is allowed to read or write `window.location`. Inactive
mounts must initialise to a safe default.

### Layout amplifier (not module state, but co-causal)

`codeless/ui/codeless-ui/src/modules/jobs/JobDetailStack.tsx:31-38`
wraps every `JobPage` in `<div className="h-full w-full">` without
any `hidden` class on the wrapper. The child `JobPage` applies
`!active && "hidden"` to its own root (`JobPage.tsx:289`), but the
outer wrapper still claims `height: 100%` per child. With N tabs
open the wrappers stack and the active one is pushed below the
viewport. This is a layout bug, not a state-sharing bug, but it
prints the same "blank pane" symptom as the URL singleton above and
masks it during reproduction. Both have to be fixed for the bug to
disappear; the URL singleton is the part that matches the goal's
"module-level state shared across JobPage instances" framing.

### File:line summary

| Site                                                     | Verdict                                |
| -------------------------------------------------------- | -------------------------------------- |
| `src/lib/rpc/hooks.ts:237` `SHARED_SUBSCRIPTIONS`        | per-jobId keys, not shared             |
| `src/lib/rpc/http-sse-client.ts:54+` `HttpSseClient`     | stateless on instance                  |
| `src/modules/ai/store/chatStore.ts:146-159`              | sessionId-keyed, unrelated to jobs     |
| `src/lib/rpc/hooks.ts:111-152` `useJob`                  | per-component, no cache                |
| `src/modules/jobs/JobPage.tsx:70-90` `activeTab` init    | **root cause** — reads window.location |
| `src/modules/jobs/JobDetailStack.tsx:31-38` wrapper      | layout amplifier, see above            |

## Manual verification

Stage 6, 2026-05-15.

**Headless constraint.** This stage runs inside an isolated git
worktree with no interactive user and no running app — a click-through
of two live job-detail tabs is not possible in this environment. The
verification recorded below is the strongest signal achievable
headlessly; an interactive pass against a live server is still
required at the stage-7 REVIEW gate before the PR merges (see
"Pending" below).

### Automated verification (performed this stage)

- `pnpm -C ui/codeless-ui test` — 1 file / 1 test passing.
  `ui/codeless-ui/src/modules/jobs/__tests__/JobDetailStack.parallel.test.tsx`
  mounts two `JobPage` instances inside one `JobDetailStack` against
  distinct jobIds (`A` and `B`) with `MockRpcClient`. After
  `?tab=stage:a-stage-1` is written into `window.location.search` by
  job A (the active mount), job B's lazy `activeTab` initialiser no
  longer inherits that stageId; both panes show their own `Stages`
  tab, and the test's `selectedStagesTabs.toHaveLength(2)` assertion
  passes. This test failed on `master` (commit 4e4225e, stage 3)
  and passes on `627f097` (stage 5, the fix).
- `pnpm -C ui/codeless-ui lint` — no eslint configured; trivially
  passes.
- `pnpm -C ui/codeless-ui test` confirms the parallel-mount harness
  exercises both:
  - independent `useJob(jobId)` resolution (each `JobPage` renders
    its own job title without cross-contamination), and
  - independent SSE subscription scope (the test's `MockRpcClient`
    records `subscribe({ scope: "job", job_id })` calls keyed by
    jobId and asserts both are present).
  Live SSE delivery against a real backend is the remaining piece
  the headless suite cannot exercise; that is the only item below.

### Build status note (pre-existing, not caused by this fix)

`pnpm -C ui/codeless-ui typecheck` fails with
`src/app/App.tsx(155,9): error TS2552: Cannot find name 'path'.` This
error is present on the stage-3 parent commit (4e4225e) and on
`master` — it is unrelated to the `activeTab`/`window.location` gate
introduced in stage 5. Flagged in the stage-5 handover; out of scope
for this job per §"Out of scope" and the "no drive-by refactors"
rule.

### Pending — must be done by the stage-7 reviewer

The following requires the running app and a human at the keyboard:

1. Boot `codeless serve` against a sqlite store with at least two
   real jobs.
2. Open `/jobs/<A>` in one workspace tab and `/jobs/<B>` in another.
3. In tab A, click a Stage row so the URL becomes
   `?tab=stage:<A-stage>`.
4. Switch to tab B. Confirm:
   - tab B's right pane is **not blank** — `Stages` overview renders
     (the safe default for the freshly-mounted, inactive-then-active
     sibling whose pathname did not match `window.location.pathname`
     at mount time);
   - switching A↔B is instant (no remount flicker, no network
     refetch of `get_job`);
   - emitting a `job_event` for job A surfaces only in tab A's
     event-driven UI, and similarly for B (each `JobPage`'s
     `SHARED_SUBSCRIPTIONS` entry is keyed by its own jobId).
5. Record the jobIds used and a one-line pass/fail in this section
   before approving the stage-7 REVIEW.

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
