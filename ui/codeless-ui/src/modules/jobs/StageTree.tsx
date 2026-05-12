import { useCallback, useEffect, useMemo, useState } from "react";

import { useEventStream, type EventEnvelope, type JobId } from "@/lib/rpc";

interface Props {
  jobId: JobId;
}

type StageState = "running" | "completed" | "failed";

interface StageRow {
  id: string;
  state: StageState;
  // Status string from the stage-completed event (e.g. "completed",
  // "skipped"). Only set once the stage terminates.
  finalStatus: string | null;
  // Exit code from the most recent verify-failed for this stage.
  // Used to surface verification failures without losing the
  // running/completed lifecycle bit.
  verifyExit: number | null;
}

// Live per-stage checklist driven by the same per-job event stream the
// timeline consumes. Folds stage-started, stage-completed, and
// verify-failed envelopes into a stable, ordered list of stage rows.
// Render-only — there is no backend involved here.
export function StageTree({ jobId }: Props) {
  const [stages, setStages] = useState<Map<string, StageRow>>(new Map());
  const [order, setOrder] = useState<string[]>([]);

  useEffect(() => {
    setStages(new Map());
    setOrder([]);
  }, [jobId]);

  const onEvent = useCallback((env: EventEnvelope) => {
    const e = env.event;
    const stageId = "stage_id" in e ? e.stage_id : env.stage_id;
    if (!stageId) return;
    if (
      e.type !== "stage-started" &&
      e.type !== "stage-completed" &&
      e.type !== "verify-failed"
    ) {
      return;
    }
    setStages((prev) => {
      const next = new Map(prev);
      const existing = next.get(stageId);
      const base: StageRow = existing ?? {
        id: stageId,
        state: "running",
        finalStatus: null,
        verifyExit: null,
      };
      let merged: StageRow = base;
      switch (e.type) {
        case "stage-started":
          merged = { ...base, state: "running" };
          break;
        case "stage-completed":
          merged = {
            ...base,
            state: "completed",
            finalStatus: e.status,
          };
          break;
        case "verify-failed":
          merged = {
            ...base,
            state: "failed",
            verifyExit: e.exit_code,
          };
          break;
      }
      next.set(stageId, merged);
      return next;
    });
    setOrder((prev) => (prev.includes(stageId) ? prev : [...prev, stageId]));
  }, []);

  useEventStream({ scope: "job", job_id: jobId }, onEvent);

  const rows = useMemo(
    () => order.map((id) => stages.get(id)).filter((r): r is StageRow => !!r),
    [order, stages],
  );

  if (rows.length === 0) return null;

  return (
    <div className="border-border/50 border-b px-4 py-2">
      <div className="text-muted-foreground mb-1.5 text-[10px] uppercase tracking-wide">
        Stages
      </div>
      <ul className="space-y-0.5">
        {rows.map((r) => (
          <StageLine key={r.id} row={r} />
        ))}
      </ul>
    </div>
  );
}

function StageLine({ row }: { row: StageRow }) {
  const { glyph, tone, label } = renderStage(row);
  return (
    <li className="flex items-baseline gap-2 text-xs">
      <span className={`w-3 text-center font-mono ${tone}`} aria-hidden>
        {glyph}
      </span>
      <span className="text-muted-foreground font-mono text-[11px]">
        {row.id}
      </span>
      {label && (
        <span className={`text-[11px] ${tone}`} title={label}>
          {label}
        </span>
      )}
    </li>
  );
}

// Stage-state glyph table. The "running" glyph is the Unicode ellipsis
// (U+2026), not three ASCII dots, so the row reads at the same width
// regardless of state. Failure renders bright; completion is muted on
// purpose — the timeline is the noisy view, the tree is the at-a-
// glance one.
function renderStage(row: StageRow): {
  glyph: string;
  tone: string;
  label: string | null;
} {
  if (row.state === "failed") {
    return {
      glyph: "!",
      tone: "text-destructive",
      label:
        row.verifyExit !== null ? `verify failed (exit ${row.verifyExit})` : "failed",
    };
  }
  if (row.state === "completed") {
    return {
      glyph: "✓",
      tone: "text-muted-foreground",
      label: row.finalStatus,
    };
  }
  return { glyph: "…", tone: "text-muted-foreground", label: null };
}
