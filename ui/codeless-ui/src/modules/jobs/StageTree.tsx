import { useCallback, useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import {
  useEventStream,
  useRpc,
  type EventEnvelope,
  type JobId,
  type StageRollup,
} from "@/lib/rpc";

interface Props {
  jobId: JobId;
  /**
   * Optional template YAML the job was submitted with. When present,
   * the stage rows show the user-authored title from `stages:` rather
   * than the runtime's ULID. Ordering by event arrival matches the
   * template's order because the orchestrator iterates the list
   * sequentially (`template_runner.rs`).
   */
  templateYaml?: string | null;
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
// Augmented with persisted `list_stages` rollups when available: the
// StageRecorder writes per-stage rows with timing and tasks.cost_cents
// so the row can display duration + cost without the UI doing the math.
export function StageTree({ jobId, templateYaml }: Props) {
  const rpc = useRpc();
  const stageTitles = useMemo(
    () => parseTemplateStageTitles(templateYaml ?? null),
    [templateYaml],
  );
  const [stages, setStages] = useState<Map<string, StageRow>>(new Map());
  const [order, setOrder] = useState<string[]>([]);
  const [rollups, setRollups] = useState<Map<string, StageRollup>>(new Map());

  useEffect(() => {
    setStages(new Map());
    setOrder([]);
    setRollups(new Map());
  }, [jobId]);

  // Pull the persisted rollups on mount + after every stage-completed
  // event so the row picks up the recorded duration/cost as soon as
  // the stage terminates. Catches the pre-recorder fallback too:
  // `stages` is empty → the existing event-derived view shows alone.
  const fetchRollups = useCallback(async () => {
    try {
      const res = await rpc.call("list_stages", { job_id: jobId });
      const map = new Map<string, StageRollup>();
      const seen: string[] = [];
      for (const r of res.stages) {
        map.set(r.stage.id, r);
        seen.push(r.stage.id);
      }
      setRollups(map);
      // Seed the ordering from the persisted stages so we don't depend
      // on the event stream having delivered every StageStarted before
      // the user opened the page.
      setOrder((prev) => {
        const merged = [...seen];
        for (const id of prev) {
          if (!merged.includes(id)) merged.push(id);
        }
        return merged;
      });
    } catch {
      // Pre-recorder jobs / wiped DB: silent. The event-derived view
      // is the fallback; nothing here is load-bearing.
    }
  }, [rpc, jobId]);
  useEffect(() => {
    void fetchRollups();
  }, [fetchRollups]);

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
    // Refresh rollups when a stage terminates so we pick up the
    // recorded duration + cost without polling.
    if (e.type === "stage-completed" || e.type === "verify-failed") {
      void fetchRollups();
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
    () =>
      order
        .map((id) => {
          // Synthesise a stage row from the rollup when the event
          // stream hasn't delivered StageStarted yet (page opened
          // after the events were lost from the broadcast tail).
          const existing = stages.get(id);
          if (existing) return existing;
          const rollup = rollups.get(id);
          if (!rollup) return null;
          return {
            id,
            state:
              rollup.stage.status === "failed"
                ? "failed"
                : rollup.stage.status === "passed"
                  ? "completed"
                  : "running",
            finalStatus:
              rollup.stage.status === "passed" || rollup.stage.status === "failed"
                ? rollup.stage.status
                : null,
            verifyExit: null,
          } satisfies StageRow;
        })
        .filter((r): r is StageRow => !!r),
    [order, stages, rollups],
  );

  // Header summary: total wall time + total cost across persisted
  // stages. Only renders when we have at least one rollup. Lets the
  // user see "this run cost N cents over M seconds" without doing
  // the math on each row.
  const totals = useMemo(() => {
    if (rollups.size === 0) return null;
    let costCents = 0;
    let minStart: number | null = null;
    let maxEnd: number | null = null;
    for (const r of rollups.values()) {
      costCents += r.cost_cents;
      if (r.stage.started_at !== null) {
        minStart = minStart === null ? r.stage.started_at : Math.min(minStart, r.stage.started_at);
      }
      if (r.stage.ended_at !== null) {
        maxEnd = maxEnd === null ? r.stage.ended_at : Math.max(maxEnd, r.stage.ended_at);
      }
    }
    const durationMs = minStart !== null && maxEnd !== null ? maxEnd - minStart : null;
    return { costCents, durationMs };
  }, [rollups]);

  // Hide only when neither the live event stream nor the template
  // can tell us anything — for a template-backed job we want to
  // render the *planned* stages even before the runner has emitted
  // a single per-stage event, so the user sees the full plan from
  // the moment the page opens.
  if (rows.length === 0 && stageTitles.length === 0) return null;

  return (
    <div className="border-border/50 border-b px-4 py-2">
      <div className="mb-1.5 flex items-baseline justify-between">
        <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
          Stages
        </div>
        {totals && (
          <div className="text-muted-foreground flex gap-3 text-[10px]">
            {totals.durationMs !== null && (
              <span title="Total wall time across all recorded stages">
                {formatDuration(totals.durationMs)}
              </span>
            )}
            <span title="Total cost across all recorded stages">
              {formatCost(totals.costCents)}
            </span>
          </div>
        )}
      </div>
      <ul className="space-y-0.5">
        {rows.map((r, i) => (
          <StageLine
            key={r.id}
            row={r}
            title={stageTitles[i] ?? rollups.get(r.id)?.stage.name ?? null}
            rollup={rollups.get(r.id) ?? null}
          />
        ))}
        {/*
          When the template names more stages than have started yet,
          render the remaining ones in a "pending" state so the user
          sees the full plan, not just what has happened so far.
        */}
        {stageTitles.slice(rows.length).map((title, i) => (
          <PendingStageLine key={`pending-${i}`} title={title} />
        ))}
      </ul>
    </div>
  );
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = Math.round(ms / 100) / 10;
  if (s < 60) return `${s}s`;
  const minutes = Math.floor(s / 60);
  const remSec = Math.round(s - minutes * 60);
  return `${minutes}m ${remSec}s`;
}

function formatCost(cents: number): string {
  if (cents === 0) return "$0.00";
  // Cents come in as integer cents; show $X.XX with two decimals.
  return `$${(cents / 100).toFixed(2)}`;
}

function StageLine({
  row,
  title,
  rollup,
}: {
  row: StageRow;
  title: string | null;
  rollup: StageRollup | null;
}) {
  const { glyph, tone, label } = renderStage(row);
  // Prefer the template's user-authored title when we have it; fall
  // back to the ULID otherwise. Either way the row's status glyph is
  // the load-bearing signal, so the right-side label is supplemental.
  const primary = title ?? row.id;
  // Per-stage rollup: duration + cost, only when the StageRecorder
  // captured them. Pre-recorder jobs render the existing event-only
  // line (no muted span at all).
  let duration: string | null = null;
  if (rollup?.stage.started_at !== null && rollup?.stage.started_at !== undefined) {
    const end =
      rollup.stage.ended_at !== null && rollup.stage.ended_at !== undefined
        ? rollup.stage.ended_at
        : Date.now();
    duration = formatDuration(end - rollup.stage.started_at);
  }
  const cost = rollup && rollup.cost_cents > 0 ? formatCost(rollup.cost_cents) : null;
  const tasks = rollup && rollup.task_count > 0 ? `${rollup.task_count} task${rollup.task_count === 1 ? "" : "s"}` : null;
  return (
    <li className="flex items-baseline gap-2 text-xs">
      <span className={`w-3 text-center font-mono ${tone}`} aria-hidden>
        {glyph}
      </span>
      <span
        className={cn(
          "min-w-0 flex-1 truncate",
          title ? "text-foreground" : "text-muted-foreground font-mono text-[11px]",
        )}
        title={title ?? row.id}
      >
        {primary}
      </span>
      {duration && (
        <span className="text-muted-foreground shrink-0 text-[10px]" title="Duration">
          {duration}
        </span>
      )}
      {cost && (
        <span className="text-muted-foreground shrink-0 text-[10px]" title="Cost">
          {cost}
        </span>
      )}
      {tasks && !cost && (
        <span className="text-muted-foreground shrink-0 text-[10px]" title="Task count">
          {tasks}
        </span>
      )}
      {label && (
        <span className={`shrink-0 text-[11px] ${tone}`} title={label}>
          {label}
        </span>
      )}
    </li>
  );
}

function PendingStageLine({ title }: { title: string }) {
  return (
    <li className="flex items-baseline gap-2 text-xs opacity-60">
      <span className="text-muted-foreground w-3 text-center font-mono" aria-hidden>
        ·
      </span>
      <span
        className="text-muted-foreground min-w-0 flex-1 truncate"
        title={title}
      >
        {title}
      </span>
    </li>
  );
}

// Extract the user-authored stage titles from the template YAML. We
// reach for a regex rather than a full YAML parse: the surface is
// known (one `stages:` block, each entry is a `- ` line), the parse
// surface is tiny, and the wire shape is stable. If `stages:` is
// absent or the YAML is something unusual, we return an empty list
// and the timeline falls back to ULID labels — no error surface
// because this is best-effort metadata, not load-bearing routing.
function parseTemplateStageTitles(yaml: string | null): string[] {
  if (!yaml) return [];
  const stagesIdx = yaml.search(/^\s*stages\s*:\s*$/m);
  if (stagesIdx < 0) return [];
  const after = yaml.slice(stagesIdx);
  const lines = after.split("\n").slice(1);
  const titles: string[] = [];
  for (const raw of lines) {
    const m = raw.match(/^\s*-\s+(.*)$/);
    if (m) {
      titles.push(stripReviewPrefix(m[1].trim()));
      continue;
    }
    if (raw.trim() === "") continue;
    // Stop at the first non-bullet, non-blank line. That's the end of
    // the `stages:` block (a sibling key in the YAML).
    if (/^\S/.test(raw)) break;
  }
  return titles;
}

function stripReviewPrefix(title: string): string {
  return title.replace(/^REVIEW\s+/, "");
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
