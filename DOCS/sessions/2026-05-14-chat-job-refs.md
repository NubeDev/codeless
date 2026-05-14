# Scope — chat-job-refs: attach other jobs as context

File: DOCS/sessions/2026-05-14-chat-job-refs.md
Started: 2026-05-14
Repo:   codeless
Branch: feat/chat-job-refs

## Goal

Add "attach other jobs as context" to the per-job chat composer so the
agent can see another job's spec files + recent history when answering.
End-to-end path: composer UI → RPC arg → preamble fold in the runtime.

The active-job preamble fold already exists; this job generalises it to
N referenced jobs, gated by per-ref toggles, and surfaces the picker in
the composer.

## Where the surfaces live — read these first

| Surface | File |
|---|---|
| Wire types (`AgentChatArgs`, `ChatContext`, `ChatAttachmentRef`, `UserPromptSnippet`) | [`crates/codeless-rpc/src/methods.rs`](../../crates/codeless-rpc/src/methods.rs) |
| Preamble builder (`build_chat_prompt`, `load_chat_job_spec`) | [`crates/codeless-runtime/src/rpc/chat.rs`](../../crates/codeless-runtime/src/rpc/chat.rs) |
| History source (`job_report` RPC — `turns[]` with prompts / tool calls / costs) | same file, `job_report` impl |
| Spec source (`list_job_files`, `read_job_file`) | same file |
| Composer + attach button | [`ui/codeless-ui/src/modules/jobs/RunPane.tsx`](../../ui/codeless-ui/src/modules/jobs/RunPane.tsx) |

## Stages

Format: `[ ] N. [S|M|L] title` — complexity tag mandatory.

- [x] 1. [S] Wire: extend `ChatContext` with `job_refs: Vec<JobContextRef>` where
       `JobContextRef { job_id: JobId, include_spec: bool, include_history: bool, history_turn_limit: Option<u32> }`.
       Regenerate `wire.ts` via `cargo run -p codeless-rpc --example wire_ts`.
- [x] 2. [M] Server: in `chat.rs`, after the active job's spec fold, iterate
       `args.context.job_refs`. For each:
       (a) load the referenced job + repo, reject with `InvalidArgument` if
           the repo differs from the active job's;
       (b) read its spec files via the same code path as `load_chat_job_spec`;
       (c) if `include_history`, call the existing `job_report` helper and
           render the last N turns (id, user prompt, tool-call summary,
           assistant reply);
       (d) append under a single "Referenced jobs" preamble section.
       Cap each section by `MAX_CHAT_SPEC_BYTES`; the new history fold gets
       its own byte cap (independent of the per-file cap already in
       `truncate_for_chat`).
- [x] 3. [S] Tests: unit tests for the new fold (empty refs / spec-only /
       history-only / spec+history / over-budget truncation /
       cross-repo rejection).
- [x] 4. [M] UI: add an "attach job" button next to "attach" in `RunPane.tsx`.
       Opens a picker listing jobs from `list_jobs` (filtered to the active
       job's repo). Selecting one appends a chip showing job name + id with
       two toggles (`spec`, `history`). Chips persist for the conversation,
       not just the next turn — mirror the existing attachments behaviour.
       Thread `job_refs` into the `rpc.call("agent_chat", ...)` payload.
- [ ] 5. [S] UI: mock-client parity — `MockRpcClient` accepts `job_refs` and
       round-trips it in the recorded request shape.

## Constraints

- `ChatContext.job_refs` is additive; existing turns continue to work with
  `job_refs: []`. No migration needed.
- Per-ref byte budgets are non-negotiable — folding two referenced jobs'
  SCOPE + WORKFLOW + last 5 turns is easily 20–30 KB. Spec fold reuses the
  existing per-file cap; history fold needs its own.
- R1 holds: no `std::process` / `tokio::process` reach added to mobile-safe
  crates. The wire-type change in `codeless-rpc` stays pure types.
- R2 holds: the UI imports `RpcClient` only — no direct fetch to the
  server, no Tauri imports.

## Out of scope

- **Cross-repo job refs.** Restrict refs to jobs in the same repo as the
  active one; reject otherwise with `InvalidArgument`. Cross-repo lands
  in a later phase once repo-trust scoping is settled.
- **Live event-tail streaming** of the referenced job. History is a
  snapshot taken at turn-send time; the referenced job's tail does not
  follow the active chat.
- **Editing the referenced job** from the chat. Read-only.
- New picker UX beyond a flat list. Search / grouping is a follow-up if
  the list gets long.

## Acceptance

- `cargo test --workspace` green.
- `cargo clippy --workspace --all-targets -- -D warnings` green.
- `cargo fmt --check` green.
- `pnpm -C ui/codeless-ui tsc` green; `wire.ts` regenerated and committed
  alongside the Rust change.
- Manual: open job A's chat, attach job B (spec + history), ask
  "what did job B do in its last stage" — the assistant cites stage names
  and turn outcomes from B without the user pasting them in.

## Risks / things to watch

- Budget interaction between active-job fold and ref folds. The active
  job's spec already consumes most of `MAX_CHAT_SPEC_BYTES`; the ref
  section needs its own budget rather than sharing, or a long active-job
  spec silently starves refs.
- `job_report` may be expensive for long-running jobs. Cap the turn
  walk at `history_turn_limit` (default ~5) before rendering, not after,
  so the cost is bounded.
- Picker filtering must use the same repo identity the server uses for
  the `InvalidArgument` check, or users will see refs in the picker that
  the server then rejects.
