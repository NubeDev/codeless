// In-memory `RpcClient` for development and tests. Hold canned repo
// and job rows; `submit_job` appends to the in-memory store and
// synthesises a small event timeline so subscribers see the same
// lifecycle a real core would emit. No persistence, no network.

import type { RpcClient } from "./client";
import { RpcError } from "./error";
import type {
  EventFilter,
  Persona,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "./methods";
import type {
  AttachedWorkspace,
  Event,
  EventEnvelope,
  FsEntry,
  FsGlobHit,
  FsGrepHit,
  FsKind,
  FsReadResult,
  Job,
  Repo,
  Review,
  ServerInfo,
  ShellBgEntry,
  ShellCommandOutput,
} from "./wire";

const MOCK_FS_ROOT = "/home/user/mock-workspace";
const MOCK_FS_READ_LIMIT_DEFAULT = 1 << 20;

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

type FileNode = { kind: "file"; content: string; mtime: number };
type DirNode = { kind: "dir"; mtime: number };
type Node = FileNode | DirNode;

function seedFs(): Map<string, Node> {
  const now = Date.now();
  const m = new Map<string, Node>();
  const dirs = [
    "",
    "/src",
    "/src/modules",
    "/docs",
  ];
  for (const d of dirs) m.set(MOCK_FS_ROOT + d, { kind: "dir", mtime: now });
  m.set(`${MOCK_FS_ROOT}/README.md`, {
    kind: "file",
    content: "# mock-workspace\n\nIn-memory fixture served by MockRpcClient.\n",
    mtime: now,
  });
  m.set(`${MOCK_FS_ROOT}/src/index.ts`, {
    kind: "file",
    content: "export const greeting = \"hello, codeless\";\n",
    mtime: now,
  });
  m.set(`${MOCK_FS_ROOT}/docs/notes.md`, {
    kind: "file",
    content: "todo: write notes\n",
    mtime: now,
  });
  return m;
}

export class MockRpcClient implements RpcClient {
  private repos: Repo[] = [...REPO_FIXTURES];
  private jobs: Job[] = [];
  private subscribers = new Set<(e: EventEnvelope) => void>();
  private fs: Map<string, Node> = seedFs();
  private secrets: Map<string, string> = new Map();
  private shellSessions: Map<number, { cwd: string }> = new Map();
  private nextShellSession = 1;
  private shellBg: Map<number, ShellBgEntry> = new Map();
  private nextShellBg = 1;
  private reviews: Map<string, Review> = new Map();
  // Index of review_id -> job_id so list_reviews can filter by job
  // without bloating the Review wire type.
  private reviewJob: Map<string, string> = new Map();
  // Per-job in-memory file surface, mirrors the runtime's
  // `<repo>/.codeless/jobs/<template.name>/`. Map<jobId, Map<filename,content>>.
  // The mock skips the directory/template-name resolution dance —
  // jobs without a `template_yaml` still get an empty entry on first
  // touch because the UI's flow always opens the Spec pane against a
  // known `JobId`.
  private jobFiles: Map<string, Map<string, string>> = new Map();
  // Mirrors the SQLite `personas` table the runtime exposes. Seeded
  // with the same built-ins as migration 0011 so the agent-personas
  // surface renders against the mock with the same row identities as
  // a real host.
  private personas: Map<string, Persona> = seedPersonas();
  // Mirrors `attached_workspaces` on the server. Empty by default so
  // the UI's empty-state path is the default in tests; tests that
  // need a populated table call `seedAttachedWorkspaces()`.
  private attachedWorkspaces: AttachedWorkspace[] = [];
  // Per-workspace list of running job ids the next `detach_workspace`
  // call should surface as a `WorkspaceError::RunningJobs` payload.
  // Cleared when the caller commits to `stop` or `leave-running` so the
  // dialog's two-step flow can be exercised end-to-end against the
  // mock without the test having to fake the conflict by hand.
  private runningJobsByWorkspace: Map<string, string[]> = new Map();
  // Per-job scoped pause schedule. Empty by default; tests that exercise
  // the planned-pause divider seed it via `seedScheduledPausePoints`.
  private scheduledPausePoints: Map<string, RpcResultOf<"list_scheduled_pause_points">["points"]> =
    new Map();

  /** Test seam: pre-populate the per-job pause-point schedule so the
   *  Stage overview can render the planned-pause divider without
   *  having to drive a full submit_job + parser cycle through the
   *  mock. The shape mirrors the runtime: one entry per declared
   *  point, in YAML order. */
  public seedScheduledPausePoints(
    jobId: string,
    points: RpcResultOf<"list_scheduled_pause_points">["points"],
  ) {
    this.scheduledPausePoints.set(jobId, [...points]);
  }

  /** Test seam: pre-populate the in-memory attached-workspaces list so
   *  a `list_workspaces` call returns a non-empty roster without
   *  having to round-trip through `attach_workspace`. */
  public seedAttachedWorkspaces(workspaces: AttachedWorkspace[]) {
    this.attachedWorkspaces = [...workspaces];
  }

  /** Test seam: pin a set of "running" job ids onto a workspace so the
   *  next `detach_workspace { on_running_jobs: "refuse" }` call throws
   *  the structured `WorkspaceError::RunningJobs` shape the dialog
   *  parses. Subsequent calls with `"stop"` / `"leave-running"` clear
   *  the seed and detach as normal. */
  public seedRunningJobsForWorkspace(repoId: string, jobs: string[]) {
    this.runningJobsByWorkspace.set(repoId, [...jobs]);
  }
  /// Last `agent_chat` request the mock observed. Public so UI tests
  /// that exercise the composer can assert on threaded fields
  /// (`context.job_refs`, `mode`, etc.) without the mock having to
  /// stream a fake reply. `null` until a turn lands.
  public lastAgentChatArgs: RpcArgs<"agent_chat"> | null = null;

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

      case "list_scheduled_pause_points": {
        const a = args as RpcArgs<"list_scheduled_pause_points">;
        if (!this.jobs.some((j) => j.id === a.job_id)) {
          throw new RpcError("not_found", `job ${a.job_id}`);
        }
        return {
          points: this.scheduledPausePoints.get(a.job_id) ?? [],
        } as RpcResultOf<M>;
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
        // Mirror the runtime: default to Draft so the user can edit
        // the spec first; `start_immediately = true` lands the job
        // straight in Queued and the synthetic lifecycle fires.
        const startImmediately = a.start_immediately ?? false;
        const job: Job = {
          id: ulid(),
          repo_id: a.repo_id,
          status: startImmediately ? "queued" : "draft",
          stop_reason: null,
          template_yaml: a.template_yaml,
          prompt: a.prompt,
          runner: a.runner,
          branch: a.branch,
          workspace_mode: a.workspace_mode ?? "in-repo",
          worktree_path: null,
          cost_cap_cents: a.cost_cap_cents,
          wall_clock_cap_ms: a.wall_clock_cap_ms,
          cost_cents: 0,
          model: a.model ?? null,
          permission_mode: a.permission_mode ?? null,
          effort: a.effort ?? null,
          system_prompt: a.system_prompt ?? null,
          persona_id: a.persona_id ?? null,
          auto_bypass_policy: a.auto_bypass_policy ?? null,
          pending_operator_comment: null,
          started_at: null,
          ended_at: null,
          created_at: now,
        };
        this.jobs.push(job);
        this.emit({ type: "job-queued", job_id: job.id, repo_id: job.repo_id });
        if (startImmediately) {
          this.synthesiseLifecycle(job);
        }
        return job as RpcResultOf<M>;
      }

      case "start_job": {
        const a = args as RpcArgs<"start_job">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        if (job.status !== "draft") {
          throw new RpcError(
            "conflict",
            `job ${a.job_id} is ${job.status}, not draft`,
          );
        }
        job.status = "queued";
        this.emit({ type: "job-promoted", job_id: job.id });
        this.synthesiseLifecycle(job);
        return job as RpcResultOf<M>;
      }

      case "resume_job": {
        const a = args as RpcArgs<"resume_job">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        // `paused` is the live state a scoped pause point lands the
        // job in; the runtime accepts it the same as `stopped` /
        // `failed`, and the divider's Resume button calls this path.
        if (
          job.status !== "stopped" &&
          job.status !== "failed" &&
          job.status !== "paused"
        ) {
          throw new RpcError(
            "conflict",
            `job ${a.job_id} is ${job.status}; only stopped, failed, or paused jobs are resumable`,
          );
        }
        const costBump = a.additional_cost_cap_cents ?? null;
        if (costBump != null && costBump > 0) {
          job.cost_cap_cents = job.cost_cap_cents + costBump;
        }
        const wallBump = a.additional_wall_clock_cap_ms ?? null;
        if (wallBump != null && wallBump > 0) {
          job.wall_clock_cap_ms = job.wall_clock_cap_ms + wallBump;
        }
        const previousReason = job.stop_reason;
        job.status = "queued";
        job.stop_reason = null;
        job.ended_at = null;
        this.emit({
          type: "job-resumed",
          job_id: job.id,
          previous_reason: previousReason,
        });
        this.synthesiseLifecycle(job);
        return job as RpcResultOf<M>;
      }

      case "override_pre_check_and_resume": {
        const a = args as RpcArgs<"override_pre_check_and_resume">;
        const comment = (a.comment ?? "").trim();
        if (!comment) {
          throw new RpcError(
            "invalid_argument",
            "override_pre_check_and_resume requires a non-empty comment",
          );
        }
        return (await this.call("resume_job", {
          job_id: a.job_id,
          additional_cost_cap_cents: a.additional_cost_cap_cents,
          additional_wall_clock_cap_ms: a.additional_wall_clock_cap_ms,
          next_stage_comment: comment,
        })) as RpcResultOf<M>;
      }

      case "reset_job": {
        const a = args as RpcArgs<"reset_job">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        if (
          job.status !== "queued" &&
          job.status !== "failed" &&
          job.status !== "stopped"
        ) {
          throw new RpcError(
            "conflict",
            `job ${a.job_id} is ${job.status}; only queued, failed, or stopped jobs are resettable`,
          );
        }
        const previousStatus = job.status;
        job.status = "draft";
        job.stop_reason = null;
        job.ended_at = null;
        job.worktree_path = null;
        this.emit({
          type: "job-reset",
          job_id: job.id,
          previous_status: previousStatus,
        });
        return job as RpcResultOf<M>;
      }

      case "pause_job": {
        const a = args as RpcArgs<"pause_job">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        if (job.status !== "running" && job.status !== "awaiting-review") {
          throw new RpcError(
            "conflict",
            `job ${a.job_id} is ${job.status}; only running or awaiting-review jobs can be paused`,
          );
        }
        job.status = "paused";
        job.stop_reason = "user";
        job.ended_at = Date.now();
        this.emit({ type: "job-paused", job_id: job.id, reason: "user" });
        return null as RpcResultOf<M>;
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

      case "stop_active": {
        // Mirror the runtime's umbrella semantics. The browser mock
        // never spawns chat turns of its own, so the chat side is
        // always empty; the job side fires only when the row is in a
        // pre-terminal status. Idempotent — a terminal-or-paused row
        // returns ok with both fields zeroed.
        const a = args as RpcArgs<"stop_active">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        let stoppedJob = false;
        if (
          job.status === "running" ||
          job.status === "awaiting-review" ||
          job.status === "queued"
        ) {
          job.status = "stopped";
          job.stop_reason = "user";
          job.ended_at = Date.now();
          this.emit({ type: "job-stopped", job_id: job.id, reason: "user" });
          stoppedJob = true;
        }
        return {
          stopped_job: stoppedJob,
          cancelled_chat_task_ids: [],
        } as RpcResultOf<M>;
      }

      case "gc_worktrees": {
        // The browser mock has no on-disk worktrees to model; report
        // an empty sweep so the GC UI renders its "nothing to
        // reclaim" empty state instead of erroring.
        return {
          entries: [],
          total_size_bytes: 0,
          removed_count: 0,
          root: null,
        } as RpcResultOf<M>;
      }

      case "set_job_policy": {
        // Mirror the runtime contract from
        // DOCS/AUTO-BYPASS-DECISIONS.md Q5: Running and
        // AwaitingReview rows reject with Conflict — the operator
        // pauses, sets, resumes. Every other status accepts the
        // mutation in place.
        const a = args as RpcArgs<"set_job_policy">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        if (job.status === "running" || job.status === "awaiting-review") {
          throw new RpcError(
            "conflict",
            `job ${a.job_id} is ${job.status}; pause the job before changing its auto-bypass policy`,
          );
        }
        job.auto_bypass_policy = a.policy ?? null;
        return job as RpcResultOf<M>;
      }

      case "rerun_job": {
        const a = args as RpcArgs<"rerun_job">;
        const src = this.jobs.find((j) => j.id === a.source_job_id);
        if (!src) throw new RpcError("not_found", `job ${a.source_job_id}`);
        const now = Date.now();
        const job: Job = {
          id: ulid(),
          repo_id: src.repo_id,
          status: "queued",
          stop_reason: null,
          template_yaml: src.template_yaml,
          prompt: src.prompt,
          runner: src.runner,
          branch: "",
          workspace_mode: src.workspace_mode,
          worktree_path: null,
          cost_cap_cents: src.cost_cap_cents,
          wall_clock_cap_ms: src.wall_clock_cap_ms,
          cost_cents: 0,
          model: src.model,
          permission_mode: src.permission_mode,
          effort: src.effort,
          system_prompt: src.system_prompt,
          persona_id: src.persona_id,
          auto_bypass_policy: src.auto_bypass_policy,
          pending_operator_comment: null,
          started_at: null,
          ended_at: null,
          created_at: now,
        };
        this.jobs.push(job);
        this.emit({ type: "job-queued", job_id: job.id, repo_id: job.repo_id });
        this.synthesiseLifecycle(job);
        return job as RpcResultOf<M>;
      }

      case "fs_read_file": {
        const a = args as RpcArgs<"fs_read_file">;
        const node = this.fsRequireFile(a.path);
        const limit = a.byte_limit ?? MOCK_FS_READ_LIMIT_DEFAULT;
        const size = byteLength(node.content);
        if (size > limit) {
          return { kind: "toolarge", size, limit } as RpcResultOf<M>;
        }
        const r: FsReadResult = {
          kind: "text",
          content: node.content,
          encoding: "utf-8",
        };
        return r as RpcResultOf<M>;
      }

      case "fs_write_file": {
        const a = args as RpcArgs<"fs_write_file">;
        const parent = parentPath(a.path);
        if (!this.fs.has(parent)) {
          if (a.create_parents) this.fsMkdirRecursive(parent);
          else throw new RpcError("not_found", `parent ${parent}`);
        }
        const existing = this.fs.get(a.path);
        if (existing && existing.kind === "dir") {
          throw new RpcError("conflict", `${a.path} is a directory`);
        }
        this.fs.set(a.path, { kind: "file", content: a.content, mtime: Date.now() });
        return null as RpcResultOf<M>;
      }

      case "fs_create_file": {
        const a = args as RpcArgs<"fs_create_file">;
        const existing = this.fs.get(a.path);
        if (existing && !a.overwrite) {
          throw new RpcError("conflict", `${a.path} already exists`);
        }
        if (existing && existing.kind === "dir") {
          throw new RpcError("conflict", `${a.path} is a directory`);
        }
        const parent = parentPath(a.path);
        if (!this.fs.has(parent)) {
          throw new RpcError("not_found", `parent ${parent}`);
        }
        this.fs.set(a.path, {
          kind: "file",
          content: a.content ?? "",
          mtime: Date.now(),
        });
        return null as RpcResultOf<M>;
      }

      case "fs_create_dir": {
        const a = args as RpcArgs<"fs_create_dir">;
        if (this.fs.has(a.path)) {
          const n = this.fs.get(a.path)!;
          if (n.kind === "dir") return null as RpcResultOf<M>;
          throw new RpcError("conflict", `${a.path} exists and is a file`);
        }
        const parent = parentPath(a.path);
        if (!this.fs.has(parent)) {
          if (a.recursive) this.fsMkdirRecursive(parent);
          else throw new RpcError("not_found", `parent ${parent}`);
        }
        this.fs.set(a.path, { kind: "dir", mtime: Date.now() });
        return null as RpcResultOf<M>;
      }

      case "fs_read_dir": {
        const a = args as RpcArgs<"fs_read_dir">;
        const node = this.fs.get(a.path);
        if (!node) throw new RpcError("not_found", a.path);
        if (node.kind !== "dir") {
          throw new RpcError("invalid_argument", `${a.path} is not a directory`);
        }
        const entries: FsEntry[] = [];
        const prefix = a.path.endsWith("/") ? a.path : `${a.path}/`;
        for (const [p, n] of this.fs) {
          if (p === a.path) continue;
          if (!p.startsWith(prefix)) continue;
          const rest = p.slice(prefix.length);
          if (rest.includes("/")) continue;
          entries.push({
            name: rest,
            kind: n.kind as FsKind,
            size: n.kind === "file" ? byteLength(n.content) : null,
            mtime: n.mtime,
          });
        }
        entries.sort((x, y) => x.name.localeCompare(y.name));
        return { entries } as RpcResultOf<M>;
      }

      case "fs_search": {
        const a = args as RpcArgs<"fs_search">;
        if (!this.fs.has(a.root)) throw new RpcError("not_found", a.root);
        const max = a.max_results ?? 500;
        const re = new RegExp(
          a.query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
          a.case_sensitive ? "" : "i",
        );
        const globRe = a.glob ? globToRegExp(a.glob) : null;
        const prefix = a.root.endsWith("/") ? a.root : `${a.root}/`;
        const hits: FsGrepHit[] = [];
        let truncated = false;
        for (const [p, n] of this.fs) {
          if (n.kind !== "file") continue;
          if (p !== a.root && !p.startsWith(prefix)) continue;
          if (globRe && !globRe.test(p.slice(prefix.length))) continue;
          const lines = n.content.split("\n");
          for (let i = 0; i < lines.length; i++) {
            const m = re.exec(lines[i]);
            if (!m) continue;
            if (hits.length >= max) {
              truncated = true;
              break;
            }
            hits.push({
              path: p,
              line: i + 1,
              column: m.index + 1,
              preview: lines[i],
            });
          }
          if (truncated) break;
        }
        return { hits, truncated } as RpcResultOf<M>;
      }

      case "fs_glob": {
        const a = args as RpcArgs<"fs_glob">;
        if (!this.fs.has(a.root)) throw new RpcError("not_found", a.root);
        const max = a.max_results ?? 500;
        const re = globToRegExp(a.pattern);
        const prefix = a.root.endsWith("/") ? a.root : `${a.root}/`;
        const hits: FsGlobHit[] = [];
        let truncated = false;
        for (const [p, n] of this.fs) {
          if (p === a.root) continue;
          if (!p.startsWith(prefix)) continue;
          if (!re.test(p.slice(prefix.length))) continue;
          if (hits.length >= max) {
            truncated = true;
            break;
          }
          hits.push({ path: p, kind: n.kind as FsKind });
        }
        return { hits, truncated } as RpcResultOf<M>;
      }

      case "fs_move": {
        const a = args as RpcArgs<"fs_move">;
        const src = this.fs.get(a.from);
        if (!src) throw new RpcError("not_found", a.from);
        if (this.fs.has(a.to) && !a.overwrite) {
          throw new RpcError("conflict", `${a.to} already exists`);
        }
        const parent = parentPath(a.to);
        if (!this.fs.has(parent)) {
          throw new RpcError("not_found", `parent ${parent}`);
        }
        // Move subtree: rewrite every key prefixed by `from`.
        const fromPrefix = a.from.endsWith("/") ? a.from : `${a.from}/`;
        const toPrefix = a.to.endsWith("/") ? a.to : `${a.to}/`;
        const moves: Array<[string, Node]> = [];
        for (const [p, n] of this.fs) {
          if (p === a.from) moves.push([a.to, n]);
          else if (p.startsWith(fromPrefix)) moves.push([toPrefix + p.slice(fromPrefix.length), n]);
        }
        // Apply: delete originals, set destinations.
        for (const [p] of this.fs) {
          if (p === a.from || p.startsWith(fromPrefix)) this.fs.delete(p);
        }
        for (const [p, n] of moves) this.fs.set(p, { ...n, mtime: Date.now() });
        return null as RpcResultOf<M>;
      }

      case "fs_delete": {
        const a = args as RpcArgs<"fs_delete">;
        const node = this.fs.get(a.path);
        if (!node) throw new RpcError("not_found", a.path);
        if (node.kind === "dir") {
          const prefix = a.path.endsWith("/") ? a.path : `${a.path}/`;
          const hasChildren = [...this.fs.keys()].some((p) => p.startsWith(prefix));
          if (hasChildren && !a.recursive) {
            throw new RpcError("conflict", `${a.path} is non-empty`);
          }
          for (const p of [...this.fs.keys()]) {
            if (p === a.path || p.startsWith(prefix)) this.fs.delete(p);
          }
        } else {
          this.fs.delete(a.path);
        }
        return null as RpcResultOf<M>;
      }

      case "fs_cwd": {
        return { path: MOCK_FS_ROOT } as RpcResultOf<M>;
      }

      case "list_job_files": {
        const a = args as RpcArgs<"list_job_files">;
        const files = this.jobFiles.get(a.job_id);
        if (!files || files.size === 0) {
          return {
            entries: [],
            layout: "none",
            directory_path: null,
          } as RpcResultOf<M>;
        }
        const names = [...files.keys()].sort();
        const entries = names.map((name) => {
          const lower = name.toLowerCase();
          return {
            name,
            is_template: lower === "template.yaml",
            is_scope: lower === "scope.md",
            is_workflow: lower === "workflow.md",
          };
        });
        const tplIdx = entries.findIndex((e) => e.is_template);
        if (tplIdx > 0) {
          const [tpl] = entries.splice(tplIdx, 1);
          entries.unshift(tpl);
        }
        return {
          entries,
          layout: "directory",
          directory_path: `${MOCK_FS_ROOT}/.codeless/jobs/${a.job_id}`,
        } as RpcResultOf<M>;
      }

      case "read_job_file": {
        const a = args as RpcArgs<"read_job_file">;
        const filename = normaliseJobFilename(a.filename);
        if (typeof filename !== "string") {
          throw new RpcError("invalid_argument", filename.error);
        }
        const files = this.jobFiles.get(a.job_id);
        const content = files?.get(filename);
        if (content === undefined) {
          throw new RpcError("not_found", `job file ${a.job_id}/${filename}`);
        }
        return { content } as RpcResultOf<M>;
      }

      case "write_job_file": {
        const a = args as RpcArgs<"write_job_file">;
        const filename = normaliseJobFilename(a.filename);
        if (typeof filename !== "string") {
          throw new RpcError("invalid_argument", filename.error);
        }
        if (filename.toLowerCase() === "template.yaml") {
          throw new RpcError(
            "invalid_argument",
            "template.yaml is reserved; use the spec editor",
          );
        }
        let files = this.jobFiles.get(a.job_id);
        if (!files) {
          files = new Map();
          this.jobFiles.set(a.job_id, files);
        }
        files.set(filename, a.content);
        return { name: filename } as RpcResultOf<M>;
      }

      case "delete_job_file": {
        const a = args as RpcArgs<"delete_job_file">;
        const filename = normaliseJobFilename(a.filename);
        if (typeof filename !== "string") {
          throw new RpcError("invalid_argument", filename.error);
        }
        if (filename.toLowerCase() === "template.yaml") {
          throw new RpcError(
            "invalid_argument",
            "template.yaml is reserved; use the spec editor",
          );
        }
        const files = this.jobFiles.get(a.job_id);
        if (!files || !files.delete(filename)) {
          throw new RpcError("not_found", `job file ${a.job_id}/${filename}`);
        }
        return null as RpcResultOf<M>;
      }

      case "update_job_template": {
        const a = args as RpcArgs<"update_job_template">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);

        const parsed = parseTemplateYaml(a.template_yaml);
        if (!parsed.ok) {
          throw new RpcError("invalid_argument", `template parse: ${parsed.error}`);
        }
        const prevName = job.template_yaml
          ? parseTemplateYaml(job.template_yaml).name ?? parsed.name
          : parsed.name;
        if (prevName !== parsed.name) {
          throw new RpcError(
            "conflict",
            `rename refused: spec name is \`${prevName}\`, cannot become \`${parsed.name}\`. Submit a fresh job to rename.`,
          );
        }

        job.template_yaml = a.template_yaml;
        let files = this.jobFiles.get(a.job_id);
        if (!files) {
          files = new Map();
          this.jobFiles.set(a.job_id, files);
        }
        files.set("template.yaml", a.template_yaml);
        return { name: parsed.name } as RpcResultOf<M>;
      }

      case "agent_chat": {
        // Mock client cannot spawn host binaries. Returning a result
        // shape (so the caller's `await` resolves) plus an immediate
        // error chunk on the event stream keeps the UI's contract
        // honest: real hosts stream tokens, the mock declines politely.
        const a = args as RpcArgs<"agent_chat">;
        this.lastAgentChatArgs = a;
        return {
          session_id: a.session_id,
          task_id: a.session_id,
        } as RpcResultOf<M>;
      }

      case "cancel_chat_task": {
        // Idempotent on the host (a missing entry returns Ok); the
        // mock client never spawned anything to cancel, so the
        // matching no-op preserves caller semantics.
        return null as RpcResultOf<M>;
      }

      case "write_handover": {
        const a = args as RpcArgs<"write_handover">;
        const job = this.jobs.find((j) => j.id === a.job_id);
        if (!job) throw new RpcError("not_found", `job ${a.job_id}`);
        if (!job.worktree_path) {
          throw new RpcError(
            "conflict",
            `job ${a.job_id} has no worktree yet; the runner must run before a handover can be seeded`,
          );
        }
        // Handover is keyed per-stage (JOB-MODEL.md H1). The mock
        // does not model stages; substitute a fixed `mock-stage`
        // segment so the synthetic path retains the per-stage shape
        // the real runtime uses.
        const stageSeg = a.stage_id ?? "mock-stage";
        const path = `${job.worktree_path}/runs/${a.job_id}/${stageSeg}/handover.md`;
        // Stash under the synthetic file path so a subsequent
        // `fs_read_file` from HandoverPanel finds it. Same contract
        // as the real runtime.
        this.fs.set(path, {
          kind: "file",
          content: serialiseHandoverMd(a.handover),
          mtime: Date.now(),
        });
        return { path } as RpcResultOf<M>;
      }

      case "secrets_set": {
        const a = args as RpcArgs<"secrets_set">;
        this.secrets.set(a.provider, a.value);
        return null as RpcResultOf<M>;
      }

      case "secrets_get": {
        const a = args as RpcArgs<"secrets_get">;
        return (this.secrets.get(a.provider) ?? null) as RpcResultOf<M>;
      }

      case "secrets_list": {
        const entries = [...this.secrets.keys()].sort().map((provider) => ({
          provider,
        }));
        return { entries } as RpcResultOf<M>;
      }

      case "secrets_rm": {
        const a = args as RpcArgs<"secrets_rm">;
        if (!this.secrets.delete(a.provider)) {
          throw new RpcError("not_found", `secret ${a.provider}`);
        }
        return null as RpcResultOf<M>;
      }

      case "shell_run": {
        const a = args as RpcArgs<"shell_run">;
        return mockShellOutput(a.command) as RpcResultOf<M>;
      }

      case "shell_session_open": {
        const id = this.nextShellSession++;
        this.shellSessions.set(id, { cwd: (args as RpcArgs<"shell_session_open">).cwd ?? MOCK_FS_ROOT });
        return id as RpcResultOf<M>;
      }

      case "shell_session_run": {
        const a = args as RpcArgs<"shell_session_run">;
        const sess = this.shellSessions.get(a.id);
        if (!sess) throw new RpcError("not_found", `session ${a.id}`);
        if (a.cwd) sess.cwd = a.cwd;
        const base = mockShellOutput(a.command);
        return { ...base, cwd_after: sess.cwd } as RpcResultOf<M>;
      }

      case "shell_session_close": {
        const a = args as RpcArgs<"shell_session_close">;
        if (!this.shellSessions.delete(a.id)) {
          throw new RpcError("not_found", `session ${a.id}`);
        }
        return null as RpcResultOf<M>;
      }

      case "shell_bg_spawn": {
        const a = args as RpcArgs<"shell_bg_spawn">;
        const handle = this.nextShellBg++;
        this.shellBg.set(handle, {
          handle,
          command: a.command,
          cwd: a.cwd,
          started_at_ms: Date.now(),
          exited: false,
          exit_code: null,
        });
        return handle as RpcResultOf<M>;
      }

      case "shell_bg_logs": {
        const a = args as RpcArgs<"shell_bg_logs">;
        if (!this.shellBg.has(a.handle)) {
          throw new RpcError("not_found", `bg ${a.handle}`);
        }
        return {
          bytes: "",
          next_offset: a.since_offset ?? 0,
          dropped: 0,
          exited: false,
          exit_code: null,
        } as RpcResultOf<M>;
      }

      case "shell_bg_kill": {
        const a = args as RpcArgs<"shell_bg_kill">;
        const entry = this.shellBg.get(a.handle);
        if (!entry) throw new RpcError("not_found", `bg ${a.handle}`);
        entry.exited = true;
        entry.exit_code = 137;
        return null as RpcResultOf<M>;
      }

      case "shell_bg_list": {
        return { entries: [...this.shellBg.values()] } as RpcResultOf<M>;
      }

      case "list_reviews": {
        const a = args as RpcArgs<"list_reviews">;
        const out: Review[] = [];
        for (const r of this.reviews.values()) {
          if (a.stage_id && r.stage_id !== a.stage_id) continue;
          if (a.job_id) {
            const job = this.reviewJob.get(r.id);
            if (job !== a.job_id) continue;
          }
          if (a.status && r.status !== a.status) continue;
          out.push(r);
        }
        return { reviews: out } as RpcResultOf<M>;
      }

      case "approve_review": {
        const a = args as RpcArgs<"approve_review">;
        const r = this.reviews.get(a.review_id);
        if (!r) throw new RpcError("not_found", `review ${a.review_id}`);
        if (r.status !== "pending") {
          throw new RpcError("conflict", `review ${a.review_id} is ${r.status}`);
        }
        r.status = "approved";
        r.resolved_at = Date.now();
        this.emit({ type: "review-approved", review_id: r.id });
        return r as RpcResultOf<M>;
      }

      case "comment_review": {
        const a = args as RpcArgs<"comment_review">;
        const r = this.reviews.get(a.review_id);
        if (!r) throw new RpcError("not_found", `review ${a.review_id}`);
        r.comment = a.comment;
        this.emit({
          type: "review-commented",
          review_id: r.id,
          comment: a.comment,
        });
        return r as RpcResultOf<M>;
      }

      case "stop_review": {
        const a = args as RpcArgs<"stop_review">;
        const r = this.reviews.get(a.review_id);
        if (!r) throw new RpcError("not_found", `review ${a.review_id}`);
        if (r.status !== "pending") {
          throw new RpcError("conflict", `review ${a.review_id} is ${r.status}`);
        }
        r.status = "stopped";
        r.resolved_at = Date.now();
        this.emit({ type: "review-stopped", review_id: r.id });
        return r as RpcResultOf<M>;
      }

      case "list_personas": {
        // Same ORDER BY as the runtime: built-ins first (by id), then
        // user rows by created_at ascending. The mock keeps the rail
        // shape identical so the agent-personas UI render does not
        // need a real host to look right.
        const all = [...this.personas.values()];
        const builtins = all
          .filter((p) => p.built_in)
          .sort((a, b) => a.id.localeCompare(b.id));
        const userRows = all
          .filter((p) => !p.built_in)
          .sort((a, b) => a.created_at - b.created_at);
        return { personas: [...builtins, ...userRows] } as RpcResultOf<M>;
      }

      case "get_persona": {
        const a = args as RpcArgs<"get_persona">;
        const p = this.personas.get(a.id);
        if (!p) throw new RpcError("not_found", `persona ${a.id}`);
        return p as RpcResultOf<M>;
      }

      case "upsert_persona": {
        const a = args as RpcArgs<"upsert_persona">;
        if (!a.id.trim()) {
          throw new RpcError("invalid_argument", "persona id is empty");
        }
        if (!a.name.trim()) {
          throw new RpcError("invalid_argument", "persona name is empty");
        }
        if (!a.instructions.trim()) {
          throw new RpcError(
            "invalid_argument",
            "persona instructions are empty",
          );
        }
        const now = Date.now();
        const prev = this.personas.get(a.id);
        const next: Persona = {
          id: a.id,
          name: a.name,
          description: a.description,
          icon: a.icon,
          instructions: a.instructions,
          use_for_jobs: a.use_for_jobs,
          default_model: a.default_model ?? null,
          allowed_subagents: a.allowed_subagents,
          default_snippets: a.default_snippets ?? [],
          // `built_in` is preserved across upsert so a user-edited
          // built-in stays a built-in (and remains undeletable).
          built_in: prev?.built_in ?? false,
          created_at: prev?.created_at ?? now,
          updated_at: now,
        };
        this.personas.set(a.id, next);
        return next as RpcResultOf<M>;
      }

      case "approve_scope_patch": {
        // The mock does not maintain a real proposal queue or git
        // history; it returns a synthesised commit sha so the inbox's
        // toast / row-folding paths can be exercised end-to-end in
        // tests and storybook fixtures. Real resolution is observable
        // only against a live runtime; the mock is just a wire shim.
        const sha = mockCommitSha();
        return { outcome: "approved", commit_sha: sha } as RpcResultOf<M>;
      }

      case "reject_scope_patch": {
        const sha = mockCommitSha();
        return { outcome: "rejected", commit_sha: sha } as RpcResultOf<M>;
      }

      case "edit_scope_patch": {
        return { outcome: "edited" } as RpcResultOf<M>;
      }

      case "revert_scope_patch": {
        return { commit_sha: mockCommitSha() } as RpcResultOf<M>;
      }

      case "list_proposed_patches": {
        // The mock has no on-disk queue file; Surface C tests inject
        // fixtures by overriding the client's `call` method directly.
        // Returning an empty list here keeps the default fixture
        // honest (no proposals across any repo) so a test that forgets
        // to override sees an empty worklist rather than mock data.
        return { entries: [] } as RpcResultOf<M>;
      }

      case "list_workspaces":
        return { workspaces: [...this.attachedWorkspaces] } as RpcResultOf<M>;

      case "attach_workspace": {
        const a = args as RpcArgs<"attach_workspace">;
        const repo = this.repos.find((r) => r.id === a.repo_id);
        if (!repo) throw new RpcError("not_found", `repo ${a.repo_id}`);
        const fs_root = a.fs_root_override ?? repo.local_path;
        const existing = this.attachedWorkspaces.find(
          (w) => w.repo_id === a.repo_id,
        );
        if (existing) {
          throw new RpcError("conflict", `repo ${a.repo_id} already attached`);
        }
        const workspace = {
          repo_id: repo.id,
          repo_name: repo.name,
          fs_root,
          attached_at: Date.now(),
          default_runner: repo.default_runner ?? null,
        };
        this.attachedWorkspaces.push(workspace);
        return { workspace } as RpcResultOf<M>;
      }

      case "detach_workspace": {
        const a = args as RpcArgs<"detach_workspace">;
        const running = this.runningJobsByWorkspace.get(a.repo_id);
        if (running && running.length > 0) {
          if (a.on_running_jobs === "refuse") {
            // Mirror the wire shape of `WorkspaceError::RunningJobs`
            // closely enough that the dialog's parser recognises the
            // variant tag and extracts the job list.
            throw new RpcError(
              "conflict",
              JSON.stringify({ "running-jobs": { jobs: running } }),
            );
          }
          // `stop` / `leave-running` both consume the seed: the server
          // would have either cancelled or detached around them.
          this.runningJobsByWorkspace.delete(a.repo_id);
        }
        const before = this.attachedWorkspaces.length;
        this.attachedWorkspaces = this.attachedWorkspaces.filter(
          (w) => w.repo_id !== a.repo_id,
        );
        if (this.attachedWorkspaces.length === before) {
          throw new RpcError("not_found", `workspace ${a.repo_id}`);
        }
        return null as RpcResultOf<M>;
      }

      case "validate_workspace_path": {
        const a = args as RpcArgs<"validate_workspace_path">;
        const already_attached = this.attachedWorkspaces.some(
          (w) => w.fs_root === a.path,
        );
        return {
          canonical: a.path,
          is_dir: true,
          is_git_repo: true,
          default_branch: "main",
          already_attached,
          readable: true,
          writable: true,
          problems: [],
        } as RpcResultOf<M>;
      }

      case "delete_persona": {
        const a = args as RpcArgs<"delete_persona">;
        const existing = this.personas.get(a.id);
        if (!existing) {
          throw new RpcError("not_found", `persona ${a.id}`);
        }
        if (existing.built_in) {
          throw new RpcError(
            "conflict",
            `persona ${a.id} is built-in and cannot be deleted`,
          );
        }
        this.personas.delete(a.id);
        return null as RpcResultOf<M>;
      }

      case "list_plugins":
        // The mock fixture has no plugins registered. Host wiring
        // exercises the boot path and every `<PluginSlot/>` renders
        // its fallback. Once a fixture plugin gets seeded the result
        // will mirror the server projection.
        return { plugins: [] } as RpcResultOf<M>;

      default:
        throw new RpcError("internal", `mock: unhandled method ${method}`);
    }
  }

  private fsRequireFile(path: string): FileNode {
    const node = this.fs.get(path);
    if (!node) throw new RpcError("not_found", path);
    if (node.kind !== "file") {
      throw new RpcError("invalid_argument", `${path} is not a file`);
    }
    return node;
  }

  private fsMkdirRecursive(path: string) {
    if (this.fs.has(path)) return;
    if (path === "" || path === "/") return;
    if (!path.startsWith(MOCK_FS_ROOT)) {
      throw new RpcError("invalid_argument", `${path} outside mock root`);
    }
    this.fsMkdirRecursive(parentPath(path));
    this.fs.set(path, { kind: "dir", mtime: Date.now() });
  }

  // Mock pretends a `--enable-claude` server discovered no binary so
