import { JobChat, type JobChatProps } from "../jobs/RunPane";
import { AiChatView, type AiChatViewProps } from "../ai/components/AiChat";

// The three chat surfaces — JobChat (in `RunPane`), AiChat (in the
// editor AI panel) and the upcoming `/assistant` thread view — share
// rendering chrome (conversation scroll, message bubbles, tool-call
// cards, attachments) but diverge in their state model: JobChat reads
// `CHAT.md` and subscribes to the job's event stream; AiChat reads
// from `chatStore.ts` and the `ai` SDK's `Chat<UIMessage>`. Until the
// AiChat surface is migrated off zustand (SCOPE.md decision 2), the
// two surfaces cannot share a single internal implementation without
// a behaviour-changing rewrite — which this stage explicitly forbids.
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
  | ({ kind: "ai" } & AiChatViewProps);

export function CommonChat(props: CommonChatProps) {
  if (props.kind === "job") {
    const { kind: _k, ...rest } = props;
    return <JobChat {...rest} />;
  }
  const { kind: _k, ...rest } = props;
  return <AiChatView {...rest} />;
}
