import type { RunnerInfo } from "@/lib/rpc";

// Sentinel for "no override" so `<Select>` always has a real value
// — empty strings collapse into the placeholder slot.
export const SERVER_PICK = "__server_default__";

// Per-runner capability spec. Drives which knobs the composer
// shows — runners absent from this map (or with flags off) hide
// the corresponding fields. New runners gain UI surface by adding
// a row here, not by editing the composer body. Model lists are
// intentionally short and curated; users pick "Default (server
// picks)" when in doubt.
export type RunnerCapabilities = {
  supportsModel: boolean;
  supportsPermission: boolean;
  supportsEffort: boolean;
  models: { id: string; label: string }[];
  defaultPermissionMode: string | null;
};

export const RUNNER_CAPS: Record<string, RunnerCapabilities> = {
  claude: {
    supportsModel: true,
    supportsPermission: true,
    supportsEffort: true,
    models: [
      { id: "claude-opus-4-7", label: "Claude Opus 4.7" },
      { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6" },
      { id: "claude-haiku-4-5", label: "Claude Haiku 4.5" },
    ],
    // Codeless's claude runner defaults to `Bypass` server-side because
    // there is no TTY user to answer mid-run prompts. Preselect the
    // same value so the UI agrees with what would actually run.
    defaultPermissionMode: "bypass",
  },
  codex: {
    supportsModel: true,
    supportsPermission: false,
    supportsEffort: false,
    models: [],
    defaultPermissionMode: null,
  },
  copilot: {
    supportsModel: true,
    supportsPermission: false,
    supportsEffort: false,
    models: [],
    defaultPermissionMode: null,
  },
};

export const PERMISSION_MODES: { id: string; label: string; hint: string }[] = [
  { id: "bypass", label: "Bypass", hint: "Skip every permission gate" },
  { id: "accept_edits", label: "Accept edits", hint: "Auto-approve file edits, prompt for shell" },
  { id: "plan", label: "Plan", hint: "Plan-only; no tools run" },
  { id: "default", label: "Default", hint: "Interactive — may stall headless jobs" },
];

export const EFFORT_LEVELS: { id: string; label: string }[] = [
  { id: "low", label: "Low — think" },
  { id: "medium", label: "Medium — think hard" },
  { id: "high", label: "High — ultrathink" },
];

// `mock` is the no-prereqs no-side-effects runner; tagging it as
// "demo" stops users from picking it expecting real edits.
export function runnerLabel(r: RunnerInfo): string {
  return r.id === "mock" ? "mock (demo)" : r.id;
}

// Server published only the built-in mock factory (no
// `--enable-claude`, no `--enable-anthropic`). Surfaced as a hint
// so the user understands why every job they submit will be a
// no-op.
export function onlyMockEnabled(runners: RunnerInfo[]): boolean {
  return runners.length === 1 && runners[0].id === "mock";
}

// Constrain the job name to a slug — it becomes a folder name on
// disk (`.codeless/jobs/<name>/`), so lower-case + collapse
// non-alphanumeric runs into single dashes + trim leading /
// trailing dashes. Returning the cleaned form also lets the
// composer preview "this is what your job will be called" under
// the input.
export function slugifyName(raw: string): string {
  return raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

// Minimal template YAML for a fresh job. The runtime rejects
// empty `goal` and empty `stages`, so seed with explicit
// placeholder strings the user can't miss. They edit these in
// the SPEC pane before clicking [run]. Both keep the YAML
// parseable; both render in the template summary as obvious
// "fill me in" cues.
export function buildInitialTemplate(name: string): string {
  return `name: ${name}
goal: "TODO: describe what success looks like for this job."
stages:
  - "TODO: rename me to the first stage's title"
`;
}
