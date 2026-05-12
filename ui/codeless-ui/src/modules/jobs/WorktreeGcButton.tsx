import { useCallback, useState } from "react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { useRpc, type GcWorktreesResult } from "@/lib/rpc";

// Default sweep window: anything older than 7 days. Matches the
// recommendation in the kickoff stage; bigger windows are explicit
// (the user always sees the count + size before confirming, and the
// modal opens in dry-run mode so an accidental click is safe).
const DEFAULT_OLDER_THAN_MS = 7 * 24 * 60 * 60 * 1000;

export function WorktreeGcButton() {
  const rpc = useRpc();
  const [open, setOpen] = useState(false);
  const [preview, setPreview] = useState<GcWorktreesResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [purging, setPurging] = useState(false);

  const loadPreview = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await rpc.call("gc_worktrees", {
        older_than_ms: DEFAULT_OLDER_THAN_MS,
        job_ids: null,
        dry_run: true,
      });
      setPreview(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [rpc]);

  const openModal = useCallback(() => {
    setOpen(true);
    setPreview(null);
    setError(null);
    void loadPreview();
  }, [loadPreview]);

  const confirmPurge = useCallback(async () => {
    setPurging(true);
    setError(null);
    try {
      const result = await rpc.call("gc_worktrees", {
        older_than_ms: DEFAULT_OLDER_THAN_MS,
        job_ids: null,
        dry_run: false,
      });
      setPreview(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPurging(false);
    }
  }, [rpc]);

  const reclaimableCount = preview?.entries.length ?? 0;
  const reclaimableSize = preview?.total_size_bytes ?? 0;
  const removedCount = preview?.removed_count ?? 0;
  const allRemoved = preview !== null && removedCount === reclaimableCount;

  return (
    <>
      <Button
        size="sm"
        variant="outline"
        className="h-7 px-2 text-[11px]"
        onClick={openModal}
        title="Reclaim disk used by old job worktrees (7+ days)"
      >
        GC worktrees
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent className="max-w-lg">
          <AlertDialogHeader>
            <AlertDialogTitle>Garbage-collect worktrees</AlertDialogTitle>
            <AlertDialogDescription>
              Removes worktrees older than 7 days. Each tree's branch
              stays in the source repo — only the working copy is
              reclaimed.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {error && (
            <div className="text-destructive text-xs">{error}</div>
          )}
          {loading ? (
            <div className="text-muted-foreground text-sm">scanning…</div>
          ) : preview === null ? null : reclaimableCount === 0 ? (
            <div className="text-muted-foreground text-sm">
              Nothing to reclaim. No worktrees on disk match the 7-day
              window.
              {preview.root && (
                <span className="text-[11px]"> Root: {preview.root}</span>
              )}
            </div>
          ) : (
            <div className="space-y-2">
              <div className="text-sm">
                <span className="font-semibold">{reclaimableCount}</span>{" "}
                worktree{reclaimableCount === 1 ? "" : "s"} would free{" "}
                <span className="font-semibold">{formatBytes(reclaimableSize)}</span>
                .
              </div>
              <ul className="max-h-40 overflow-y-auto rounded border border-border/50 text-xs">
                {preview.entries.map((e) => (
                  <li
                    key={e.path}
                    className="flex items-center justify-between gap-2 border-b border-border/30 px-2 py-1 last:border-b-0"
                  >
                    <span className="truncate font-mono text-[11px]" title={e.path}>
                      {e.path}
                    </span>
                    <span
                      className={`font-mono text-[11px] ${e.removed ? "text-emerald-500" : e.error ? "text-destructive" : "text-muted-foreground"}`}
                    >
                      {e.removed
                        ? "removed"
                        : e.error
                          ? "failed"
                          : formatBytes(e.size_bytes)}
                    </span>
                  </li>
                ))}
              </ul>
              {allRemoved && removedCount > 0 && (
                <div className="text-emerald-500 text-xs">
                  removed {removedCount} worktree
                  {removedCount === 1 ? "" : "s"}.
                </div>
              )}
            </div>
          )}
          <AlertDialogFooter>
            <AlertDialogCancel>Close</AlertDialogCancel>
            {preview !== null && reclaimableCount > 0 && !allRemoved && (
              <AlertDialogAction
                onClick={(e) => {
                  // Stop the dialog from auto-closing on Action click so
                  // the result list stays visible after the purge.
                  e.preventDefault();
                  void confirmPurge();
                }}
                disabled={purging}
              >
                {purging ? "removing…" : `Remove ${reclaimableCount}`}
              </AlertDialogAction>
            )}
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const kb = n / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  const gb = mb / 1024;
  return `${gb.toFixed(2)} GB`;
}
