import { useState } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useRpc, type JobId } from "@/lib/rpc";

interface Props {
  jobId: JobId;
}

// Restart options for a failed verify row. Two modes, no others:
//
//   rerun now         — re-queues the job using the captured session_id so
//                       the next runner invocation passes `--continue`; the
//                       agent picks up where it left off without re-onboarding.
//
//   new session + handover — creates a fresh copy of the job so the next
//                       runner gets a clean context seeded from the stage's
//                       handover.md. Use when the warm session has drifted.
//
// Both modes go through existing job-control RPCs; neither is a stage-
// granularity operation on the current API, which is why both touch the
// whole job rather than just the failing stage.
export function RestartMenu({ jobId }: Props) {
  const rpc = useRpc();
  const [busy, setBusy] = useState<"rerun" | "new-session" | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const rerunNow = async () => {
    setBusy("rerun");
    setErr(null);
    try {
      await rpc.call("resume_job", { job_id: jobId });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const newSession = async () => {
    setBusy("new-session");
    setErr(null);
    try {
      await rpc.call("rerun_job", { source_job_id: jobId });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="inline-flex flex-col items-end gap-1">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            className="border-border/60 bg-background hover:bg-accent inline-flex items-center gap-1 rounded border px-2 py-0.5 text-[11px] disabled:opacity-50"
            disabled={busy !== null}
            aria-label="Restart options"
          >
            {busy !== null ? "…" : "restart"}{" "}
            <span className="text-[9px]">&#9660;</span>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="text-xs">
          <DropdownMenuItem
            className="text-xs"
            onSelect={() => void rerunNow()}
            disabled={busy !== null}
          >
            rerun now
          </DropdownMenuItem>
          <DropdownMenuItem
            className="text-xs"
            onSelect={() => void newSession()}
            disabled={busy !== null}
          >
            new session + handover
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {err !== null && (
        <span className="text-destructive text-[10px]">{err}</span>
      )}
    </div>
  );
}
