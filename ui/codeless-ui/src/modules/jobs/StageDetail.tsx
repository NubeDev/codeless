import { useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  useRpc,
  type JobId,
  type StageRollup,
} from "@/lib/rpc";

interface Props {
  jobId: JobId;
  stageId: string;
  onBack: () => void;
}

// Right-pane detail for one selected stage. Today renders the rollup
// (status, duration, cost, task count) and placeholders for the
// pending wishlist items: claude session id, per-stage commits, tool
// ribbon, final assistant message. Each placeholder is an honest
// "coming next session" with the source the data will come from, so
// future agents picking up this work know where to look.
//
// The rollup itself is fetched fresh on mount + on stageId change;
// this is a focused query, not the all-stages list, but the existing
// list_stages RPC is the cheapest source — we filter client-side
// rather than adding a new endpoint until per-stage detail proves
// noisy enough to warrant it.
export function StageDetail({ jobId, stageId, onBack }: Props) {
  const rpc = useRpc();
  const [rollup, setRollup] = useState<StageRollup | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setRollup(null);
    setError(null);
    rpc
      .call("list_stages", { job_id: jobId })
      .then((res) => {
        if (cancelled) return;
        const found = res.stages.find((s) => s.stage.id === stageId) ?? null;
        setRollup(found);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId, stageId]);

  return (
    <ScrollArea className="h-full">
      <div className="space-y-4 p-4">
        <div className="flex items-center justify-between gap-3">
          <Button
            variant="ghost"
            size="sm"
            onClick={onBack}
            className="h-7 px-2 text-xs"
          >
            ← all stages
          </Button>
          {rollup && <StatusPill status={rollup.stage.status} />}
        </div>

        <div className="space-y-1">
          <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
            Stage {(rollup?.stage.ordinal ?? 0) + 1}
          </div>
          <h2 className="text-base font-medium">
            {rollup?.stage.name || (
              <span className="text-muted-foreground">unnamed</span>
            )}
          </h2>
          <div className="text-muted-foreground font-mono text-[10px]">
            {stageId}
          </div>
        </div>

        {error && (
          <div className="text-destructive text-xs">{error}</div>
        )}

        {rollup && (
          <div className="grid grid-cols-3 gap-3">
            <Stat label="duration" value={formatDuration(rollup)} />
            <Stat label="cost" value={formatCost(rollup.cost_cents)} />
            <Stat label="tasks" value={String(rollup.task_count)} />
          </div>
        )}

        {rollup?.stage.session_id ? (
          <Captured label="Claude session id" value={rollup.stage.session_id} />
        ) : (
          <Pending
            label="Claude session id"
            source="captured from RunResult.session_id; mock-runner stages never emit one"
          />
        )}
        <Pending
          label="Commits made in this stage"
          source="git log <branch> joined to stage timestamps"
        />
        <Pending
          label="Tool-call ribbon"
          source="rolled up from Event::ToolCall grouped by stage_id"
        />
        <Pending
          label="Final assistant message"
          source="last AiMessageComplete + buffered text from claude_runner.rs"
        />
      </div>
    </ScrollArea>
  );
}

function StatusPill({ status }: { status: string }) {
  const tone =
    status === "passed"
      ? "border-emerald-500/40 text-emerald-600 dark:text-emerald-400"
      : status === "failed"
        ? "border-destructive/40 text-destructive"
        : "border-border text-muted-foreground";
  return (
    <Badge variant="outline" className={`text-[10px] ${tone}`}>
      {status}
    </Badge>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-muted/40 rounded p-2.5">
      <div className="text-base font-medium">{value}</div>
      <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
        {label}
      </div>
    </div>
  );
}

// One placeholder card per wishlist item. Renders enough scaffolding
// that a future agent picking up the wishlist work has a concrete
// home for the data and can swap the placeholder for real content
// without reshaping the page.
// Sibling to Pending for wishlist items that now have data. Keeps
// the same outer card shape so the layout doesn't shift when a stage
// transitions from no-session to has-session.
function Captured({ label, value }: { label: string; value: string }) {
  return (
    <div className="border-border/40 bg-muted/20 rounded border p-3">
      <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
        {label}
      </div>
      <div className="mt-1 font-mono text-xs break-all">{value}</div>
    </div>
  );
}

function Pending({ label, source }: { label: string; source: string }) {
  return (
    <div className="border-border/40 bg-muted/20 rounded border border-dashed p-3">
      <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
        {label}
      </div>
      <div className="text-muted-foreground mt-1 text-xs">
        Not wired yet. Source: {source}.
      </div>
    </div>
  );
}

function formatDuration(rollup: StageRollup): string {
  const start = rollup.stage.started_at;
  const end = rollup.stage.ended_at;
  if (start === null) return "—";
  const ended = end ?? Date.now();
  const ms = ended - start;
  if (ms < 1000) return `${ms}ms`;
  const s = Math.round(ms / 100) / 10;
  if (s < 60) return `${s}s`;
  const minutes = Math.floor(s / 60);
  const remSec = Math.round(s - minutes * 60);
  return `${minutes}m ${remSec}s`;
}

function formatCost(cents: number): string {
  if (cents === 0) return "$0.00";
  return `$${(cents / 100).toFixed(2)}`;
}
