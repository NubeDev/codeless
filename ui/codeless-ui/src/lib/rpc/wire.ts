// Wire types — mirrored from the Rust source of truth in
// `codeless/crates/codeless-types`. The canonical generated copy lives
// at `codeless/crates/codeless-types/tests/wire.ts.snap` (produced by
// `specta` at build time). This file is a hand-mirrored subset until
// the Phase 1 codegen step is wired into the UI build to drop the
// `.ts` output here directly. Do not hand-edit shapes — change the
// Rust types and let codegen update this.

export type CostCents = number;
export type EventCursor = number;
export type UnixMillis = number;

export type RepoId = string;
export type JobId = string;
export type StageId = string;
export type TaskId = string;
export type ReviewId = string;

export type GitAuth =
  | { kind: "ssh"; key_path: string }
  | { kind: "token"; env_var: string }
  | { kind: "github_app"; app_id: string; installation_id: string };

export type JobStatus =
  | "queued"
  | "running"
  | "awaiting-review"
  | "completed"
  | "failed"
  | "stopped";

export type StageStatus =
  | "pending"
  | "running"
  | "awaiting-review"
  | "passed"
  | "failed";

export type TaskStatus =
  | "enqueued"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

export type ReviewStatus =
  | "pending"
  | "approved"
  | "rejected"
  | "stopped"
  | "rerun-requested";

export type StopReason = "user" | "cost-cap" | "wall-clock" | "runner-crash";

export interface Repo {
  id: RepoId;
  name: string;
  clone_url: string;
  default_branch: string;
  local_path: string;
  git_auth: GitAuth;
  concurrency_cap: number | null;
  default_runner: string | null;
  created_at: UnixMillis;
  updated_at: UnixMillis;
}

export interface Job {
  id: JobId;
  repo_id: RepoId;
  status: JobStatus;
  stop_reason: StopReason | null;
  template_yaml: string | null;
  prompt: string | null;
  runner: string;
  branch: string;
  worktree_path: string | null;
  cost_cap_cents: CostCents;
  wall_clock_cap_ms: number;
  cost_cents: CostCents;
  started_at: UnixMillis | null;
  ended_at: UnixMillis | null;
  created_at: UnixMillis;
}

export interface Stage {
  id: StageId;
  job_id: JobId;
  ordinal: number;
  name: string;
  status: StageStatus;
  verify_cmd: string | null;
  started_at: UnixMillis | null;
  ended_at: UnixMillis | null;
}

export interface Task {
  id: TaskId;
  stage_id: StageId;
  ordinal: number;
  status: TaskStatus;
  depends_on: TaskId[];
  lease_holder: string | null;
  lease_expires_at: UnixMillis | null;
  cost_cents: CostCents;
  input_tokens: number;
  output_tokens: number;
  started_at: UnixMillis | null;
  ended_at: UnixMillis | null;
}

export interface Review {
  id: ReviewId;
  stage_id: StageId;
  status: ReviewStatus;
  comment: string | null;
  requested_at: UnixMillis;
  resolved_at: UnixMillis | null;
}

export type Event =
  | { type: "repo-added"; repo_id: RepoId }
  | { type: "repo-removed"; repo_id: RepoId }
  | { type: "repo-updated"; repo_id: RepoId }
  | { type: "job-queued"; job_id: JobId; repo_id: RepoId }
  | { type: "job-promoted"; job_id: JobId }
  | { type: "job-started"; job_id: JobId }
  | { type: "job-completed"; job_id: JobId }
  | { type: "job-stopped"; job_id: JobId; reason: StopReason }
  | { type: "job-failed"; job_id: JobId }
  | { type: "stage-started"; stage_id: StageId; job_id: JobId }
  | { type: "verify-started"; stage_id: StageId }
  | { type: "verify-passed"; stage_id: StageId }
  | { type: "verify-failed"; stage_id: StageId; exit_code: number }
  | { type: "stage-completed"; stage_id: StageId; status: StageStatus }
  | {
      type: "task-enqueued";
      task_id: TaskId;
      stage_id: StageId;
      depends_on: TaskId[];
    }
  | { type: "task-started"; task_id: TaskId }
  | { type: "tool-call"; task_id: TaskId; tool: string; args_json: string }
  | {
      type: "tool-approval-requested";
      task_id: TaskId;
      tool: string;
      args_json: string;
    }
  | { type: "ai-token"; task_id: TaskId; delta: string }
  | {
      type: "ai-message-complete";
      task_id: TaskId;
      input_tokens: number;
      output_tokens: number;
      cost_cents: CostCents;
    }
  | { type: "task-completed"; task_id: TaskId; status: TaskStatus }
  | { type: "review-requested"; review_id: ReviewId; stage_id: StageId }
  | { type: "review-approved"; review_id: ReviewId }
  | { type: "review-commented"; review_id: ReviewId; comment: string }
  | { type: "review-stopped"; review_id: ReviewId };

export interface EventEnvelope {
  cursor: EventCursor;
  job_id: JobId | null;
  stage_id: StageId | null;
  task_id: TaskId | null;
  created_at: UnixMillis;
  event: Event;
}
