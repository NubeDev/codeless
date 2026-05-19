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

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type { Event, EventEnvelope, Job, PausePoint } from "@/lib/rpc/wire";

import {
  aggregateTaskStatus,
  applyEvent,
  effectiveTaskStatus,
  emptyStagesState,
  stageGlyph,
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

// A failed-and-bypassed stage row reads as "the runtime recovered,
// keep watching" rather than "the runtime halted and is waiting".
// The two states share the same persisted `status: failed` so the
// audit trail is preserved; the divergence is purely render-side:
// `~` in muted-foreground (bypassed) vs `!` in destructive (halt).
// The stage title carries a tooltip naming the policy + the
// rail-level reason so the operator sees recovery happening at a
// glance instead of clicking through to the run log.
describe("StagesOverview render — bypassed-after-failure", () => {
  afterEach(() => cleanup());

  it("returns the muted `~` glyph only for failed + bypassed", () => {
    expect(stageGlyph("failed", false)).toMatchObject({
      char: "!",
      tone: "text-destructive",
    });
    expect(stageGlyph("failed", true)).toMatchObject({
      char: "~",
      tone: "text-muted-foreground",
      label: "bypassed after failure",
    });
    // The flag is ignored on non-failed rows — a passed/running row
    // with a stale `bypassed=true` (e.g. mid-rebuild) must not flip
    // its glyph; the failure column is what `bypassed` qualifies.
    expect(stageGlyph("passed", true).char).toBe("✓");
    expect(stageGlyph("running", true).char).toBe("●");
    expect(stageGlyph("pending", true).char).toBe("○");
  });

  it("renders the bypass tooltip from `stage-auto-bypassed` + `stage-completed`", async () => {
    let s = emptyStagesState();
    s = applyEvent(
      s,
      env({ type: "stage-started", stage_id: STAGE, job_id: JOB, name: "auth", ordinal: 0 }),
    );
    s = applyEvent(
      s,
      env({
        type: "stage-completed",
        stage_id: STAGE,
        status: "failed",
        failure_class: "pre-check-failed",
        failure_detail: "scope-patch path drift: src/auth/mod.rs",
      }),
    );
    s = applyEvent(
      s,
      env({
        type: "stage-auto-bypassed",
        stage_id: STAGE,
        policy_name: "Quick",
        comment_used: "ignored — UI reads policy_name only",
        applied_at: 1_700_000_001_000,
      }),
    );

    const stage = s.stages.get(STAGE)!;
    expect(stage.status).toBe("failed");
    expect(stage.bypassed).toBe(true);
    expect(stage.bypassedPolicy).toBe("Quick");
    expect(stage.failureDetail).toBe("scope-patch path drift: src/auth/mod.rs");

    // The reducer feeds the render path; pin the rendered tooltip
    // shape so a future refactor that splits the tooltip composer
    // out can't silently drop either side of the colon.
    const client = new MockRpcClient();
    render(
      <RpcProvider client={client}>
        <StagesOverview jobId={JOB} />
      </RpcProvider>,
    );
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
      type: "stage-completed",
      stage_id: STAGE,
      status: "failed",
      failure_class: "pre-check-failed",
      failure_detail: "scope-patch path drift: src/auth/mod.rs",
    });
    emit({
      type: "stage-auto-bypassed",
      stage_id: STAGE,
      policy_name: "Quick",
      comment_used: "ignored",
      applied_at: 1_700_000_001_000,
    });

    const title = await screen.findByText("auth");
    expect(title.getAttribute("data-bypassed")).toBe("true");
    expect(title.getAttribute("title")).toBe(
      "auto-bypassed by Quick: scope-patch path drift: src/auth/mod.rs",
    );
    // Glyph aria-label flips to the bypass copy so screen readers
    // also hear "recovered" instead of "halted".
    expect(screen.getByLabelText("bypassed after failure")).toBeTruthy();
  });

  it("falls back to a policy-less tooltip when failure_detail is absent", () => {
    let s = emptyStagesState();
    s = applyEvent(
      s,
      env({ type: "stage-started", stage_id: STAGE, job_id: JOB, name: "auth", ordinal: 0 }),
    );
    s = applyEvent(
      s,
      env({
        type: "stage-completed",
        stage_id: STAGE,
        status: "failed",
      }),
    );
    s = applyEvent(
      s,
      env({
        type: "stage-auto-bypassed",
        stage_id: STAGE,
        policy_name: "Relentless",
        comment_used: "",
        applied_at: 1_700_000_002_000,
      }),
    );
    const stage = s.stages.get(STAGE)!;
    expect(stage.failureDetail).toBeNull();
    expect(stage.bypassedPolicy).toBe("Relentless");
  });
});

