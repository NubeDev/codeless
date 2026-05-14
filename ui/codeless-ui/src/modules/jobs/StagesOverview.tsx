import { useCallback, useEffect, useMemo, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import {
  useEventStream,
  useRpc,
  type EventEnvelope,
  type JobId,
  type StageRollup,
} from "@/lib/rpc";

import { RestartMenu } from "./RestartMenu";

// ------------------------------------------------------------------ types

type StageStatus = "pending" | "running" | "passed" | "failed";
type TaskStatus = "queued" | "running" | "passed" | "failed";
type VerifyStepStatus = "running" | "passed" | "failed" | "skipped";

interface TaskRow {
  kind: "task";
  taskId: string;
  // 1-based display ordinal within the stage. Tasks are numbered in
  // arrival order (task-enqueued); the spec calls them "tick N".
  ordinal: number;
  status: TaskStatus;
}

interface VerifyStepRow {
  kind: "verify-step";
  stepIndex: number;
  name: string;
  status: VerifyStepStatus;
  // Wall-clock duration of the step in ms; only present on passed steps.
  durationMs: number | null;
  // Last ~16 lines of merged stdout+stderr; only present on failed steps.
  tail: string | null;
  exitCode: number | null;
}

type ChildRow = TaskRow | VerifyStepRow;

interface StageData {
  id: string;
  status: StageStatus;
  name: string | null;
  ordinal: number | null;
  startedAt: number | null;
  endedAt: number | null;
  costCents: number;
  children: ChildRow[];
}

// ------------------------------------------------------------------ helpers

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
  return `$${(cents / 100).toFixed(2)}`;
}

// ------------------------------------------------------------------ glyph table

interface Glyph {
  char: string;
  tone: string;
  label: string;
}

function stageGlyph(status: StageStatus): Glyph {
  switch (status) {
    case "passed":
      return { char: "✓", tone: "text-emerald-600 dark:text-emerald-400", label: "passed" };
    case "running":
      return { char: "●", tone: "text-blue-500", label: "running" };
    case "failed":
      return { char: "!", tone: "text-destructive", label: "failed" };
    case "pending":
      return { char: "○", tone: "text-muted-foreground", label: "queued" };
  }
}

function taskGlyph(status: TaskStatus): Glyph {
  switch (status) {
    case "passed":
      return { char: "✓", tone: "text-emerald-600 dark:text-emerald-400", label: "passed" };
    case "running":
      return { char: "●", tone: "text-blue-500", label: "running" };
    case "failed":
      return { char: "!", tone: "text-destructive", label: "failed" };
    case "queued":
      return { char: "○", tone: "text-muted-foreground", label: "queued" };
  }
}

function verifyStepGlyph(status: VerifyStepStatus): Glyph {
  switch (status) {
    case "passed":
      return { char: "✓", tone: "text-emerald-600 dark:text-emerald-400", label: "passed" };
    case "running":
      return { char: "●", tone: "text-blue-500", label: "running" };
    case "failed":
      return { char: "!", tone: "text-destructive", label: "failed" };
    case "skipped":
      return { char: "—", tone: "text-muted-foreground", label: "skipped" };
  }
}

// ------------------------------------------------------------------ reducer

interface StagesState {
  // Stable insertion-order list of stage ids.
  order: string[];
  stages: Map<string, StageData>;
}

