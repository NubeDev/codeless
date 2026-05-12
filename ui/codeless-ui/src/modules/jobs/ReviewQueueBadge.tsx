import { Button } from "@/components/ui/button";
import { useReviews } from "@/lib/rpc";

interface Props {
  onOpen: () => void;
}

// Count of REVIEW gates awaiting a human in the top bar. Subscribes
// to `list_reviews({ status: "pending" })` via `useReviews`; absent
// or zero-result data renders nothing so the chrome stays clean when
// there's nothing waiting. Click invokes the caller-supplied `onOpen`
// handler that navigates into the jobs view — concrete per-status
// filtering of the jobs list is a future polish; the badge's job is
// to surface the queue.
export function ReviewQueueBadge({ onOpen }: Props) {
  const { data } = useReviews({
    job_id: null,
    stage_id: null,
    status: "pending",
  });
  const count = data?.length ?? 0;
  if (count === 0) return null;
  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={onOpen}
      className="h-7 shrink-0 gap-1.5 rounded-md px-2 text-[11px] text-muted-foreground hover:text-foreground"
      title={`${count} pending review${count === 1 ? "" : "s"} awaiting approval`}
    >
      <span className="bg-amber-500/15 text-amber-500 rounded px-1.5 py-0.5 font-mono">
        {count}
      </span>
      <span>review{count === 1 ? "" : "s"}</span>
    </Button>
  );
}
