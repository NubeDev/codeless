import { Streamdown } from "streamdown";

import { cn } from "@/lib/utils";

// Shared chat-bubble renderer used by both the assistant transcript
// and (incrementally) the job chat. The two surfaces already share
// the visual grammar — right-aligned primary-tinted bubble for the
// user, left-aligned muted bubble for the model — but until now each
// surface re-implemented the styling and only `JobChat` ran model
// prose through Streamdown. Keeping the renderer in one file means a
// markdown-quirk fix lands in both surfaces at once and the assistant
// stays at parity with the job chat as the renderer evolves.
//
// `role: "assistant"` renders through Streamdown so headings, code
// fences and lists in planner output look like markdown. `role: "user"`
// stays as plain `whitespace-pre-wrap` text: the user's own input
// should never reinterpret syntax it didn't ask to be parsed.
export type BubbleRole = "user" | "assistant";

export type MarkdownBubbleProps = {
  role: BubbleRole;
  content: string;
  // Optional streaming marker. When true, the bubble shows a faint
  // pulse on the trailing edge so a long planner turn doesn't look
  // frozen between ai-token deltas.
  streaming?: boolean;
};

export function MarkdownBubble({
  role,
  content,
  streaming,
}: MarkdownBubbleProps) {
  const isUser = role === "user";
  return (
    <div
      className={cn(
        "flex w-full",
        isUser ? "justify-end" : "justify-start",
      )}
    >
      <div
        className={cn(
          "max-w-[85%] rounded-md px-3 py-2 text-sm",
          isUser
            ? "whitespace-pre-wrap bg-primary text-primary-foreground"
            : "bg-muted text-foreground",
          streaming && "ring-1 ring-primary/30",
        )}
      >
        {isUser ? content : <Streamdown>{content}</Streamdown>}
        {streaming && !isUser && (
          <span className="ml-0.5 inline-block h-3 w-1 animate-pulse bg-current align-middle" />
        )}
      </div>
    </div>
  );
}
