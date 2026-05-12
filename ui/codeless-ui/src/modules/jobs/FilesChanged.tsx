import { useEffect, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { useRpc, type JobDiffResult, type JobId } from "@/lib/rpc";

// Renders the diff of a job's branch against its repo's default
// branch. Fetched on-demand from the `job_diff` RPC — diffs can be
// slow for big jobs, so we do not refetch unless the user re-opens
// the tab. Worktrees may be reaped between job completion and view,
// but the branch survives in the source repo, so this works whether
// or not the worktree directory still exists on disk.
interface FilesChangedProps {
  jobId: JobId;
  /**
   * Open the given path in an editor tab. Receives the worktree-
   * relative path as it comes back from `job_diff`; the host wires
   * the resolution against the worktree root (the dashboard does not
   * know the worktree root from this surface alone).
   */
  onOpenFile?: (relPath: string) => void;
}

export function FilesChanged({ jobId, onOpenFile }: FilesChangedProps) {
  const rpc = useRpc();
  const [state, setState] = useState<
    | { kind: "loading" }
    | { kind: "ready"; diff: JobDiffResult }
    | { kind: "empty"; reason: string }
    | { kind: "error"; message: string }
  >({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    rpc
      .call("job_diff", { job_id: jobId })
      .then((diff) => {
        if (cancelled) return;
        if (diff.files.length === 0) {
          setState({
            kind: "empty",
            reason: `no changes between ${diff.base} and ${diff.head}`,
          });
        } else {
          setState({ kind: "ready", diff });
        }
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        // The most common error here is "head ref missing" — the job
        // never provisioned a worktree (mock runner, or pre-provision
        // crash). Surface it as an empty state rather than an angry
        // error because it's expected, not exceptional.
        const message = e instanceof Error ? e.message : String(e);
        if (/head ref|base ref/i.test(message)) {
          setState({
            kind: "empty",
            reason: "no branch to diff (job did not provision a worktree)",
          });
        } else {
          setState({ kind: "error", message });
        }
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId]);

  if (state.kind === "loading") {
    return (
      <div className="text-muted-foreground p-4 text-sm">computing diff…</div>
    );
  }
  if (state.kind === "empty") {
    return (
      <div className="text-muted-foreground p-4 text-sm italic">
        {state.reason}
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <div className="text-destructive p-4 text-sm">
        {state.message}
      </div>
    );
  }

  const { diff } = state;
  return (
    <ScrollArea className="h-full">
      <div className="p-3">
        <div className="text-muted-foreground mb-2 font-mono text-[11px]">
          {diff.files.length} file{diff.files.length === 1 ? "" : "s"} changed —{" "}
          <span className="text-foreground">{diff.head}</span> vs{" "}
          <span className="text-foreground">{diff.base}</span>
        </div>
        <ul className="space-y-2">
          {diff.files.map((f) => (
            <FileBlock key={f.path} file={f} onOpen={onOpenFile} />
          ))}
        </ul>
      </div>
    </ScrollArea>
  );
}

function FileBlock({
  file,
  onOpen,
}: {
  file: JobDiffResult["files"][number];
  onOpen?: (relPath: string) => void;
}) {
  const [open, setOpen] = useState(file.additions + file.deletions <= 40);
  return (
    <li className="border-border/50 rounded border bg-card/30">
      <div className="hover:bg-accent/20 flex w-full items-center gap-2 px-2 py-1.5 text-xs">
        <StatusPill status={file.status} />
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="min-w-0 flex-1 truncate text-left font-mono"
        >
          {file.path}
        </button>
        {!file.is_binary && (
          <span className="text-muted-foreground font-mono text-[11px]">
            <span className="text-emerald-500">+{file.additions}</span>{" "}
            <span className="text-destructive">−{file.deletions}</span>
          </span>
        )}
        {file.is_binary && (
          <span className="text-muted-foreground text-[10.5px] italic">
            binary
          </span>
        )}
        {onOpen && file.status !== "D" && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onOpen(file.path);
            }}
            className="text-muted-foreground hover:text-foreground rounded px-1.5 py-0.5 text-[10px]"
            title="Open in editor"
          >
            open
          </button>
        )}
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="text-muted-foreground text-[10px]"
          aria-label={open ? "collapse" : "expand"}
        >
          {open ? "−" : "+"}
        </button>
      </div>
      {open && !file.is_binary && file.patch && (
        <DiffBody patch={file.patch} />
      )}
    </li>
  );
}

// Status pill for the file row. Maps `git diff --name-status` letters
// to a short, colour-coded label. Unknown letters fall through with
// the raw letter so we never silently mislabel a wire value we don't
// yet handle (e.g. `T` for type-change, `C` for copy).
function StatusPill({ status }: { status: string }) {
  const meta: Record<string, { label: string; tone: string }> = {
    A: { label: "added", tone: "bg-emerald-500/15 text-emerald-500" },
    M: { label: "modified", tone: "bg-amber-500/15 text-amber-500" },
    D: { label: "deleted", tone: "bg-destructive/15 text-destructive" },
    R: { label: "renamed", tone: "bg-blue-500/15 text-blue-500" },
    C: { label: "copied", tone: "bg-blue-500/15 text-blue-500" },
    T: { label: "type", tone: "bg-muted text-muted-foreground" },
  };
  const m = meta[status] ?? { label: status, tone: "bg-muted text-muted-foreground" };
  return (
    <span
      className={cn(
        "shrink-0 rounded px-1.5 py-0.5 text-[9.5px] font-medium uppercase tracking-wide",
        m.tone,
      )}
    >
      {m.label}
    </span>
  );
}

// Render a unified-diff body line-by-line. Hunk headers, additions,
// deletions, and context lines all get distinct colouring; the raw
// `git` headers (`diff --git`, `index`, `+++`, `---`) are dropped
// because the surrounding row already shows the path and the diff
// metadata adds noise without information for a single-file view.
function DiffBody({ patch }: { patch: string }) {
  const lines = patch.split("\n").filter((line) => {
    if (line.startsWith("diff --git")) return false;
    if (line.startsWith("index ")) return false;
    if (line.startsWith("+++ ") || line.startsWith("--- ")) return false;
    if (line.startsWith("new file mode") || line.startsWith("deleted file mode"))
      return false;
    if (line.startsWith("similarity index") || line.startsWith("rename ")) return false;
    return true;
  });
  return (
    <pre className="border-border/40 border-t bg-background/60 overflow-x-auto px-2 py-1.5 text-[11px] leading-tight">
      {lines.map((line, i) => (
        <DiffLine key={i} line={line} />
      ))}
    </pre>
  );
}

function DiffLine({ line }: { line: string }) {
  let className = "block whitespace-pre font-mono";
  if (line.startsWith("@@")) {
    className = `${className} text-muted-foreground bg-muted/30`;
  } else if (line.startsWith("+")) {
    className = `${className} text-emerald-500`;
  } else if (line.startsWith("-")) {
    className = `${className} text-destructive`;
  } else {
    className = `${className} text-muted-foreground`;
  }
  return <span className={className}>{line || " "}</span>;
}
