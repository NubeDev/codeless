import { useEffect, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { RpcError, useRpc, type Job } from "@/lib/rpc";

interface Props {
  job: Job;
}

type State =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "missing"; tried: string[] }
  | { kind: "ready"; path: string; content: string }
  | { kind: "error"; message: string };

// Best-effort preview of a job's `handover.md` (the JOB-MODEL.md
// session contract — see DOCS/JOB-MODEL.md). The runtime does not
// yet write these automatically for every job, so the panel must
// degrade cleanly to "not found" rather than treating absence as
// an error. Two locations are probed in priority order:
//   1. <worktree>/runs/<job_id>/handover.md  — the canonical layout
//   2. <worktree>/handover.md                — ad-hoc demos at the root
// `fs_read_file` returns `not_found` if the path is missing; any
// other error (binary content, transport, permissions) surfaces
// verbatim so the operator can tell what went wrong.
export function HandoverPanel({ job }: Props) {
  const rpc = useRpc();
  const [state, setState] = useState<State>({ kind: "idle" });

  useEffect(() => {
    if (!job.worktree_path) {
      setState({ kind: "missing", tried: [] });
      return;
    }
    let cancelled = false;
    setState({ kind: "loading" });
    const candidates = [
      `${job.worktree_path}/runs/${job.id}/handover.md`,
      `${job.worktree_path}/handover.md`,
    ];
    void (async () => {
      let lastError: string | null = null;
      for (const path of candidates) {
        try {
          const result = await rpc.call("fs_read_file", {
            path,
            byte_limit: null,
          });
          if (cancelled) return;
          if (result.kind === "text") {
            setState({ kind: "ready", path, content: result.content });
            return;
          }
          if (result.kind === "binary") {
            lastError = "handover.md is binary, not utf-8 text";
            continue;
          }
          if (result.kind === "toolarge") {
            lastError = `handover.md exceeds the ${result.limit}-byte read limit`;
            continue;
          }
        } catch (e) {
          if (cancelled) return;
          if (e instanceof RpcError && e.kind === "not_found") continue;
          lastError = e instanceof Error ? e.message : String(e);
        }
      }
      if (cancelled) return;
      if (lastError) {
        setState({ kind: "error", message: lastError });
      } else {
        setState({ kind: "missing", tried: candidates });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [job.id, job.worktree_path, rpc]);

  return (
    <ScrollArea className="flex-1">
      <div className="p-4">
        {state.kind === "idle" || state.kind === "loading" ? (
          <div className="text-muted-foreground text-sm">loading…</div>
        ) : state.kind === "missing" ? (
          <MissingNotice tried={state.tried} hasWorktree={!!job.worktree_path} />
        ) : state.kind === "error" ? (
          <div className="text-destructive text-sm">{state.message}</div>
        ) : (
          <Handover content={state.content} path={state.path} />
        )}
      </div>
    </ScrollArea>
  );
}

function Handover({ content, path }: { content: string; path: string }) {
  return (
    <div className="space-y-2">
      <div className="text-muted-foreground font-mono text-[10px]" title={path}>
        {path}
      </div>
      <pre className="bg-muted/30 border-border/40 whitespace-pre-wrap rounded border px-3 py-2 font-mono text-xs leading-snug">
        {content}
      </pre>
    </div>
  );
}

function MissingNotice({
  tried,
  hasWorktree,
}: {
  tried: string[];
  hasWorktree: boolean;
}) {
  return (
    <div className="text-muted-foreground space-y-2 text-sm">
      <p>No handover yet for this job.</p>
      {!hasWorktree && (
        <p className="text-xs">
          The job has no worktree path on disk — handover files live under
          the worktree, so nothing to preview until the runner provisions one.
        </p>
      )}
      {tried.length > 0 && (
        <div className="text-xs">
          <p>Looked under:</p>
          <ul className="mt-1 list-disc pl-5">
            {tried.map((p) => (
              <li key={p} className="font-mono text-[11px]">
                {p}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
