import { Streamdown } from "streamdown";

import { cn } from "@/lib/utils";

import { PulseDot } from "./PulseDot";
import type { ChatMessage } from "./feed";

// Rich chat bubble used by every chat surface that renders a
// persisted `ChatMessage` (role + text + ts). Renders the assistant's
// prose through Streamdown so markdown (code fences, headings, lists)
// looks like markdown — `MarkdownBubble` in this module is the
// minimal sibling used by callers that don't carry a per-message
// timestamp yet. The two converge in W1b when the renderer mounts
// every history row through this component.
export function ChatBubble({
  message,
  streaming,
}: {
  message: ChatMessage;
  streaming?: boolean;
}) {
  const isUser = message.role === "user";
  return (
    <li
      className={cn(
        "rounded-md border px-2.5 py-2",
        isUser
          ? "border-zinc-500/30 bg-zinc-500/5"
          : "border-blue-500/30 bg-blue-500/5",
      )}
    >
      <div className="text-muted-foreground mb-1 flex items-center justify-between gap-1.5 text-[9px] uppercase tracking-wide">
        <span className={isUser ? "" : "text-blue-700 dark:text-blue-300"}>
          {isUser ? "you" : "assistant"}
        </span>
        {streaming && <PulseDot color="bg-blue-500" />}
        {!streaming && message.ts && (
          <span className="font-mono normal-case tracking-normal">
            {shortTime(message.ts)}
          </span>
        )}
      </div>
      <div className="prose prose-sm dark:prose-invert max-w-none text-[12px] break-words [&_pre]:my-1.5 [&_pre]:bg-background/60 [&_pre]:p-2 [&_pre]:text-[11px] [&_pre]:whitespace-pre-wrap [&_pre]:break-words [&_pre]:overflow-x-auto [&_code]:bg-background/60 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[11px] [&_code]:break-all [&_h1]:text-sm [&_h2]:text-sm [&_h3]:text-[13px] [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_p]:my-1 [&_ul]:my-1 [&_ol]:my-1 [&_li]:my-0">
        <Streamdown>{message.text}</Streamdown>
      </div>
    </li>
  );
}

// ISO `YYYY-MM-DDTHH:MM:SSZ` rendered as the same string minus the
// `T` / `Z` separators. Kept local to the bubble because nothing else
// formats a `ChatMessage.ts` directly — the live-feed rows use a
// wall-clock helper instead.
function shortTime(iso: string): string {
  return iso.replace("T", " ").replace("Z", "");
}
