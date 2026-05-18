// Stage-6 coverage for todo rows under the Stages tab.
//
// Two contracts exercised here, neither covered elsewhere:
//
//   1. The reducer routes `todo-added` / `todo-updated` / `todo-completed`
//      to the right task without requiring an envelope `task_id` on the
//      transition events. The recorder is the source of truth for the
//      `(todo_id → task_id)` mapping at insert time; the UI mirrors
//      that with `state.todoIndex` so the gate-driving events can stay
//      payload-minimal.
//
//   2. The tick row aggregates its todos exactly as `JOB-UI.md` says:
//      `!` if any todo failed, `●` while any is in-progress, `✓` only
//      when every todo is `done` or `skipped`. This is the user-visible
//      payoff — without aggregation the tick row stays `●` for the
//      whole Claude session.
//
// Render coverage is light on purpose: the title is verbatim, the
// trio kind labels survive, and the trio sorts last because its
// ordinals are `u32::MAX - 2 ..= u32::MAX`. Everything else is the
// same render path verify-step and task child rows already use.

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { Event, EventEnvelope } from "@/lib/rpc/wire";

import {
  aggregateTaskStatus,
  applyEvent,
  effectiveTaskStatus,
  emptyStagesState,
  StagesOverview,
  type TaskRow,
  type TodoRow,
} from "./StagesOverview";

const JOB = "01HJOB0000000000000000000000";
const STAGE = "01HSTAGE0000000000000000000";
const TASK = "01HTASK00000000000000000000";

function env(event: Event, overrides: Partial<EventEnvelope> = {}): EventEnvelope {
  return {
    cursor: 1,
    job_id: JOB,
    stage_id: STAGE,
    task_id: null,
    created_at: 1_700_000_000_000,
    event,
    ...overrides,
  };
}

function todo(overrides: Partial<TodoRow> = {}): TodoRow {
  return {
    todoId: "01HTODO0000000000000000000A",
    ordinal: 0,
    title: "do the thing",
    kind: "runner",
    status: "pending",
    ...overrides,
  };
}

function task(todos: TodoRow[], status: TaskRow["status"] = "running"): TaskRow {
  return {
    kind: "task",
    taskId: TASK,
    ordinal: 1,
    status,
    todos,
  };
}

