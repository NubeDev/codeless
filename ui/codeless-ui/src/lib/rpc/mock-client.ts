// In-memory `RpcClient` for development and tests. Hold canned repo
// and job rows; `submit_job` appends to the in-memory store and
// synthesises a small event timeline so subscribers see the same
// lifecycle a real core would emit. No persistence, no network.

import type { RpcClient } from "./client";
import { RpcError } from "./error";
import type {
  EventFilter,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "./methods";
import type { Event, EventEnvelope, Job, Repo } from "./wire";

let nextCursor = 1;
let counter = 0;
const ulid = () => `01HMOCK${(counter++).toString(36).padStart(19, "0").toUpperCase()}`;

const REPO_FIXTURES: Repo[] = [
  {
    id: ulid(),
    name: "codeless-workspace",
    clone_url: "https://github.com/NubeDev/codeless-workspace",
    default_branch: "master",
    local_path: "/home/user/code/rust/codeless-workspace",
    git_auth: { kind: "ssh", key_path: "~/.ssh/id_ed25519" },
    concurrency_cap: 2,
    default_runner: "claude",
    created_at: Date.now(),
    updated_at: Date.now(),
  },
  {
    id: ulid(),
    name: "codeless",
    clone_url: "https://github.com/NubeDev/codeless",
    default_branch: "master",
    local_path: "/home/user/code/rust/codeless-workspace/codeless",
    git_auth: { kind: "ssh", key_path: "~/.ssh/id_ed25519" },
    concurrency_cap: null,
    default_runner: "claude",
    created_at: Date.now(),
    updated_at: Date.now(),
  },
];

export class MockRpcClient implements RpcClient {
  private repos: Repo[] = [...REPO_FIXTURES];
  private jobs: Job[] = [];
  private subscribers = new Set<(e: EventEnvelope) => void>();

  async call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    await sleep(80);

    switch (method) {
      case "list_repos":
        return { repos: this.repos } as RpcResultOf<M>;

      case "add_repo": {
        const a = args as RpcArgs<"add_repo">;
        const repo: Repo = {
          id: ulid(),
          name: a.name,
          clone_url: a.clone_url,
          default_branch: a.default_branch,
          local_path: a.local_path,
          git_auth: a.git_auth,
          concurrency_cap: a.concurrency_cap,
          default_runner: a.default_runner,
          created_at: Date.now(),
          updated_at: Date.now(),
        };
        this.repos.push(repo);
        this.emit({ type: "repo-added", repo_id: repo.id });
        return repo as RpcResultOf<M>;
      }

      case "remove_repo": {
        const a = args as RpcArgs<"remove_repo">;
        const before = this.repos.length;
        this.repos = this.repos.filter((r) => r.id !== a.repo_id);
        if (this.repos.length === before) {
          throw new RpcError("not_found", `repo ${a.repo_id}`);
        }
        this.emit({ type: "repo-removed", repo_id: a.repo_id });
        return null as RpcResultOf<M>;
      }

      case "list_jobs": {
        const a = args as RpcArgs<"list_jobs">;
        const jobs = a.repo_id
          ? this.jobs.filter((j) => j.repo_id === a.repo_id)
          : this.jobs;
        return { jobs } as RpcResultOf<M>;
      }

      case "get_job": {
        const a = args as RpcArgs<"get_job">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        return job as RpcResultOf<M>;
      }

      case "submit_job": {
        const a = args as RpcArgs<"submit_job">;
        if (!this.repos.some((r) => r.id === a.repo_id)) {
          throw new RpcError("not_found", `repo ${a.repo_id}`);
        }
        const now = Date.now();
        const job: Job = {
          id: ulid(),
          repo_id: a.repo_id,
          status: "queued",
          stop_reason: null,
          template_yaml: a.template_yaml,
          prompt: a.prompt,
          runner: a.runner,
          branch: a.branch,
          worktree_path: null,
          cost_cap_cents: a.cost_cap_cents,
          wall_clock_cap_ms: a.wall_clock_cap_ms,
          cost_cents: 0,
          started_at: null,
          ended_at: null,
          created_at: now,
        };
        this.jobs.push(job);
        this.emit({ type: "job-queued", job_id: job.id, repo_id: job.repo_id });
        this.synthesiseLifecycle(job);
        return job as RpcResultOf<M>;
      }

      case "stop_job": {
        const a = args as RpcArgs<"stop_job">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        if (
          job.status === "completed" ||
          job.status === "failed" ||
          job.status === "stopped"
        ) {
          throw new RpcError("conflict", `job ${a.job_id} already terminal`);
        }
        job.status = "stopped";
        job.stop_reason = "user";
        job.ended_at = Date.now();
        this.emit({ type: "job-stopped", job_id: job.id, reason: "user" });
        return null as RpcResultOf<M>;
      }

      default:
        throw new RpcError("internal", `mock: unhandled method ${method}`);
    }
  }

  subscribe(filter: EventFilter, _since?: Since): AsyncIterable<EventEnvelope> {
    const matches = (env: EventEnvelope) => {
      if (filter.scope === "all") return true;
      return env.job_id === filter.job_id;
    };
    return makeIterable((push, _close) => {
      const handler = (env: EventEnvelope) => {
        if (matches(env)) push(env);
      };
      this.subscribers.add(handler);
      return () => this.subscribers.delete(handler);
    });
  }

  private advance(job: Job, status: Job["status"]) {
    job.status = status;
    if (status === "running") {
      job.started_at = Date.now();
      this.emit({ type: "job-started", job_id: job.id }, job.id);
    } else if (status === "completed") {
      job.ended_at = Date.now();
      this.emit({ type: "job-completed", job_id: job.id }, job.id);
    }
  }

  // Synthesise a happy-path lifecycle: queue → run → two stages each
  // with one task → complete. Timings are short so the mock stays
  // snappy; structure mirrors what a real `codeless-runtime` emits so
  // timeline UI can be developed against the same shape.
  private synthesiseLifecycle(job: Job) {
    setTimeout(() => this.advance(job, "running"), 150);

    const stages = ["plan", "implement"] as const;
    let t = 250;

    for (const name of stages) {
      const stageId = ulid();
      const taskId = ulid();
      schedule(t, () =>
        this.emit(
          { type: "stage-started", stage_id: stageId, job_id: job.id },
          job.id,
          stageId,
        ),
      );
      schedule(t + 50, () =>
        this.emit(
          {
            type: "task-enqueued",
            task_id: taskId,
            stage_id: stageId,
            depends_on: [],
          },
          job.id,
          stageId,
          taskId,
        ),
      );
      schedule(t + 100, () =>
        this.emit({ type: "task-started", task_id: taskId }, job.id, stageId, taskId),
      );
      // Stream a few AI tokens so the timeline shows live activity.
      for (let i = 0; i < 4; i++) {
        schedule(t + 200 + i * 80, () =>
          this.emit(
            {
              type: "ai-token",
              task_id: taskId,
              delta: name === "plan" ? "plan… " : "edit… ",
            },
            job.id,
            stageId,
            taskId,
          ),
        );
      }
      schedule(t + 600, () =>
        this.emit(
          {
            type: "ai-message-complete",
            task_id: taskId,
            input_tokens: 320,
            output_tokens: 180,
            cost_cents: 4,
          },
          job.id,
          stageId,
          taskId,
        ),
      );
      schedule(t + 700, () =>
        this.emit(
          { type: "task-completed", task_id: taskId, status: "completed" },
          job.id,
          stageId,
          taskId,
        ),
      );
      schedule(t + 800, () =>
        this.emit(
          { type: "stage-completed", stage_id: stageId, status: "passed" },
          job.id,
          stageId,
        ),
      );
      t += 1000;
    }

    setTimeout(() => this.advance(job, "completed"), t + 100);
  }

  // Override scope ids when the event variant doesn't carry them
  // explicitly — `stage-completed` etc. omit `job_id` even though the
  // envelope still needs to be tagged with one for filter matching.
  private emit(
    event: Event,
    jobId?: string | null,
    stageId?: string | null,
    taskId?: string | null,
  ) {
    const env: EventEnvelope = {
      cursor: nextCursor++,
      job_id: jobId ?? jobIdOf(event),
      stage_id: stageId ?? stageIdOf(event),
      task_id: taskId ?? taskIdOf(event),
      created_at: Date.now(),
      event,
    };
    for (const s of this.subscribers) s(env);
  }
}