function applyEvent(state: StagesState, env: EventEnvelope): StagesState {
  const e = env.event;
  // Stage-id is carried on the envelope's denormalised column or as a
  // field on the event itself; prefer the envelope column when available.
  const stageId =
    env.stage_id ??
    ("stage_id" in e && typeof e.stage_id === "string" ? e.stage_id : null);

  switch (e.type) {
    case "stage-started": {
      const sid = e.stage_id;
      const existing = state.stages.get(sid);
      const updated: StageData = {
        id: sid,
        status: "running",
        name: e.name ?? existing?.name ?? null,
        ordinal: e.ordinal ?? existing?.ordinal ?? null,
        startedAt: env.created_at,
        endedAt: existing?.endedAt ?? null,
        costCents: existing?.costCents ?? 0,
        children: existing?.children ?? [],
      };
      const nextStages = new Map(state.stages);
      nextStages.set(sid, updated);
      const nextOrder = state.order.includes(sid)
        ? state.order
        : [...state.order, sid];
      return { order: nextOrder, stages: nextStages };
    }

    case "stage-completed": {
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const updated: StageData = {
        ...existing,
        status: e.status === "passed" ? "passed" : "failed",
        endedAt: env.created_at,
      };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, updated);
      return { ...state, stages: nextStages };
    }

    case "verify-failed": {
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const updated: StageData = { ...existing, status: "failed" };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, updated);
      return { ...state, stages: nextStages };
    }

    case "task-enqueued": {
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const alreadyExists = existing.children.some(
        (c) => c.kind === "task" && c.taskId === e.task_id,
      );
      if (alreadyExists) return state;
      const taskOrdinal =
        existing.children.filter((c) => c.kind === "task").length + 1;
      const newRow: TaskRow = {
        kind: "task",
        taskId: e.task_id,
        ordinal: taskOrdinal,
        status: "queued",
      };
      const updated: StageData = {
        ...existing,
        children: [...existing.children, newRow],
      };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, updated);
      return { ...state, stages: nextStages };
    }

    case "task-started": {
      const sid = stageId ?? env.stage_id;
      if (!sid) return state;
      const existing = state.stages.get(sid);
      if (!existing) return state;
      const updated: StageData = {
        ...existing,
        children: existing.children.map((c) =>
          c.kind === "task" && c.taskId === e.task_id
            ? { ...c, status: "running" as TaskStatus }
            : c,
        ),
      };
      // If the task isn't in children yet (task-started arrived before
      // task-enqueued due to ordering), synthesise it as running.
      const hasTask = updated.children.some(
        (c) => c.kind === "task" && c.taskId === e.task_id,
      );
      const final = hasTask
        ? updated
        : {
            ...updated,
            children: [
              ...updated.children,
              {
                kind: "task" as const,
                taskId: e.task_id,
                ordinal:
                  updated.children.filter((c) => c.kind === "task").length + 1,
                status: "running" as TaskStatus,
              },
            ],
          };
      const nextStages = new Map(state.stages);
      nextStages.set(sid, final);
      return { ...state, stages: nextStages };
    }

    case "task-completed": {
      const sid = stageId ?? env.stage_id;
      if (!sid) return state;
      const existing = state.stages.get(sid);
      if (!existing) return state;
      const termStatus: TaskStatus =
        e.status === "completed" ? "passed" : "failed";
      const updated: StageData = {
        ...existing,
        children: existing.children.map((c) =>
          c.kind === "task" && c.taskId === e.task_id
            ? { ...c, status: termStatus }
            : c,
        ),
      };
      const nextStages = new Map(state.stages);
      nextStages.set(sid, updated);
      return { ...state, stages: nextStages };
    }

    case "verify-step-started": {
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const alreadyExists = existing.children.some(
        (c) =>
          c.kind === "verify-step" && c.stepIndex === e.step_index,
      );
      if (alreadyExists) {
        const updated: StageData = {
          ...existing,
          children: existing.children.map((c) =>
            c.kind === "verify-step" && c.stepIndex === e.step_index
              ? { ...c, status: "running" as VerifyStepStatus }
              : c,
          ),
        };
        const nextStages = new Map(state.stages);
        nextStages.set(stageId, updated);
        return { ...state, stages: nextStages };
      }
      const newStep: VerifyStepRow = {
        kind: "verify-step",
        stepIndex: e.step_index,
        name: e.name,
        status: "running",
        durationMs: null,
        tail: null,
        exitCode: null,
      };
      const updated: StageData = {
        ...existing,
        children: [...existing.children, newStep],
      };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, updated);
      return { ...state, stages: nextStages };
    }

    case "verify-step-passed": {
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const updated: StageData = {
        ...existing,
        children: existing.children.map((c) =>
          c.kind === "verify-step" && c.stepIndex === e.step_index
            ? {
                ...c,
                status: "passed" as VerifyStepStatus,
                durationMs: e.duration_ms,
              }
            : c,
        ),
      };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, updated);
      return { ...state, stages: nextStages };
    }

    case "verify-step-failed": {
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const updated: StageData = {
        ...existing,
        children: existing.children.map((c) =>
          c.kind === "verify-step" && c.stepIndex === e.step_index
            ? {
                ...c,
                status: "failed" as VerifyStepStatus,
                exitCode: e.exit_code,
                tail: e.tail,
              }
            : c,
        ),
      };
      // If the step row doesn't exist yet, synthesise it.
      const hasStep = existing.children.some(
        (c) => c.kind === "verify-step" && c.stepIndex === e.step_index,
      );
      const final = hasStep
        ? updated
        : {
            ...updated,
            children: [
              ...updated.children,
              {
                kind: "verify-step" as const,
                stepIndex: e.step_index,
                name: e.name,
                status: "failed" as VerifyStepStatus,
                durationMs: null,
                tail: e.tail,
                exitCode: e.exit_code,
              },
            ],
          };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, final);
      return { ...state, stages: nextStages };
    }

    case "verify-step-skipped": {
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const alreadyExists = existing.children.some(
        (c) => c.kind === "verify-step" && c.stepIndex === e.step_index,
      );
      if (alreadyExists) {
        const updated: StageData = {
          ...existing,
          children: existing.children.map((c) =>
            c.kind === "verify-step" && c.stepIndex === e.step_index
              ? { ...c, status: "skipped" as VerifyStepStatus }
              : c,
          ),
        };
        const nextStages = new Map(state.stages);
        nextStages.set(stageId, updated);
        return { ...state, stages: nextStages };
      }
      const newStep: VerifyStepRow = {
        kind: "verify-step",
        stepIndex: e.step_index,
        name: e.name,
        status: "skipped",
        durationMs: null,
        tail: null,
        exitCode: null,
      };
      const updated: StageData = {
        ...existing,
        children: [...existing.children, newStep],
      };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, updated);
      return { ...state, stages: nextStages };
    }

    default:
      return state;
  }
}