describe("StagesOverview reducer — todo events", () => {
  function seeded() {
    let s = emptyStagesState();
    s = applyEvent(
      s,
      env({ type: "stage-started", stage_id: STAGE, job_id: JOB, name: "auth", ordinal: 0 }),
    );
    s = applyEvent(
      s,
      env({
        type: "task-enqueued",
        task_id: TASK,
        stage_id: STAGE,
        depends_on: [],
      }),
    );
    return s;
  }

  it("appends a todo row under the right task on todo-added", () => {
    const state = applyEvent(
      seeded(),
      env({
        type: "todo-added",
        todo_id: "01HTODO0000000000000000000A",
        task_id: TASK,
        ordinal: 0,
        title: "scan the routes",
        kind: "runner",
      }),
    );
    const stage = state.stages.get(STAGE)!;
    const t = stage.children.find((c) => c.kind === "task" && c.taskId === TASK);
    expect(t).toBeDefined();
    if (t?.kind !== "task") throw new Error("expected task row");
    expect(t.todos).toEqual([
      {
        todoId: "01HTODO0000000000000000000A",
        ordinal: 0,
        title: "scan the routes",
        kind: "runner",
        status: "pending",
      },
    ]);
    expect(state.todoIndex.get("01HTODO0000000000000000000A")).toEqual({
      stageId: STAGE,
      taskId: TASK,
    });
  });

  it("sorts the runtime-injected trio below runner items", () => {
    // The runtime writes the trio with the three highest ordinals it
    // has seen for the task (in practice `u32::MAX - 2 ..= u32::MAX`).
    // Whatever the actual numbers, the trio must render below
    // runner-emitted items so the user sees the safety steps tick
    // after the real work.
    let state = seeded();
    state = applyEvent(
      state,
      env({
        type: "todo-added",
        todo_id: "T-checks",
        task_id: TASK,
        ordinal: 4_294_967_293, // u32::MAX - 2
        title: "checks",
        kind: "checks",
      }),
    );
    state = applyEvent(
      state,
      env({
        type: "todo-added",
        todo_id: "T-runner-0",
        task_id: TASK,
        ordinal: 0,
        title: "first runner item",
        kind: "runner",
      }),
    );
    state = applyEvent(
      state,
      env({
        type: "todo-added",
        todo_id: "T-git",
        task_id: TASK,
        ordinal: 4_294_967_295, // u32::MAX
        title: "git",
        kind: "git",
      }),
    );
    state = applyEvent(
      state,
      env({
        type: "todo-added",
        todo_id: "T-docs",
        task_id: TASK,
        ordinal: 4_294_967_294, // u32::MAX - 1
        title: "docs",
        kind: "docs",
      }),
    );
    const stage = state.stages.get(STAGE)!;
    const tasks = stage.children.filter((c): c is TaskRow => c.kind === "task");
    const ids = tasks[0].todos.map((t) => t.todoId);
    expect(ids).toEqual(["T-runner-0", "T-checks", "T-docs", "T-git"]);
  });

  it("routes todo-updated through todoIndex without an envelope task_id", () => {
    let state = seeded();
    state = applyEvent(
      state,
      env({
        type: "todo-added",
        todo_id: "01HTODO0000000000000000000A",
        task_id: TASK,
        ordinal: 0,
        title: "scan the routes",
        kind: "runner",
      }),
    );
    // The trio-emitter and the upstream forwarder publish `todo-updated`
    // with `task_id: TASK` on the envelope, but the gate logic never
    // requires it: drop both the envelope's task_id and stage_id to
    // prove the index-based routing is the load-bearing path.
    state = applyEvent(
      state,
      env(
        {
          type: "todo-updated",
          todo_id: "01HTODO0000000000000000000A",
          status: "in-progress",
        },
        { stage_id: null, task_id: null },
      ),
    );
    const stage = state.stages.get(STAGE)!;
    const t = stage.children.find((c) => c.kind === "task") as TaskRow;
    expect(t.todos[0].status).toBe("in-progress");
  });

  it("routes todo-completed and ignores events with no matching todo", () => {
    let state = seeded();
    state = applyEvent(
      state,
      env({
        type: "todo-added",
        todo_id: "01HTODO0000000000000000000A",
        task_id: TASK,
        ordinal: 0,
        title: "scan the routes",
        kind: "runner",
      }),
    );
    state = applyEvent(
      state,
      env({
        type: "todo-completed",
        todo_id: "01HTODO0000000000000000000A",
        status: "done",
      }),
    );
    // Unknown todo id: dropped.
    state = applyEvent(
      state,
      env({
        type: "todo-completed",
        todo_id: "unknown-todo",
        status: "done",
      }),
    );
    const stage = state.stages.get(STAGE)!;
    const t = stage.children.find((c) => c.kind === "task") as TaskRow;
    expect(t.todos[0].status).toBe("done");
  });

  it("drops todo-added for an unseen stage rather than synthesising one", () => {
    const before = emptyStagesState();
    const after = applyEvent(
      before,
      env({
        type: "todo-added",
        todo_id: "01HTODO0000000000000000000A",
        task_id: TASK,
        ordinal: 0,
        title: "x",
        kind: "runner",
      }),
    );
    expect(after).toBe(before);
  });
});