function schedule(ms: number, fn: () => void) {
  setTimeout(fn, ms);
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

function jobIdOf(e: Event): string | null {
  return "job_id" in e ? e.job_id : null;
}
function stageIdOf(e: Event): string | null {
  return "stage_id" in e ? e.stage_id : null;
}
function taskIdOf(e: Event): string | null {
  return "task_id" in e ? e.task_id : null;
}

function makeIterable(
  start: (
    push: (env: EventEnvelope) => void,
    close: () => void,
  ) => () => void,
): AsyncIterable<EventEnvelope> {
  return {
    [Symbol.asyncIterator]() {
      const queue: EventEnvelope[] = [];
      const waiters: Array<(v: IteratorResult<EventEnvelope>) => void> = [];
      let done = false;

      const push = (env: EventEnvelope) => {
        if (done) return;
        const w = waiters.shift();
        if (w) w({ value: env, done: false });
        else queue.push(env);
      };
      const close = () => {
        done = true;
        while (waiters.length)
          waiters.shift()!({ value: undefined, done: true });
      };
      const cleanup = start(push, close);

      return {
        async next() {
          if (queue.length) return { value: queue.shift()!, done: false };
          if (done) return { value: undefined, done: true };
          return new Promise<IteratorResult<EventEnvelope>>((resolve) =>
            waiters.push(resolve),
          );
        },
        async return() {
          cleanup();
          close();
          return { value: undefined, done: true };
        },
      };
    },
  };
}