// Stage 8 scope: the Stage overview renders an operator-declared
// planned-pause chip for every scheduled point on the job, and when
// the job is actually paused at one of those points the chip's Resume
// button advances the runner past it via the existing `resume_job`
// RPC. No new RPC, no new pause primitive — this test pins the wire
// shape the chip consumes and the click path the Resume button drives.
describe("StagesOverview render — planned-pause divider", () => {
  afterEach(() => cleanup());

  // Mint a draft job through the mock so `get_job` returns a row
  // (the mock indexes by id; ad-hoc test ids 404). Returns the seeded
  // `Job` so the test can mutate `status` / `stop_reason` between
  // renders to simulate the scoped-pause transition the runtime drives.
  async function seedJob(client: MockRpcClient): Promise<Job> {
    const repos = await client.call("list_repos", {});
    const repo = repos.repos[0];
    return client.call("submit_job", {
      repo_id: repo.id,
      template_yaml: "name: test\nstages:\n  - design\n  - implement\n",
      prompt: "do stage 2",
      runner: "claude",
      branch: "feat/test",
      workspace_mode: "in-repo",
      cost_cap_cents: 10_000,
      wall_clock_cap_ms: 600_000,
      model: null,
      permission_mode: null,
      effort: null,
      system_prompt: null,
      persona_id: null,
      auto_bypass_policy: null,
      start_immediately: false,
    });
  }

  it("renders a planned-pause chip per scheduled point in YAML order", async () => {
    const client = new MockRpcClient();
    const job = await seedJob(client);
    const point1: PausePoint = {
      id: "01HPP000000000000000000001",
      target: { kind: "stage", ordinal: 1 },
      position: "before",
      reason: "smoke test stage 1",
    };
    const point2: PausePoint = {
      id: "01HPP000000000000000000002",
      target: { kind: "stage", ordinal: 2 },
      position: "after",
      reason: null,
    };
    client.seedScheduledPausePoints(job.id, [point1, point2]);

    render(
      <RpcProvider client={client}>
        <StagesOverview jobId={job.id} />
      </RpcProvider>,
    );

    // Seed the stage rows the chips attach to. The mock's auto
    // lifecycle didn't run (start_immediately: false), so emit by hand.
    const emit = (ev: Event, sid: string | null) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (client as any).emit(ev, job.id, sid, null);
    };
    emit(
      { type: "stage-started", stage_id: "S-1", job_id: job.id, name: "design", ordinal: 0 },
      "S-1",
    );
    emit(
      {
        type: "stage-started",
        stage_id: "S-2",
        job_id: job.id,
        name: "implement",
        ordinal: 1,
      },
      "S-2",
    );

    const chips = await screen.findAllByTestId("planned-pause-chip");
    expect(chips).toHaveLength(2);
    // Chip 1: before stage 1 (the operator-authored reason wins as
    // the label, not the structural fallback).
    expect(chips[0].getAttribute("data-pause-point-id")).toBe(point1.id);
    expect(chips[0].getAttribute("data-pause-position")).toBe("before");
    expect(chips[0].textContent).toContain("smoke test stage 1");
    // Chip 2: after stage 2 (no reason — structural fallback wins).
    expect(chips[1].getAttribute("data-pause-point-id")).toBe(point2.id);
    expect(chips[1].getAttribute("data-pause-position")).toBe("after");
    expect(chips[1].textContent).toContain("pause after stage 2");
    // Neither chip is "fired" — the job is still in draft.
    for (const chip of chips) {
      expect(chip.getAttribute("data-pause-fired")).toBe("false");
    }
    expect(screen.queryByTestId("planned-pause-resume")).toBeNull();
  });

  it("surfaces a Resume button when paused on a scoped point and clears the pause on click", async () => {
    const client = new MockRpcClient();
    const job = await seedJob(client);
    const pointId = "01HPP00000000000000000FIRE";
    const point: PausePoint = {
      id: pointId,
      target: { kind: "stage", ordinal: 1 },
      position: "before",
      reason: "halt before design",
    };
    client.seedScheduledPausePoints(job.id, [point]);

    // Simulate the runtime: the scoped pause hook flipped the row to
    // `paused` and stamped `stop_reason = ScopedPausePoint { point_id }`.
    // The wire shape is the serde-JSON form (underscore `point_id`),
    // not the specta-hyphen form — the SSE pump uses serde-JSON.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const stored = (client as any).jobs.find((j: Job) => j.id === job.id) as Job;
    stored.status = "paused";
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (stored as any).stop_reason = { "scoped-pause-point": { point_id: pointId } };

    render(
      <RpcProvider client={client}>
        <StagesOverview jobId={job.id} />
      </RpcProvider>,
    );

    // Seed the stage row so the chip has a host to attach to.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (client as any).emit(
      { type: "stage-started", stage_id: "S-1", job_id: job.id, name: "design", ordinal: 0 },
      job.id,
      "S-1",
      null,
    );

    const chip = await screen.findByTestId("planned-pause-chip");
    await waitFor(() =>
      expect(chip.getAttribute("data-pause-fired")).toBe("true"),
    );
    const resumeBtn = screen.getByTestId("planned-pause-resume");
    expect(resumeBtn.textContent).toMatch(/Resume/i);

    await act(async () => {
      fireEvent.click(resumeBtn);
    });

    await waitFor(() => {
      expect(stored.status).toBe("queued");
      expect(stored.stop_reason).toBeNull();
    });
  });
});
