# Assistant follow-ups (F2 → F3 → F1)

Branch:      codeless/fix-ai-agent (inner) — outer workspace is master
Status file: this file
Spec:        DOCS/ASSISTANT-SCOPE.md (§Follow-ups)
Goal:        a user can chat at `/assistant` (or from the footer
             `AiInputBar`), receive a streamed model reply, confirm
             action cards that mutate job state via real RPCs, and
             find one SQLite-backed source of truth behind every
             chat surface.

## Stages

1. [x] [S] Survey F2/F3/F1; map code anchors; record cwd answer + open questions.
2. [x] [M] F2 — replace `NOOP_ASSISTANT_REPLY` with an `agent_chat`-driven planner that emits `AssistantActionCard` cards. Streams via the existing event bus tagged with the per-turn session id.
3. [x] [M] F2 — feed thread history + tool-result transcript into the prompt; persist the assistant turn (+ any cards) on completion so the `/assistant` view replays it via `list_assistant_messages`.
4. [x] [S] F3 — add `jobs.draftFromConversation(thread_id)` and `jobs.updateScope(job_id, new_markdown)` to `codeless-rpc::RpcServer`; wire to existing `submit_job` / `write_job_file` paths and enforce the paused-job rule on `updateScope`.
5. [x] [S] F3 — route confirmed `draft_job` and `edit_scope` cards through the two new methods (replacing the inline `submit_job`/`write_job_file` calls in `dispatch_action`).
6. [x] [S] F3 — `MockRunner` dispatch tests + reject-on-running-job test + round-trip test that a confirmed `start` card calls `start_job` exactly once.
7. [x] [M] F1 — drive `AiInputBar` from the current assistant thread (last-used or pinned); messages sent from the footer appear in `/assistant`'s transcript on next render.
8. [x] [S] F1 — collapse `useChatStore` message ownership: either retire it or shrink to UI presentation state (scroll, composer draft) so SQLite stays the only source of truth.
9. [x] [S] Tighten: run `cargo test --workspace`, clippy, fmt; `tsc --noEmit` + vitest in `ui/codeless-ui`; regenerate wire types if any RPC arg shape changed.

## Stage 1 — survey notes

### Code anchors located

- **Planner stub**: `codeless/crates/codeless-runtime/src/rpc/assistant.rs:182` (`NOOP_ASSISTANT_REPLY`). The full `append_assistant_message` body (lines 185-262) does a slash-command parser fallback (`parse_action`, lines 289-412) that already produces `AssistantActionCard`s for `list/get/start/stop/pause/resume/restart/update/draft/edit-scope`. F2 replaces the no-op branch with a real model loop while keeping the slash parser as a deterministic fallback (or retiring it — see open questions).
- **Existing tool dispatcher**: `assistant.rs:715-908` (`dispatch_action`). Covers every action F3 enumerates. Two arms call existing RPCs directly today — `DraftJob` → `submit_job` (line 821), `EditScope` → `read_job_file` + `write_job_file` (lines 872, 885). F3 introduces named wrappers and routes through them.
- **Confirm/cancel**: `assistant.rs:910-982` (`confirm_assistant_action`, `cancel_assistant_action`). Already idempotent in the "non-pending → InvalidArgument" sense via `load_pending_card` (line 622).
- **`agent_chat` RPC**: `codeless/crates/codeless-runtime/src/rpc/chat.rs:34-174`. Streams events tagged with `args.session_id`; the per-call `cwd` override is validated against `fs roots ∪ registered-repo local_paths` (lines 46-82). Default cwd is `agent_chat_cwd` (`mod.rs:68`), set once on runtime construction.
- **`agent_chat` trait method**: `codeless/crates/codeless-rpc/src/server.rs:247` (description at 235-247).
- **Frontend transport**: `codeless/ui/codeless-ui/src/modules/ai/lib/cliRunnerTransport.ts:65-158` (`createCliRunnerTransport`). Mints a per-turn `session_id`, subscribes to `EventFilter::Job{job_id: session_id}` first, then fires `agent_chat`. Maps `ai-token` / `tool-call` / `ai-message-complete` envelopes to `UIMessageChunk`. Assumes a single text block per turn.
- **Assistant view**: `codeless/ui/codeless-ui/src/modules/assistant/AssistantThreadView.tsx`. Append at line 75-101 (`onSubmit` → `append_assistant_message`); confirm/cancel at lines 108-148; renders action cards from `meta_json` (line 270-281, `parseActionCard`). Currently has no subscription — it just appends user/assistant rows from the synchronous `append_assistant_message` result.
- **Footer composer**: `codeless/ui/codeless-ui/src/app/App.tsx:20-27` imports `AiInputBar` + `AiInputBarConnect`; `useChatStore` is read from line 238 down. The composer is currently wired to the in-editor AI chat via `useChatStore`, not to assistant threads.
- **Server bootstrap**: `codeless/crates/codeless-cli/src/serve.rs:286-296` sets `agent_chat_cwd = std::env::current_dir()` and calls `runtime.with_agent_chat(registry, cwd)`. There is no separate `with_assistant_chat_cwd` today.