// Merge persisted stage rollups into the live event-driven state.
// The rollup carries authoritative name, ordinal, timing, and cost for
// stages that completed before the page opened; the event-driven state
// fills in the live children as events arrive.
function mergeRollup(state: StagesState, rollup: StageRollup): StagesState {
  const s = rollup.stage;
  const existing = state.stages.get(s.id);
  const status: StageStatus =
    s.status === "passed"
      ? "passed"
      : s.status === "failed"
        ? "failed"
        : s.status === "running"
          ? "running"
          : "pending";
  const merged: StageData = {
    id: s.id,
    status,
    name: s.name || existing?.name || null,
    ordinal: s.ordinal,
    startedAt: s.started_at ?? existing?.startedAt ?? null,
    endedAt: s.ended_at ?? existing?.endedAt ?? null,
    costCents: rollup.cost_cents,
    children: existing?.children ?? [],
  };
  const nextStages = new Map(state.stages);
  nextStages.set(s.id, merged);
  const nextOrder = state.order.includes(s.id)
    ? state.order
    : [...state.order, s.id];
  return { order: nextOrder, stages: nextStages };
}

// ------------------------------------------------------------------ component

interface Props {
  jobId: JobId;
  // Called when the user clicks a stage row, so the parent can open a
  // Stage-N detail tab. This stage (4) just emits; stage 5 builds the
  // detail view.
  onOpenStageTab?: (stageId: string, stageName: string) => void;
}

