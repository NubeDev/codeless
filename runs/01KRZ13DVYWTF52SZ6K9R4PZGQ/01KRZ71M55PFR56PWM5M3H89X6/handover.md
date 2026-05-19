## Done

- Audited every `localStorage` / `sessionStorage` / zustand `persist` hit under `ui/codeless-ui/src/`; no `persist()` middleware in use, only `localStorage` (direct and via `lib/shell/kv-store.ts`).
- Classified each hit in `DOCS/STORAGE-AUDIT.md` — library-level vs tab-level — with a per-site table and rationale.
- Moved the two tab-level hits to `sessionStorage` (with key-version bumps to avoid silently importing the leaking snapshot):
- `ui/codeless-ui/src/modules/assistant/focusStore.ts` — `codeless.assistant.currentThreadId` → `codeless.assistant.currentThreadId.v2` in `sessionStorage`.
- `ui/codeless-ui/src/modules/tabs/lib/useTabs.ts` — `codeless-open-job-tabs-v3` → `codeless-open-job-tabs.v4` in `sessionStorage` via a `tabsStorage()` helper.
- `pnpm typecheck` and `pnpm test` (28 files / 145 tests) both green.
- Committed as `stage 8: per-tab storage audit …` on `codeless/workspace-scoping`.

## Next

- Stage 9 (end-to-end smoke + REVIEW gate): two Firefox tabs, two attached workspaces; verify scoped jobs / fs / events / refresh-stickiness; record evidence in `DOCS/WORKSPACE-SCOPING-SMOKE.md` per the workflow's third REVIEW gate.

## What you need to know

- `JobPage.tsx`'s `codeless-pinned-tabs:${jobId}` is intentionally left in `localStorage` — already partitioned by `jobId` (a job belongs to one workspace), so two tabs viewing different workspaces address different keys and cross-tab sharing of a *same-job* pin is desired.
- `lib/shell/kv-store.ts` and its consumers (`settings/store.ts`, `ai/lib/{agents,sessions,snippets,todos}.ts`) stayed on `localStorage` — see the rationale rows in `DOCS/STORAGE-AUDIT.md`. The `/assistant` rail is a singleton tab in `useTabs.ts`, so the "active AI session" is not a new cross-tab regression on top of today's behaviour.
- Worktree had no `bin/mani` / `mani.yaml`, so the commit went via raw git, matching the cadence of prior stages on this branch.
- Stale `localStorage` rows under the old key names are not actively cleared; the workflow says cross-tab cleanup is out of scope and the version bump prevents misuse.

## Open questions

- Whether to additionally namespace the `KVStoreAdapter` keys for `ai-sessions` / `ai-agents` `active-*` pointers per-tab once the `/assistant` rail becomes workspace-scoped. Out of scope for this job; flagged in `DOCS/STORAGE-AUDIT.md`'s library-level table.
