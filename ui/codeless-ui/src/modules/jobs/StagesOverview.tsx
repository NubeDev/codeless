import { useCallback, useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import {
  useEventStream,
  useJob,
  useRpc,
  type EventEnvelope,
  type JobId,
  type StageRollup,
} from "@/lib/rpc";
import type {
  PausePoint,
  PausePointPosition,
  PausePointTarget,
  TodoKind,
  TodoStatus,
} from "@/lib/rpc/wire";
import { scopedPausePointId } from "@/modules/chat/feed";

import { RestartMenu } from "./RestartMenu";

// ------------------------------------------------------------------ types

type StageStatus = "pending" | "running" | "passed" | "failed";
type TaskStatus = "queued" | "running" | "passed" | "failed";
type VerifyStepStatus = "running" | "passed" | "failed" | "skipped";

// One sub-step inside a task — runner-emitted (`TodoWrite`) or
// runtime-injected (the closing trio `Checks` / `Docs` / `Git`). The
// row's glyph flips `○ → ● → ✓` as `todo-updated` / `todo-completed`
// arrive. The trio's `Checks` / `Docs` / `Git` rows are load-bearing:
// `JOB-UI.md` says the stage cannot pass until all three are resolved,
// and the runtime enforces that gate.
export interface TodoRow {
  todoId: string;
  ordinal: number;
  title: string;
  kind: TodoKind;
  status: TodoStatus;
}

export interface TaskRow {
  kind: "task";
  taskId: string;
  // 1-based display ordinal within the stage. Tasks are numbered in
  // arrival order (task-enqueued); the spec calls them "tick N".
  ordinal: number;
  status: TaskStatus;
  // Todo rows nested under this tick. Kept sorted by ordinal so the
  // runtime's trio (`u32::MAX - 2 ..= u32::MAX`) consistently sorts
  // below any runner-emitted items (which start at 0).
  todos: TodoRow[];
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

// Per `JOB-UI.md`'s "Todo rows (nested under a tick)" glyph table:
// `○` pending, `●` in-progress, `✓` done, `!` failed, `~` skipped.
function todoGlyph(status: TodoStatus): Glyph {
  switch (status) {
    case "done":
      return { char: "✓", tone: "text-emerald-600 dark:text-emerald-400", label: "done" };
    case "in-progress":
      return { char: "●", tone: "text-blue-500", label: "in progress" };
    case "failed":
      return { char: "!", tone: "text-destructive", label: "failed" };
    case "skipped":
      return { char: "~", tone: "text-muted-foreground", label: "skipped" };
    case "pending":
      return { char: "○", tone: "text-muted-foreground", label: "pending" };
  }
}

// Trio rows render their kind ("checks" / "docs" / "git") as the label
// column so the user sees the safety-step name instead of "tick N".
// Runner-emitted items have no kind-label — their title is the only
// summary the runner gave us. `JOB-UI.md` treats the title as the
// runner's verbatim plan text, no codeless-side prettifying.
function todoKindLabel(kind: TodoKind): string | null {
  switch (kind) {
    case "checks":
      return "checks";
    case "docs":
      return "docs";
    case "git":
      return "git";
    case "runner":
    case "planner":
      return null;
  }
}

// Aggregated tick glyph when the tick has at least one todo. Spec
// (`JOB-UI.md` § "State that drives this UI"): `!` if any todo is
// failed, `●` while any todo is in-progress, `✓` only when every todo
// (including the closing trio) is `done` or `skipped`, else `○`.
// Without this aggregation the tick row stays `●` for the full Claude
// session — todos exist precisely to make that long middle visible.
export function aggregateTaskStatus(todos: TodoRow[]): TaskStatus {
  if (todos.some((t) => t.status === "failed")) return "failed";
  if (todos.some((t) => t.status === "in-progress")) return "running";
  const allResolved = todos.every(
    (t) => t.status === "done" || t.status === "skipped",
  );
  if (allResolved) return "passed";
  return "running";
}

// Compose the task's display status: event-driven `failed`/`passed`
// wins (the runtime spoke a terminal answer), otherwise let the todo
// aggregate drive the glyph so a 30-minute tick visibly progresses.
export function effectiveTaskStatus(task: TaskRow): TaskStatus {
  if (task.status === "failed") return "failed";
  if (task.status === "passed" && task.todos.length === 0) return "passed";
  if (task.todos.length > 0) {
    const agg = aggregateTaskStatus(task.todos);
    // Don't downgrade a passed task to running just because the trio
    // happens to be missing a `TodoCompleted` we never recorded.
    if (task.status === "passed" && agg !== "failed") return "passed";
    return agg;
  }
  return task.status;
}

// ------------------------------------------------------------------ reducer

export interface StagesState {
  // Stable insertion-order list of stage ids.
  order: string[];
  stages: Map<string, StageData>;
  // todo-id → (stage-id, task-id) routing. `todo-updated` and
  // `todo-completed` only carry `todo_id` on the event payload, so we
  // record where each todo lives at `todo-added` time and look the
  // row up here on transitions. Envelopes do carry `task_id` on the
  // recorder path, but the routing index keeps the reducer
  // independent of envelope denormalisation.
  todoIndex: Map<string, { stageId: string; taskId: string }>;
}

export function emptyStagesState(): StagesState {
  return { order: [], stages: new Map(), todoIndex: new Map() };
}

export function applyEvent(state: StagesState, env: EventEnvelope): StagesState {
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
      return { ...state, order: nextOrder, stages: nextStages };
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
        todos: [],
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
                todos: [],
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

    case "todo-added": {
      // `todo-added` carries `task_id`; the envelope carries `stage_id`.
      // If the parent stage hasn't been seen yet (replay arrived in
      // unexpected order), drop the event — the rollup seed plus a
      // later `stage-started` will reconstruct the row when it shows
      // up, and the recorder is the source of truth for missed
      // rebuilds. Likewise for an unknown task: synthesise the tick
      // so the todo has somewhere to hang, mirroring the
      // `task-started`-before-`task-enqueued` recovery above.
      if (!stageId) return state;
      const existing = state.stages.get(stageId);
      if (!existing) return state;
      const dupe = existing.children.some(
        (c) =>
          c.kind === "task" &&
          c.taskId === e.task_id &&
          c.todos.some((t) => t.todoId === e.todo_id),
      );
      if (dupe) return state;
      const newTodo: TodoRow = {
        todoId: e.todo_id,
        ordinal: e.ordinal,
        title: e.title,
        kind: e.kind,
        status: "pending",
      };
      let synthesised = false;
      const children = existing.children.map((c) => {
        if (c.kind === "task" && c.taskId === e.task_id) {
          const merged = [...c.todos, newTodo].sort(
            (a, b) => a.ordinal - b.ordinal,
          );
          return { ...c, todos: merged };
        }
        return c;
      });
      const hasTask = existing.children.some(
        (c) => c.kind === "task" && c.taskId === e.task_id,
      );
      if (!hasTask) {
        synthesised = true;
        children.push({
          kind: "task",
          taskId: e.task_id,
          ordinal:
            existing.children.filter((c) => c.kind === "task").length + 1,
          status: "running",
          todos: [newTodo],
        });
      }
      const updated: StageData = { ...existing, children };
      const nextStages = new Map(state.stages);
      nextStages.set(stageId, updated);
      const nextIndex = new Map(state.todoIndex);
      nextIndex.set(e.todo_id, { stageId, taskId: e.task_id });
      // `synthesised` is informational only; the surrounding code
      // already covers the empty-children path through the standard
      // task-started arm. Silence an unused-binding warning without
      // adding an emitting side effect.
      void synthesised;
      return { ...state, stages: nextStages, todoIndex: nextIndex };
    }

    case "todo-updated":
    case "todo-completed": {
      // Both events carry only `todo_id` + `status`. Look up the
      // owning task via the routing index; if the index is missing
      // the entry (we never saw `todo-added`), drop the event — the
      // recorder will rebuild on replay through `list_stages` and the
      // gate logic lives in the runtime, not here.
      const route = state.todoIndex.get(e.todo_id);
      if (!route) return state;
      const stage = state.stages.get(route.stageId);
      if (!stage) return state;
      const children = stage.children.map((c) => {
        if (c.kind === "task" && c.taskId === route.taskId) {
          return {
            ...c,
            todos: c.todos.map((t) =>
              t.todoId === e.todo_id ? { ...t, status: e.status } : t,
            ),
          };
        }
        return c;
      });
      const updated: StageData = { ...stage, children };
      const nextStages = new Map(state.stages);
      nextStages.set(route.stageId, updated);
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
  return { ...state, order: nextOrder, stages: nextStages };
}

// ------------------------------------------------------------------ component

interface Props {
  jobId: JobId;
  // Called when the user clicks a stage row, so the parent can open a
  // Stage-N detail tab. This stage (4) just emits; stage 5 builds the
  // detail view.
  onOpenStageTab?: (stageId: string, stageName: string) => void;
}

// One scheduled point grouped against the stage row it attaches to.
// `position` flips the chip placement to before/after the stage row;
// `firedHere` is true when the job is currently paused on this point
// (the row's `stop_reason` matches the point's id) — in which case the
// chip shows the Resume button, so the operator's "advance past this
// point" action lives next to the divider that named it.
interface ScheduledChip {
  point: PausePoint;
  position: PausePointPosition;
  firedHere: boolean;
}

// Slot one pause-point row into the stage it attaches to. `Stage` and
// `StageTodo` both carry a stage ordinal in the wire shape — for the
// chip placement we collapse stage-todo points onto the parent stage
// so the user sees one chip per declared row in the operator's YAML
// order. A future refinement can position the chip at the matching
// todo row; the chip text already names the selector so the user is
// not blind to it.
function stageOrdinalOfTarget(t: PausePointTarget): number {
  return t.kind === "stage" ? t.ordinal : t.stage_ordinal;
}

// Human label for the chip. Mirrors what stage 6's `point.reason`
// column carries when set; falls back to a structural description so
// a point with no operator reason still renders something useful.
function pausePointLabel(p: PausePoint): string {
  if (p.reason && p.reason.trim().length > 0) return p.reason;
  const pos = p.position === "before" ? "before" : "after";
  if (p.target.kind === "stage") {
    return `pause ${pos} stage ${p.target.ordinal}`;
  }
  const sel = p.target.selector;
  if ("kind" in sel) return `pause ${pos} stage ${p.target.stage_ordinal} ${sel.kind}`;
  if ("ordinal" in sel)
    return `pause ${pos} stage ${p.target.stage_ordinal} todo ${sel.ordinal}`;
  return `pause ${pos} stage ${p.target.stage_ordinal} ~${sel.pattern}`;
}

export function StagesOverview({ jobId, onOpenStageTab }: Props) {
  const rpc = useRpc();
  const { data: job, refetch: refetchJob } = useJob(jobId);
  const [state, setState] = useState<StagesState>({
    order: [],
    stages: new Map(),
    todoIndex: new Map(),
  });
  const [pausePoints, setPausePoints] = useState<PausePoint[]>([]);

  // Reset state when the job changes so stale children from a prior job
  // don't bleed through while the new event stream replays.
  useEffect(() => {
    setState({ order: [], stages: new Map(), todoIndex: new Map() });
    setPausePoints([]);
  }, [jobId]);

  // Seed the scoped pause schedule. The schedule is operator-authored
  // in `template.yaml` and persisted server-side; this is a read-only
  // mirror so the divider chips have something to render before the
  // first event arrives. Empty list when the job predates the feature
  // or carries no `pause_points:` block — that branch silently falls
  // back to "no chips", which is the correct empty state.
  useEffect(() => {
    let cancelled = false;
    rpc
      .call("list_scheduled_pause_points", { job_id: jobId })
      .then((res) => {
        if (!cancelled) setPausePoints(res.points);
      })
      .catch(() => {
        // Older runtimes (and the unit-test fixtures that don't seed
        // a schedule) will 404 or return an empty list; treat both as
        // "no schedule" — the rest of the overview keeps rendering.
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, jobId]);

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

  const firedPointId = scopedPausePointId(job?.stop_reason);

  const onResume = useCallback(async () => {
    await rpc.call("resume_job", {
      job_id: jobId,
      additional_cost_cap_cents: null,
      additional_wall_clock_cap_ms: null,
      next_stage_comment: null,
    });
    refetchJob();
  }, [rpc, jobId, refetchJob]);

  if (rows.length === 0) {
    return (
      <div className="text-muted-foreground px-4 py-8 text-sm">
        No stages yet.
      </div>
    );
  }

  // Group the schedule by 1-based stage ordinal so each `StageRow` can
  // pick out the chips that attach to it. The wire shape is stable;
  // the index is rebuilt from scratch on every render rather than
  // memoised because `pausePoints` is operator-authored and small
  // (under a dozen entries in any plausible plan).
  const chipsByStage = new Map<number, { before: ScheduledChip[]; after: ScheduledChip[] }>();
  for (const point of pausePoints) {
    const ordinal = stageOrdinalOfTarget(point.target);
    const slot = chipsByStage.get(ordinal) ?? { before: [], after: [] };
    const chip: ScheduledChip = {
      point,
      position: point.position,
      firedHere: firedPointId === point.id,
    };
    if (point.position === "before") slot.before.push(chip);
    else slot.after.push(chip);
    chipsByStage.set(ordinal, slot);
  }

  return (
    <ScrollArea className="h-full">
      <div className="space-y-0 px-4 py-3">
        <div className="text-muted-foreground mb-3 text-[10px] font-semibold uppercase tracking-wider">
          Stages
        </div>
        <ul className="space-y-3">
          {rows.map((stage) => {
            const stageOrdinal1 =
              stage.ordinal !== null ? stage.ordinal + 1 : null;
            const chips =
              stageOrdinal1 !== null ? chipsByStage.get(stageOrdinal1) : undefined;
            return (
              <li key={`stage-block-${stage.id}`} className="space-y-1">
                {chips?.before.map((c) => (
                  <PlannedPauseChip
                    key={`chip-before-${c.point.id}`}
                    chip={c}
                    onResume={onResume}
                  />
                ))}
                <StageRow
                  stage={stage}
                  jobId={jobId}
                  onOpenStageTab={onOpenStageTab}
                />
                {chips?.after.map((c) => (
                  <PlannedPauseChip
                    key={`chip-after-${c.point.id}`}
                    chip={c}
                    onResume={onResume}
                  />
                ))}
              </li>
            );
          })}
        </ul>
      </div>
    </ScrollArea>
  );
}

// ------------------------------------------------------------------ planned-pause chip

// One operator-declared pause point rendered inline with the stages.
// Distinct from a runtime-pause divider on purpose: the dashed border
// and "planned" label tell the user the runner was *scheduled* to halt
// here, not that they (or a cap) interrupted it. When the chip's point
// is the one the job is currently paused on, a `Resume` button appears
// in-place — same `resume_job` surface as the run strip, just located
// next to the divider that named the pause.
function PlannedPauseChip({
  chip,
  onResume,
}: {
  chip: ScheduledChip;
  onResume: () => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const fire = async () => {
    setBusy(true);
    setErr(null);
    try {
      await onResume();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const label = pausePointLabel(chip.point);
  const tone = chip.firedHere
    ? "border-amber-500/60 text-amber-700 dark:text-amber-300"
    : "border-border/60 text-muted-foreground";
  return (
    <div
      data-testid="planned-pause-chip"
      data-pause-point-id={chip.point.id}
      data-pause-position={chip.position}
      data-pause-fired={chip.firedHere ? "true" : "false"}
      className={cn(
        "flex items-center gap-2 rounded border border-dashed px-2 py-1 text-[11px]",
        tone,
      )}
    >
      <span aria-hidden="true">⏸</span>
      <span className="font-mono uppercase tracking-wide text-[9px]">
        planned
      </span>
      <span className="min-w-0 flex-1 truncate" title={label}>
        {label}
      </span>
      {chip.firedHere && (
        <Button
          size="sm"
          variant="outline"
          className="h-6 px-2 text-[11px]"
          onClick={fire}
          disabled={busy}
          data-testid="planned-pause-resume"
        >
          {busy ? "Resuming…" : "Resume"}
        </Button>
      )}
      {err && (
        <span className="text-destructive shrink-0 text-[10px]" title={err}>
          {err}
        </span>
      )}
    </div>
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
  // The tick row's glyph is the runtime's terminal answer (`failed` /
  // `passed`) when present; otherwise it is aggregated from the
  // nested todos so a long-running Claude session visibly progresses
  // tick-by-tick instead of staying `●` for half an hour.
  const glyph = taskGlyph(effectiveTaskStatus(row));
  return (
    <li className="space-y-0.5">
      <div className="flex items-baseline gap-2 text-xs">
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
      </div>
      {row.todos.length > 0 && (
        <ul
          className="ml-5 space-y-0 border-l border-border/20 pl-3"
          data-testid="todo-list"
        >
          {row.todos.map((todo) => (
            <TodoChildRow key={todo.todoId} row={todo} />
          ))}
        </ul>
      )}
    </li>
  );
}

function TodoChildRow({ row }: { row: TodoRow }) {
  const glyph = todoGlyph(row.status);
  const kindLabel = todoKindLabel(row.kind);
  return (
    <li
      className="flex items-baseline gap-2 text-[11px]"
      data-testid="todo-row"
      data-todo-kind={row.kind}
      data-todo-status={row.status}
    >
      <span
        className={cn("w-3 shrink-0 text-center font-mono", glyph.tone)}
        aria-label={glyph.label}
      >
        {glyph.char}
      </span>
      {kindLabel !== null ? (
        <span className="text-muted-foreground w-10 shrink-0 font-mono">
          {kindLabel}
        </span>
      ) : (
        // Reserve the same column width as the trio label so titles
        // line up across runner- and runtime-emitted rows.
        <span className="w-10 shrink-0" aria-hidden="true" />
      )}
      <span
        className={cn(
          "min-w-0 flex-1 truncate",
          row.status === "failed" ? glyph.tone : "text-foreground/90",
        )}
        title={row.title}
      >
        {row.title}
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