export function StagesOverview({ jobId, onOpenStageTab }: Props) {
  const rpc = useRpc();
  const [state, setState] = useState<StagesState>({
    order: [],
    stages: new Map(),
  });

  // Reset state when the job changes so stale children from a prior job
  // don't bleed through while the new event stream replays.
  useEffect(() => {
    setState({ order: [], stages: new Map() });
  }, [jobId]);

  // Seed from persisted rollups so completed stages are visible on cold
  // open before the event stream delivers its replay.
  useEffect(() => {
    let cancelled = false;
    rpc
      .call("list_stages", { job_id: jobId })
      .then((res) => {
        if (cancelled) return;
        setState((prev) => {
          let next = prev;
          // Sort by ordinal so the seeded order is stable.
          const sorted = [...res.stages].sort(
            (a, b) => a.stage.ordinal - b.stage.ordinal,
          );
          for (const r of sorted) {
            next = mergeRollup(next, r);
          }
          return next;
        });
      })
      .catch(() => {
        // Pre-recorder jobs: silent. The event-driven view is the fallback.
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId]);

  const onEvent = useCallback((env: EventEnvelope) => {
    setState((prev) => applyEvent(prev, env));
  }, []);

  useEventStream({ scope: "job", job_id: jobId }, onEvent);

  const rows = useMemo(
    () =>
      state.order
        .map((id) => state.stages.get(id))
        .filter((s): s is StageData => s !== undefined),
    [state],
  );

  if (rows.length === 0) {
    return (
      <div className="text-muted-foreground px-4 py-8 text-sm">
        No stages yet.
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="space-y-0 px-4 py-3">
        <div className="text-muted-foreground mb-3 text-[10px] font-semibold uppercase tracking-wider">
          Stages
        </div>
        <ul className="space-y-3">
          {rows.map((stage) => (
            <StageRow
              key={stage.id}
              stage={stage}
              jobId={jobId}
              onOpenStageTab={onOpenStageTab}
            />
          ))}
        </ul>
      </div>
    </ScrollArea>
  );
}

// ------------------------------------------------------------------ stage row

function StageRow({
  stage,
  jobId,
  onOpenStageTab,
}: {
  stage: StageData;
  jobId: JobId;
  onOpenStageTab?: (stageId: string, stageName: string) => void;
}) {
  const glyph = stageGlyph(stage.status);
  const hasChildren = stage.children.length > 0;

  const title =
    stage.name ?? `Stage ${stage.ordinal !== null ? stage.ordinal + 1 : "?"}`;

  let duration: string | null = null;
  if (stage.startedAt !== null) {
    const end = stage.endedAt ?? Date.now();
    duration = formatDuration(end - stage.startedAt);
  }
  const cost = stage.costCents > 0 ? formatCost(stage.costCents) : null;

  const canOpenTab = onOpenStageTab !== undefined;

  return (
    <li className="space-y-1">
      {/* Stage header row */}
      <div
        className={cn(
          "flex items-baseline gap-2 rounded px-1 py-1 text-sm",
          canOpenTab &&
            "cursor-pointer hover:bg-accent/40 transition-colors",
        )}
        onClick={
          canOpenTab
            ? () => onOpenStageTab(stage.id, title)
            : undefined
        }
        role={canOpenTab ? "button" : undefined}
        tabIndex={canOpenTab ? 0 : undefined}
        onKeyDown={
          canOpenTab
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onOpenStageTab?.(stage.id, title);
                }
              }
            : undefined
        }
      >
        <span
          className={cn("w-3 shrink-0 text-center font-mono", glyph.tone)}
          aria-label={glyph.label}
        >
          {glyph.char}
        </span>
        <span className="font-medium">{title}</span>
        <span className="text-muted-foreground min-w-0 flex-1 truncate font-mono text-[10px]">
          {stage.id.slice(0, 8)}
        </span>
        {duration && (
          <span
            className="text-muted-foreground shrink-0 text-xs"
            title="Duration"
          >
            {duration}
          </span>
        )}
        {cost && (
          <span
            className="text-muted-foreground shrink-0 text-xs"
            title="Cost"
          >
            {cost}
          </span>
        )}
      </div>

      {/* Child rows (ticks + verify steps) */}
      {hasChildren && (
        <ul className="ml-5 space-y-0.5 border-l border-border/30 pl-3">
          {stage.children.map((child, i) =>
            child.kind === "task" ? (
              <TaskChildRow key={`task-${child.taskId}`} row={child} />
            ) : (
              <VerifyStepChildRow
                key={`vs-${child.stepIndex}-${i}`}
                row={child}
                jobId={jobId}
              />
            ),
          )}
        </ul>
      )}
    </li>
  );
}

// ------------------------------------------------------------------ child rows

function TaskChildRow({ row }: { row: TaskRow }) {
  const glyph = taskGlyph(row.status);
  return (
    <li className="flex items-baseline gap-2 text-xs">
      <span
        className={cn("w-3 shrink-0 text-center font-mono", glyph.tone)}
        aria-label={glyph.label}
      >
        {glyph.char}
      </span>
      <span className="text-muted-foreground w-10 shrink-0">
        tick {row.ordinal}
      </span>
      <span className="text-muted-foreground min-w-0 flex-1 truncate font-mono text-[10px]">
        {row.taskId.slice(0, 8)}
      </span>
    </li>
  );
}

function VerifyStepChildRow({
  row,
  jobId,
}: {
  row: VerifyStepRow;
  jobId: JobId;
}) {
  const glyph = verifyStepGlyph(row.status);
  const isFailed = row.status === "failed";
  return (
    <li className="space-y-1">
      <div className="flex items-baseline gap-2 text-xs">
        <span
          className={cn("w-3 shrink-0 text-center font-mono", glyph.tone)}
          aria-label={glyph.label}
        >
          {glyph.char}
        </span>
        <span className="text-muted-foreground w-10 shrink-0">
          {row.stepIndex === 0 && !isFailed ? "test" : "test"}
        </span>
        <span className={cn("min-w-0 flex-1 truncate", isFailed ? glyph.tone : "")}>
          {row.name}
        </span>
        {row.durationMs !== null && (
          <span className="text-muted-foreground shrink-0 text-[10px]">
            {formatDuration(row.durationMs)}
          </span>
        )}
        {isFailed && (
          <div className="shrink-0">
            <RestartMenu jobId={jobId} />
          </div>
        )}
      </div>
      {/* Show last few lines of failed output inline for context. */}
      {isFailed && row.tail !== null && (
        <pre className="bg-muted/40 text-muted-foreground ml-5 overflow-x-auto rounded px-2 py-1 text-[10px] leading-snug">
          {row.tail}
        </pre>
      )}
    </li>
  );
}