### Runner-side cwd for assistant threads

The spec (§F2 last bullet) says "pick the workspace root as cwd since
threads are workspace-scoped". Current state:

- `codeless-server` sets `agent_chat_cwd` to `std::env::current_dir()`
  on startup (`serve.rs:287`). When the operator launches the server
  from the workspace root, that is *coincidentally* the workspace root.
  Nothing on the runtime side enforces or documents this.
- Per-call cwd overrides are accepted on `agent_chat` (chat.rs:46),
  validated against the fs-root + registered-repo allowlist (chat.rs:64).
- **Decision for F2**: keep one knob, the runtime-level default. The
  assistant turns call `agent_chat` without setting `args.cwd`, so they
  inherit `agent_chat_cwd`. The CLI sets it to the canonicalised
  workspace root explicitly (rather than `current_dir()`), and the
  fs-roots include that path so `agent_chat`'s allowlist accepts it.
  Per-job chat keeps overriding `cwd` to the job's worktree — already
  works today.
- Single-tenant trust boundary (R5) means there is one workspace per
  server process. No per-thread cwd needed; the default is correct for
  every assistant thread on the host.

### Confirmed: F3 is mostly a renaming exercise

The `dispatch_action` switch (assistant.rs:715-908) already implements
every capability F3 enumerates. The remaining work is:

1. Promote the inline `submit_job` (for `DraftJob`) and `write_job_file`
   (for `EditScope`) calls to first-class RPC methods on `RpcServer`
   (`jobs.draftFromConversation`, `jobs.updateScope`) so external
   clients — including the CLI and future native shells — can hit them
   without going through the assistant message surface.
2. Move the paused-job guard (assistant.rs:854-865) into
   `update_scope`'s implementation so it holds for every caller, not
   just the assistant.
3. Re-point `dispatch_action`'s `DraftJob` / `EditScope` arms at the
   new methods.

That's why F3 is two `[S]` stages, not the `[M]` it would be if the
dispatcher didn't exist.

### F1 — what changes about the footer

- Footer composer today uses `useChatStore` (App.tsx:238+). Messages
  live in browser state only; tool cards live in `chatStore.ts`.
- F1 binds it to the **current assistant thread** (last-used or pinned
  in `localStorage` until we add a `assistant.preferred_thread_id` row).
  Submissions go through `append_assistant_message` (post-F2 that
  triggers the planner), not the `agent_chat` direct path the AI panel
  uses today.
- Action cards that need full-width rendering surface a compact
  "open in /assistant to confirm →" link in the footer; full rendering
  stays on the `/assistant` route. The composer is for input + short
  responses.
- Both surfaces read messages from `list_assistant_messages` so the
  footer and `/assistant` transcripts are the same data, not two
  buffers. Live updates need a subscription channel for assistant
  thread messages — currently we only get them via the synchronous
  `append_assistant_message` return value, which the footer cannot
  rely on (the user might be on a different tab when a card lands).
  See open questions.