describe("StagesOverview reducer — tick aggregation", () => {
  it("aggregates `✓` only when every todo is done or skipped", () => {
    expect(
      aggregateTaskStatus([
        todo({ todoId: "a", status: "done" }),
        todo({ todoId: "b", status: "skipped" }),
      ]),
    ).toBe("passed");
  });

  it("flips to `!` as soon as any todo fails", () => {
    expect(
      aggregateTaskStatus([
        todo({ todoId: "a", status: "done" }),
        todo({ todoId: "b", status: "failed" }),
        todo({ todoId: "c", status: "in-progress" }),
      ]),
    ).toBe("failed");
  });

  it("shows `●` while any todo is in-progress", () => {
    expect(
      aggregateTaskStatus([
        todo({ todoId: "a", status: "done" }),
        todo({ todoId: "b", status: "in-progress" }),
        todo({ todoId: "c", status: "pending" }),
      ]),
    ).toBe("running");
  });

  it("falls back to event-derived task status when there are no todos", () => {
    expect(effectiveTaskStatus(task([], "queued"))).toBe("queued");
    expect(effectiveTaskStatus(task([], "running"))).toBe("running");
    expect(effectiveTaskStatus(task([], "passed"))).toBe("passed");
    expect(effectiveTaskStatus(task([], "failed"))).toBe("failed");
  });

  it("never demotes a terminal task to a todo-derived running state", () => {
    // The runtime's terminal answer wins. Once `task-completed` lands,
    // a still-`in-progress` todo (recorder lag) cannot make the tick
    // re-render as `●`.
    const t = task(
      [
        todo({ todoId: "a", status: "in-progress" }),
        todo({ todoId: "b", status: "pending" }),
      ],
      "passed",
    );
    expect(effectiveTaskStatus(t)).toBe("passed");
  });
});

describe("StagesOverview render — todo rows", () => {
  afterEach(() => cleanup());

  it("renders runner titles verbatim and labels the closing trio", async () => {
    const client = new MockRpcClient();
    render(
      <RpcProvider client={client}>
        <StagesOverview jobId={JOB} />
      </RpcProvider>,
    );

    // The mock's auto-synthesised lifecycle does not emit todo events.
    // Drive a hand-crafted timeline through the same subscriber path
    // by reaching into the mock; the per-emit publisher is the seam
    // the rest of the UI plumbs against.
    const emit = (
      ev: Event,
      stageId: string | null = STAGE,
      taskId: string | null = TASK,
    ) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (client as any).emit(ev, JOB, stageId, taskId);
    };

    emit({ type: "stage-started", stage_id: STAGE, job_id: JOB, name: "auth", ordinal: 0 });
    emit({
      type: "task-enqueued",
      task_id: TASK,
      stage_id: STAGE,
      depends_on: [],
    });
    emit({ type: "task-started", task_id: TASK });
    emit({
      type: "todo-added",
      todo_id: "todo-1",
      task_id: TASK,
      ordinal: 0,
      title: "wire bearer middleware",
      kind: "runner",
    });
    emit({
      type: "todo-added",
      todo_id: "todo-checks",
      task_id: TASK,
      ordinal: 4_294_967_293,
      title: "checks",
      kind: "checks",
    });
    emit({
      type: "todo-added",
      todo_id: "todo-docs",
      task_id: TASK,
      ordinal: 4_294_967_294,
      title: "docs",
      kind: "docs",
    });
    emit({
      type: "todo-added",
      todo_id: "todo-git",
      task_id: TASK,
      ordinal: 4_294_967_295,
      title: "git",
      kind: "git",
    });
    emit({ type: "todo-updated", todo_id: "todo-1", status: "in-progress" });

    await screen.findByText("wire bearer middleware");

    const rows = await screen.findAllByTestId("todo-row");
    expect(rows).toHaveLength(4);
    // Order matches ordinal: runner item first, trio last in
    // checks → docs → git order.
    expect(rows.map((r) => r.getAttribute("data-todo-kind"))).toEqual([
      "runner",
      "checks",
      "docs",
      "git",
    ]);
    expect(rows[0].getAttribute("data-todo-status")).toBe("in-progress");
    // The trio kind labels render so the user sees the safety-step
    // names. Each trio row writes its kind twice (label column +
    // title column, because the title is "checks"/"docs"/"git" too);
    // the runner row has no label column, only its verbatim title.
    expect(screen.getAllByText("checks")).toHaveLength(2);
    expect(screen.getAllByText("docs")).toHaveLength(2);
    expect(screen.getAllByText("git")).toHaveLength(2);
  });
});
