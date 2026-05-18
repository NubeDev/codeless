## Done

- Stage W1d shipped: commit `660126b` on `feat/assistant-parity` adds `ui/codeless-ui/src/modules/chat/CommonChat.parity.test.tsx`, which mounts both `JobChat` and `AssistantThreadView` against `MockRpcClient` subclasses serving the same prose rows (CHAT.md heading format vs `list_assistant_messages` shape) and asserts each `<ul>`'s `<li>` children have identical `outerHTML`.
- Production parity fix to make the assertion hold: `ChatMessageList` now falls through to the default `<ChatBubble>` when `renderMessage` returns `null`/`undefined` (instead of wrapping `null` in an empty `<li>`); `AssistantThreadView.renderMessage` early-returns `null` for plain user/assistant prose so the wrapper opts out of its custom dispatch for rows the default can render byte-for-byte. Removed the now-unused `MarkdownBubble` import.
- Verified: `pnpm vitest run` → 14 files / 64 tests pass; `pnpm typecheck` clean; `pnpm lint` is a no-op stub; R2 grep (`@tauri-apps`) and R3 grep (`.{web,desktop,mobile}.tsx`) both empty across `modules/{chat,assistant,jobs}`.
- Pushed to `origin/feat/assistant-parity`.

## Next

- Stage 8 is the REVIEW gate before W3: "shared renderer and composer must be stable before policy cards land". The reviewer should spot-check the parity test fixture (`CommonChat.parity.test.tsx`), the `ChatMessageList` fall-through semantics, and the `AssistantThreadView.renderMessage` early-return — those three together lock the §W1 promise.
- After the gate passes, stage 9 starts block 3: `W3a extract POLICY_PRESETS to ui/codeless-ui/src/lib/policy/presets.ts and Rust mirror`.

## What you need to know

- The pre-existing modification to `crates/codeless-runtime/src/diff_verify.rs` (added Rust/JS expression-prefix and PascalCase-method-call rejections in `looks_path_like`, plus six new tests) is unrelated to W1d. It was already in the worktree at the start of this stage. I stashed it for the commit and popped it back; it remains uncommitted in the worktree. Whoever picks up the next stage should decide whether it belongs to a different job or needs its own commit before continuing.
- Parity is enforced at the `<li>` level, not the `<ul>` level: `AssistantThreadView` passes a custom `className` to `ChatMessageList`'s scroll container, so the `<ul>` attributes deliberately differ. The test compares `ul > li` children so a future reshuffle of the container styling doesn't false-fail the parity contract.
- `ChatMessageList`'s `renderMessage` API now formally accepts a `null`/`undefined` return as "use the default". The doc comment on the prop was updated; any new caller can opt out per-row by returning `null`. Returning a React element whose component renders `null` (the old assistant pattern) still produces an empty `<li>` — that's by design, since `ChatMessageList` cannot see through the element to know the inner component will bail.
- `MockRpcClient.subscribe` only filters by `job_id`. The parity test's two render trees share a process but each gets its own client instance, so subscriptions don't cross-pollinate.

## Open questions

- The MarkdownBubble component is no longer imported by any production file (only its own `MarkdownBubble.tsx` and its `index.ts` re-export). It's effectively dead code. Removing it is a drive-by refactor (R4), so I left it; W3 or a follow-up cleanup tick can prune it if no policy-card surface ends up needing it.
