import { useState } from "react";

import { useRpc, useReviews, type JobId, type Review } from "@/lib/rpc";

import { Button } from "@/components/ui/button";

// Pending-review surface for one job. Lists every review attached to
// any of the job's stages and exposes approve / stop / comment. The
// stage-gated lifecycle in `codeless-runtime` parks at AwaitingReview
// until one of approve / stop arrives; the UI's job here is to make
// that gate visible and actionable.
export function ReviewPanel({ jobId }: { jobId: JobId }) {
  const { data: reviews, loading, error } = useReviews({
    job_id: jobId,
    stage_id: null,
    pending_only: false,
  });

  if (loading) {
    return <div className="text-muted-foreground p-3 text-xs">loading reviews…</div>;
  }
  if (error) {
    return <div className="text-destructive p-3 text-xs">{error.message}</div>;
  }
  if (!reviews || reviews.length === 0) return null;

  const pending = reviews.filter((r) => r.status === "pending");
  const resolved = reviews.filter((r) => r.status !== "pending");

  return (
    <div className="border-border/50 border-b">
      {pending.length > 0 && (
        <div className="p-3">
          <div className="text-muted-foreground mb-2 text-[11px] uppercase tracking-wide">
            Awaiting review ({pending.length})
          </div>
          <div className="flex flex-col gap-2">
            {pending.map((r) => (
              <ReviewRow key={r.id} review={r} />
            ))}
          </div>
        </div>
      )}
      {resolved.length > 0 && (
        <div className="p-3 pt-0">
          <div className="text-muted-foreground mb-1 text-[11px] uppercase tracking-wide">
            Resolved ({resolved.length})
          </div>
          <div className="flex flex-col gap-1">
            {resolved.map((r) => (
              <div
                key={r.id}
                className="text-muted-foreground flex items-center gap-2 text-xs"
              >
                <span className="font-mono">{r.status}</span>
                <span className="font-mono text-[10px] opacity-70">{r.id}</span>
                {r.comment && <span className="truncate">— {r.comment}</span>}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function ReviewRow({ review }: { review: Review }) {
  const rpc = useRpc();
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState<"approve" | "stop" | "comment" | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const run = async (
    action: "approve" | "stop" | "comment",
    fn: () => Promise<unknown>,
  ) => {
    setBusy(action);
    setErr(null);
    try {
      await fn();
      if (action === "comment") setComment("");
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="bg-muted/40 rounded p-2">
      <div className="text-muted-foreground flex items-center gap-2 font-mono text-[11px]">
        <span>stage {review.stage_id.slice(0, 10)}…</span>
        <span className="opacity-70">·</span>
        <span>requested {timeAgo(review.requested_at)}</span>
      </div>
      <div className="mt-2 flex gap-2">
        <input
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          placeholder="optional comment"
          className="border-border/50 placeholder:text-muted-foreground flex-1 rounded border bg-transparent px-2 py-1 text-xs"
        />
        <Button
          size="sm"
          variant="ghost"
          disabled={busy !== null || comment.trim().length === 0}
          onClick={() =>
            void run("comment", () =>
              rpc.call("comment_review", {
                review_id: review.id,
                comment: comment.trim(),
              }),
            )
          }
        >
          Comment
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy !== null}
          onClick={() =>
            void run("stop", () =>
              rpc.call("stop_review", { review_id: review.id }),
            )
          }
        >
          Stop
        </Button>
        <Button
          size="sm"
          disabled={busy !== null}
          onClick={() =>
            void run("approve", () =>
              rpc.call("approve_review", { review_id: review.id }),
            )
          }
        >
          Approve
        </Button>
      </div>
      {err && <div className="text-destructive mt-1 text-[11px]">{err}</div>}
    </div>
  );
}

function timeAgo(ms: number): string {
  const delta = Math.max(0, Date.now() - ms);
  if (delta < 1000) return "just now";
  if (delta < 60_000) return `${Math.floor(delta / 1000)}s ago`;
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  return `${Math.floor(delta / 3_600_000)}h ago`;
}
