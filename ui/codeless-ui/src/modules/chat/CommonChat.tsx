import { JobChat, type JobChatProps } from "../jobs/RunPane";
import { AiChatView, type AiChatViewProps } from "../ai/components/AiChat";
import {
  AssistantThreadView,
  type AssistantThreadViewProps,
} from "../assistant/AssistantThreadView";

// The three chat surfaces — JobChat (in `RunPane`), AiChat (in the
// editor AI panel) and the `/assistant` thread view — share rendering
// chrome (conversation scroll, message bubbles, tool-call cards,
// attachments) but diverge in their state model: JobChat reads
// `CHAT.md` and subscribes to the job's event stream; AiChat reads
// from `chatStore.ts` and the `ai` SDK's `Chat<UIMessage>`; the
// assistant view reads `assistant_messages` rows via the `assistant.*`
// RPCs. Until the three surfaces converge on a single message model,
// they cannot share one internal implementation without a
// behaviour-changing rewrite — which the per-stage rules forbid.
//
// `CommonChat` is therefore a discriminated-union facade that picks
// the existing implementation by `kind`. Call sites import this
// single component; the props collapse into a stable surface that
// later stages can preserve while swapping the underlying impl.
//
// Capabilities (start/stop/edit-scope/etc.) are **not** derived from
// `kind` — those come from the server-side thread row per SCOPE.md
// constraint. `kind` is UI-only routing.
export type CommonChatProps =
  | ({ kind: "job" } & JobChatProps)
  | ({ kind: "ai" } & AiChatViewProps)
  | ({ kind: "assistant" } & AssistantThreadViewProps);

export function CommonChat(props: CommonChatProps) {
  if (props.kind === "job") {
    const { kind: _k, ...rest } = props;
    return <JobChat {...rest} />;
  }
  if (props.kind === "assistant") {
    const { kind: _k, ...rest } = props;
    return <AssistantThreadView {...rest} />;
  }
  const { kind: _k, ...rest } = props;
  return <AiChatView {...rest} />;
}