## Open questions (to resolve before stage 2 starts)

1. **Slash parser fate.** After F2 wires the real planner, do we keep
   `parse_action` as a deterministic fast-path (so `/start <job_id>`
   bypasses the LLM) or retire it entirely? Bias: keep it. Free, fast,
   and the muscle-memory shortcut is useful for power users. The
   planner falls through to slash-parse only if the input starts with
   `/`; otherwise the LLM owns the turn.
2. **Live assistant transcript channel.** `AssistantThreadView` does
   not subscribe today — it relies on the synchronous response. Once
   the planner streams tokens, the view needs a `subscribe` channel
   keyed on `thread_id` (or on a per-turn session id, like
   `agent_chat`). Pick one before F2 lands. Bias: per-turn session id,
   reusing the `agent_chat` envelope shape so the footer (F1) can
   subscribe to the same stream as `/assistant` without a second wire
   format. The thread id becomes the persistence key, not the
   subscription key.
3. **`agent_chat` registry for assistant turns.** Today `agent_chat`
   accepts a `runner: "claude" | "codex" | "copilot"`. Assistant
   threads need to pick *something*; the right default is probably
   `claude` (matching the project's primary CLI). A per-thread
   `preferred_runner` column is a future revision — for F2 we hard-code
   the default and surface it in the proposal card.
4. **Card carry across the stream.** The planner has to emit cards
   *within* a streamed turn (model invokes a tool, runtime renders
   `AssistantActionCard` and inserts the row, the stream continues with
   the model's surrounding text). Two shapes possible: (a) cards are
   persisted as their own message rows interleaved with the assistant
   token row; (b) the assistant token row carries a sibling list of
   card ids in `meta_json`. Bias: (a) — keeps the existing
   `meta_json: AssistantActionCard` shape and means
   `list_assistant_messages` returns them in order without UI gymnastics.
5. **Workspace-root cwd on multi-workspace hosts.** R5 says one trust
   boundary; the server already assumes one workspace per process.
   This survey assumes the same. Worth a one-line confirmation in the
   F2 design before we hard-code `current_dir` → `workspace_root`
   resolution in `codeless-cli`.

## Stages 7-8 — F1 footer rewire notes

The footer composer in `app/App.tsx` was rebuilt around a new
`AssistantFooterBar` component (`modules/assistant/AssistantFooterBar.tsx`).
Key choices:

- **Focus pointer in a tiny zustand store** (`modules/assistant/focusStore.ts`,
  `useAssistantFocus`). Persists `currentThreadId` to localStorage so a
  reload re-binds the footer to the last thread; the `/assistant` rail
  writes the same store when its selection changes. SQLite stays the
  source of truth — the store only holds the pointer + a `refreshTick`
  used as a re-fetch signal in lieu of a per-thread subscription
  channel (open question §2 — still deferred). When the planner gets a
  proper streamed subscribe path (future revision), `refreshTick`
  retires; until then it covers the latency between a footer submit
  and the rail re-list.
- **No third chat surface.** The footer reads / writes via the existing
  `assistant.*` RPCs (`append_assistant_message`, `create_assistant_thread`,
  `list_assistant_messages`). `AssistantThreadView` and the new footer
  both render off SQLite; they share data via `list_assistant_messages`
  + `refreshTick`, not a footer-local buffer.
- **`useChatStore` shrunk to UI-presentation slots** (panel-open flag,
  focus signal, composer draft prefill, selection bridge). Its
  `Chat<UIMessage>` / per-token persistence machinery survives only
  because the in-editor `AiMiniWindow` has not been folded into the
  assistant yet — that's a future revision the spec calls out
  ("the in-editor AI chat surface and the assistant become the same
  thing"). The footer no longer touches `getOrCreateChat`,
  `activeSessionId`, or `sendMessage`; a top-of-file comment on
  `chatStore.ts` documents the narrowed scope.
- **Action cards stay on `/assistant`.** The footer renders a compact
  "N pending actions — review and confirm in /assistant" affordance
  when `list_assistant_messages` returns at least one pending card on
  the bound thread; clicking it focuses the `/assistant` tab via the
  existing `newAssistantTab()` route. Full-width card rendering
  (draft preview, scope diff) only happens on the assistant page —
  the footer is for input + short responses.
- **API-key gate dropped from `togglePanelAndFocus`.** The assistant
  runs server-side via the planner, so a keyless user can still open
  the footer. The `<AgentRunBridge>` / `<AiMiniWindow>` gates remain
  because those drive the in-editor browser-side SDK transport.

Out of scope for stage 8 (intentional, follow-up):

- Wiring file-attachments, snippet picker, slash commands, voice
  capture, and selection ingestion into the footer. The previous
  `AiInputBar` had all of these glued to the chatStore-backed composer;
  re-plumbing them through `upload_assistant_attachment` + the
  assistant planner is a larger change and was not blocking the F1
  "messages go to one source of truth" goal.
- Folding `AiMiniWindow` into the assistant. The mini-window remains a
  chatStore-driven mini panel; the spec's "two places, same thing"
  end state requires this collapse, but it's a separate refactor
  (the mini-window has its own SDK transport, todo strip,
  approval bridge — none of which is in the assistant runner yet).

## Stage 9 — tighten

Stage 9 ran the full verification sweep and updated
`DOCS/ASSISTANT-SCOPE.md` to mark F2, F3, F1 completed. Findings:

- `cargo fmt --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace --no-fail-fast`: 3 pre-existing failures,
  none introduced by F2/F3/F1. Verified by re-running the affected
  tests after checking each target file back to `183635a^` (F2
  baseline); failures reproduce identically there.
  1. `codeless-runtime::rpc_in_process::job_filtered_subscription_drops_unrelated_events`
  2. `codeless-runtime::since_replay::since_zero_replays_everything_then_attaches_live_tail`
     — both panic with `Conflict("repo … is already in use by job …
     in in_repo mode; stop it or submit as worktree")`, caused by
     the R0 `workspace_mode in_repo | worktree` validation
     (`8d6d733`) which the two tests never adopted. Fix is a test
     update (set `workspace_mode = Some(WorkspaceMode::Worktree)`
     on the second `submit_job`); out of scope for this job.
  3. `codeless-types::specta_snapshot::wire_types_match_snapshot`
     — ordering drift + the `workspace-unhealthy` /
     `workspace-recovered` event arms added by the
     `7cb9508` master merge, never snapshot-refreshed. Fix is one
     `SPECTA_UPDATE=1` rerun; out of scope here.
- R1: `git diff 183635a^..50cdc4a` shows no `process::Command` or
  `tokio::process` introduced in this job's commits anywhere.
  Pre-existing uses outside `codeless-adapters-host` are tests, the
  CLI, the server bootstrap, and `codeless-tools/src/browser`
  (host-only sidecar) — all host-only crates by design; the rule
  forbids reaching this code from a mobile-safe crate, not the
  existence of the import.
- R2: no `@tauri-apps` imports added outside `src/shells/desktop/`
  in this job's commits. Pre-existing matches outside
  `src/shells/desktop/` are: the two `RpcClient` plumbing files
  (`lib/rpc/tauri-ipc-client.ts`, `lib/rpc/client.ts`), three
  shell-capability adapters under `lib/shell/` (`autostart.ts`,
  `external-opener.ts`, `kv-store.ts`), the settings-window entry
  (`settings/main.tsx`), and two terminal modules
  (`modules/terminal/lib/pty-bridge.ts`,
  `modules/terminal/lib/useTerminalSession.ts`). The latter two
  are an existing R2 follow-up tracked in
  `DOCS/UI-PORT-AUDIT.md` (PTY RPC not landed); none are this
  job's concern.
- R3: zero `*.web.tsx` / `*.mobile.tsx` / `*.ios.tsx` /
  `*.android.tsx` (or `.ts` variants) under `ui/codeless-ui/src/`.

The /loop is complete; nothing left to schedule.