// the UI can render the "install Claude Code" hint without a real
// host probe. `mock` stays the default because `claude` here is a
// non-functional placeholder; flipping it would mislead users into
// submitting jobs the mock cannot run.
  async serverInfo(): Promise<ServerInfo> {
    return {
      version: "mock",
      runners: [
        { id: "mock", default: true },
        { id: "claude", default: false },
      ],
      fs_root: MOCK_FS_ROOT,
      worktree_root: null,
      claude: null,
      // Mock client is a pure in-browser fixture; no host CLI binaries
      // are reachable, so the footer agent's CLI dropdown stays empty
      // under the mock client just as it would on a real host without
      // any CLI installed.
      available_cli_runners: [],
    };
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
      // After the plan stage, register a pending review so the
      // review-approval surfaces have something to drive against
      // /?mock=1. Real runtime gates the stage transition on the
      // review; the mock leaves the synthesised lifecycle moving so
      // the dashboard still animates without a hand-approve.
      if (name === "plan") {
        const reviewId = ulid();
        schedule(t + 850, () => {
          const review: Review = {
            id: reviewId,
            stage_id: stageId,
            status: "pending",
            comment: null,
            requested_at: Date.now(),
            resolved_at: null,
          };
          this.reviews.set(reviewId, review);
          this.reviewJob.set(reviewId, job.id);
          this.emit(
            { type: "review-requested", review_id: reviewId, stage_id: stageId },
            job.id,
            stageId,
          );
        });
      }
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

// Built-in persona ids mirror migration 0011's seed. Instructions are
// not duplicated here — the UI's fallback defaults live in
// `modules/ai/lib/agents.ts`. The mock returns abbreviated text so a
// test that asserts on field shape stays small; full prompts come
// from the real runtime.
const BUILTIN_PERSONA_IDS = [
  "builtin:architect",
  "builtin:coder",
  "builtin:designer",
  "builtin:reviewer",
  "builtin:security",
] as const;

const ALL_MOCK_SUBAGENTS = ["explore", "code-review", "security", "general"];

function seedPersonas(): Map<string, Persona> {
  const m = new Map<string, Persona>();
  for (const id of BUILTIN_PERSONA_IDS) {
    const slug = id.slice("builtin:".length);
    m.set(id, {
      id,
      name: slug.replace(/^./, (c) => c.toUpperCase()),
      description: `${slug} built-in persona`,
      icon: slug,
      instructions: `mock instructions for ${slug}`,
      use_for_jobs: false,
      default_model: null,
      allowed_subagents: [...ALL_MOCK_SUBAGENTS],
      default_snippets: [],
      built_in: true,
      created_at: 0,
      updated_at: 0,
    });
  }
  return m;
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
function mockCommitSha(): string {
  // Random 40-hex digit string. The inbox's revert/link rendering
  // treats it as opaque; the mock only needs the shape to be
  // recognisable as a SHA.
  const chars = "0123456789abcdef";
  let out = "";
  for (let i = 0; i < 40; i += 1) {
    out += chars[Math.floor(Math.random() * chars.length)];
  }
  return out;
}

function stageIdOf(e: Event): string | null {
  return "stage_id" in e ? e.stage_id : null;
}
function taskIdOf(e: Event): string | null {
  return "task_id" in e ? e.task_id : null;
}

// Stub shell output. The mock does not actually run anything; the
// real runner is gated by SCOPE.md R1 (process spawn lives in
// codeless-adapters-host). Tests/dev UIs that need a non-trivial
// echo can replace this. Returning exit 0 with a short stdout keeps
// happy-path UI flows alive against /?mock=1.
function mockShellOutput(command: string): ShellCommandOutput {
  return {
    stdout: `[mock] ${command}\n`,
    stderr: "",
    exit_code: 0,
    timed_out: false,
    truncated: false,
  };
}

function parentPath(p: string): string {
  const i = p.lastIndexOf("/");
  if (i <= 0) return "/";
  return p.slice(0, i);
}

function byteLength(s: string): number {
  return new TextEncoder().encode(s).length;
}

// Translate a shell-style glob (`*`, `?`, `**`) into a regexp anchored
// to the full relative path. Intentionally minimal — the real adapter
// will use `globset`; the mock just needs enough to drive UI flows.
function globToRegExp(glob: string): RegExp {
  let re = "^";
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*") {
      if (glob[i + 1] === "*") {
        re += ".*";
        i++;
      } else {
        re += "[^/]*";
      }
    } else if (c === "?") {
      re += "[^/]";
    } else if ("\\^$.|+()[]{}".includes(c)) {
      re += "\\" + c;
    } else {
      re += c;
    }
  }
  return new RegExp(re + "$");
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

// Mirror of `codeless_runtime::job_dir::sanitise_filename`. Returns
// the normalised filename on success, or `{ error }` for the wire-side
// `InvalidArgument` reason. The mock keeps parity so the UI sees the
// same rejection messages whether it talks to the in-memory mock or
// the real Rust runtime.
function normaliseJobFilename(raw: string): string | { error: string } {
  const trimmed = raw.trim();
  if (!trimmed) return { error: "filename is empty" };
  if (trimmed.includes("/") || trimmed.includes("\\")) {
    return { error: "filename contains path traversal" };
  }
  if (trimmed.split(".").some((s) => s === "..") || trimmed === "..") {
    return { error: "filename contains path traversal" };
  }
  if (trimmed.startsWith(".")) return { error: "dotfiles are not allowed" };
  const lower = trimmed.toLowerCase();
  if (
    lower.endsWith(".md") ||
    lower.endsWith(".yaml") ||
    lower.endsWith(".yml") ||
    trimmed.includes(".")
  ) {
    return trimmed;
  }
  return `${trimmed}.md`;
}

// Minimal mirror of `codeless_runtime::template::JobTemplate::parse_yaml`.
// Validates the three load-bearing fields (name, goal, non-empty
// stages). We avoid pulling in a real YAML parser — the spec is
// authored as a list-editor on the UI side, so the surface the mock
// has to handle is small and stable.
type ParsedTemplate =
  | { ok: true; name: string }
  | { ok: false; error: string; name?: undefined };

function parseTemplateYaml(yaml: string): ParsedTemplate {
  const nameMatch = /^\s*name\s*:\s*(.+?)\s*$/m.exec(yaml);
  if (!nameMatch || !nameMatch[1].trim()) {
    return { ok: false, error: "name is empty" };
  }
  const goalMatch = /^\s*goal\s*:\s*(.+?)\s*$/m.exec(yaml);
  if (!goalMatch || !goalMatch[1].trim()) {
    return { ok: false, error: "goal is empty" };
  }
  const stagesIdx = yaml.search(/^\s*stages\s*:\s*$/m);
  if (stagesIdx < 0) return { ok: false, error: "stages is empty" };
  const after = yaml.slice(stagesIdx).split("\n").slice(1);
  let count = 0;
  for (const raw of after) {
    if (/^\s*-\s+/.test(raw)) count++;
    else if (raw.trim() === "") continue;
    else if (/^\S/.test(raw)) break;
  }
  if (count === 0) return { ok: false, error: "stages is empty" };
  return { ok: true, name: nameMatch[1].trim() };
}

// Mirror of `codeless_types::Handover::to_markdown`. Same four
// sections, same `(none)` placeholder for empty bullet lists, so a
// downstream parser still finds all four headings.
function serialiseHandoverMd(h: {
  done: string[];
  next: string[];
  what_you_need_to_know: string[];
  open_questions: string[];
}): string {
  const section = (title: string, items: string[]): string => {
    const lines = items.length === 0 ? ["- (none)"] : items.map((i) => `- ${i}`);
    return `## ${title}\n\n${lines.join("\n")}\n\n`;
  };
  return (
    section("Done", h.done) +
    section("Next", h.next) +
    section("What you need to know", h.what_you_need_to_know) +
    section("Open questions", h.open_questions)
  );
}
