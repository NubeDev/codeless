import type { EventEnvelope } from "@/lib/rpc";

// Helpers shared by the timeline (per-event row) and the dashboard
// (per-job last-event chip). One concept: turn an event envelope into
// the shortest readable string a user can scan. Lives outside any one
// component because both surfaces render the same vocabulary.

export function truncate(s: string, max: number): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

// Strip the worktree prefix from a path so callers read
// `Read(src/main.rs)` rather than the full
// `/tmp/demo-target/.codeless/worktrees/job-<id>/src/main.rs`. The
// boundary marker is the `.codeless/worktrees/job-<id>/` segment —
// after that the rest is the repo-relative path the user thinks in.
export function relativise(absolute: string): string {
  const m = absolute.match(/\.codeless\/worktrees\/job-[^/]+\/(.*)$/);
  return m ? m[1] : absolute;
}

// Render a tool-call as a single-line summary like `Bash(git status)`,
// `Read(src/main.rs)`. Falls back to truncated JSON for tools without
// a registered shaper, and to the raw JSON for non-object args. The
// component is intentionally loose with types: args_json is whatever
// the upstream emitted, and if a key isn't where we expect it we
// drop to the JSON fallback rather than throw.
export function summariseToolArgs(tool: string, argsJson: string): string {
  if (!argsJson) return "";
  let parsed: unknown;
  try {
    parsed = JSON.parse(argsJson);
  } catch {
    return truncate(argsJson, 80);
  }
  if (!parsed || typeof parsed !== "object") return truncate(argsJson, 80);
  const args = parsed as Record<string, unknown>;
  const pick = (key: string): string | null =>
    typeof args[key] === "string" ? (args[key] as string) : null;

  switch (tool) {
    case "Bash": {
      const cmd = pick("command");
      return cmd ? truncate(cmd, 80) : truncate(argsJson, 80);
    }
    case "Read":
    case "Write":
    case "Edit":
    case "MultiEdit":
    case "NotebookEdit": {
      const path = pick("file_path") ?? pick("path") ?? pick("notebook_path");
      return path ? relativise(path) : truncate(argsJson, 80);
    }
    case "Glob": {
      const pattern = pick("pattern");
      return pattern ?? truncate(argsJson, 80);
    }
    case "Grep": {
      const pattern = pick("pattern");
      const path = pick("path");
      if (pattern && path) return `${pattern} in ${relativise(path)}`;
      return pattern ?? truncate(argsJson, 80);
    }
    case "AskUserQuestion": {
      const q = pick("question");
      return q ? `"${truncate(q, 60)}"` : truncate(argsJson, 80);
    }
    case "TodoWrite": {
      const todos = args.todos;
      if (Array.isArray(todos))
        return `${todos.length} item${todos.length === 1 ? "" : "s"}`;
      return truncate(argsJson, 80);
    }
    default:
      return truncate(argsJson, 80);
  }
}

// One-line, dashboard-friendly description of an envelope. Returns
// null for envelopes the dashboard chooses to ignore (e.g. raw
// ai-token deltas which would otherwise flicker the chip on every
// stream chunk). The caller decides what to do with null — typically
// "keep the previous summary".
export function summariseEnvelope(env: EventEnvelope): string | null {
  const e = env.event;
  switch (e.type) {
    case "tool-call":
    case "tool-approval-requested":
      return `${e.tool}(${summariseToolArgs(e.tool, e.args_json)})`;
    case "ai-token":
      return null;
    case "ai-message-complete":
      return `assistant · $${(e.cost_cents / 100).toFixed(2)}`;
    case "job-queued":
      return "queued";
    case "job-started":
      return "started";
    case "job-promoted":
      return "promoted";
    case "job-completed":
      return "completed";
    case "job-failed":
      return "failed";
    case "job-stopped":
      return `stopped (${e.reason})`;
    case "stage-started":
      return "stage started";
    case "stage-completed":
      return `stage ${e.status}`;
    case "verify-started":
      return "verify started";
    case "verify-passed":
      return "verify passed";
    case "verify-failed":
      return `verify failed (exit ${e.exit_code})`;
    case "task-enqueued":
      return "task enqueued";
    case "task-started":
      return "task started";
    case "task-completed":
      return `task ${e.status}`;
    case "review-requested":
      return "review requested";
    case "review-approved":
      return "review approved";
    case "review-stopped":
      return "review stopped";
    case "review-commented":
      return "review commented";
    default:
      return e.type;
  }
}

// Relative time like "3m ago", "2h ago", "just now". Caller supplies
// `now` (Date.now() at render time) so the function stays pure and
// each render re-evaluates without a hook.
export function relativeTime(thenMs: number, nowMs: number): string {
  const diff = nowMs - thenMs;
  if (diff < 0) return "just now";
  const s = Math.round(diff / 1000);
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
}
