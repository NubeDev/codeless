import { useMemo } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

// Modal shown on Approve-after-Edit (decision OQ#3). Renders a
// line-by-line diff between the original proposal text and the
// edited buffer so the operator sees the delta before the approval
// commit lands. Confirm calls `approve_scope_patch` with the edited
// rendered buffer; Cancel just closes.

interface Props {
  original: string;
  edited: string;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ApproveDiffDialog({
  original,
  edited,
  onCancel,
  onConfirm,
}: Props) {
  const hunks = useMemo(() => buildLineDiff(original, edited), [original, edited]);

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Approve after edit</DialogTitle>
          <DialogDescription>
            Confirm the delta between the original proposal and your edited
            buffer. The approval commit will use the edited text.
          </DialogDescription>
        </DialogHeader>
        <ScrollArea className="max-h-96 rounded border border-border/60 bg-background">
          <pre className="font-mono text-xs leading-relaxed">
            {hunks.map((h, i) => (
              <div
                key={i}
                className={cn("px-3 py-[1px]", toneFor(h.kind))}
              >
                <span className="select-none pr-2">{prefixFor(h.kind)}</span>
                {h.line === "" ? " " : h.line}
              </div>
            ))}
          </pre>
        </ScrollArea>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            cancel
          </Button>
          <Button onClick={onConfirm}>approve</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ------------------------------------------------------------------ diff

type HunkKind = "context" | "added" | "removed";
interface Hunk {
  kind: HunkKind;
  line: string;
}

// Minimal line-level diff via LCS. Produces a `removed` row for each
// line in `original` that isn't in the LCS, an `added` row for each
// line in `edited` that isn't in the LCS, and `context` rows for
// lines that match. The patch buffer is small (one proposal block),
// so the O(n*m) DP is fine.
function buildLineDiff(original: string, edited: string): Hunk[] {
  const a = original.split("\n");
  const b = edited.split("\n");
  const n = a.length;
  const m = b.length;
  // dp[i][j] = LCS length of a[i..] and b[j..]
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array<number>(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out: Hunk[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "context", line: a[i] });
      i += 1;
      j += 1;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ kind: "removed", line: a[i] });
      i += 1;
    } else {
      out.push({ kind: "added", line: b[j] });
      j += 1;
    }
  }
  while (i < n) {
    out.push({ kind: "removed", line: a[i] });
    i += 1;
  }
  while (j < m) {
    out.push({ kind: "added", line: b[j] });
    j += 1;
  }
  return out;
}

function prefixFor(kind: HunkKind): string {
  switch (kind) {
    case "added":
      return "+";
    case "removed":
      return "-";
    case "context":
      return " ";
  }
}

function toneFor(kind: HunkKind): string {
  switch (kind) {
    case "added":
      return "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
    case "removed":
      return "bg-destructive/10 text-destructive";
    case "context":
      return "text-muted-foreground";
  }
}
