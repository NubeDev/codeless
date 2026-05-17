import { JobChat, type JobChatProps } from "../jobs/RunPane";
import { AiChatView, type AiChatViewProps } from "../ai/components/AiChat";
import {
  AssistantThreadView,
  type AssistantThreadViewProps,
} from "../assistant/AssistantThreadView";
import type { AssistantThreadId, JobId } from "@/lib/rpc";

// `CommonChat` is the single entry point every chat surface in the
// UI renders through — the assistant page, the in-job chat, and the
// in-editor AI panel all import this component and bind it to a
// `threadId` that names the server-resident conversation the runtime
// keys all state by. The id slot is the load-bearing addition in
// this stage:
//
//   - PS3 derives the thread's allowed-tools list from that id on
//     the server side, never from a value the client passed at
//     render time. Pinning the id on every call site now means PS3
//     does not have to revisit them.
//   - PS4 replaces the per-kind internals (`CHAT.md` for job,
//     `chatStore` for ai, `assistant_messages` for assistant) with
//     one SQLite-backed reader keyed off the same id, so the prop
//     surface survives the collapse.
//
// `kind` stays for this stage only — it is routing, not capability —
// and PS3 retires it. `assistant` threads are already
// server-resident; `job` threads are server-resident through the
// job row; `ai` threads still live in `chatStore.ts` until PS4
// promotes them, but the id slot is present now so the call sites
// do not move.
//
// Capabilities (start/stop/edit-scope/etc.) are not derived from
// `kind` — those come from the server-side thread row per
// `SCOPE.md`. `kind` is UI-only routing until PS3 deletes it.
export type CommonChatProps =
  | ({ kind: "job"; threadId: JobId } & JobChatProps)
  | ({ kind: "ai"; threadId: string } & AiChatViewProps)
  | ({ kind: "assistant"; threadId: AssistantThreadId } & AssistantThreadViewProps);

export function CommonChat(props: CommonChatProps) {
  if (props.kind === "job") {
    const { kind: _k, threadId: _t, ...rest } = props;
    return <JobChat {...rest} />;
  }
  if (props.kind === "assistant") {
    const { kind: _k, threadId: _t, ...rest } = props;
    return <AssistantThreadView {...rest} />;
  }
  const { kind: _k, threadId: _t, ...rest } = props;
  return <AiChatView {...rest} />;
}
