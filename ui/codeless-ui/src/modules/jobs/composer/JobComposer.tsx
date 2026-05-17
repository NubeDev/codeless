import { useEffect, useMemo, useState } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type {
  AutoBypassPolicy,
  Repo,
  ServerInfo,
  SubmitJobArgs,
} from "@/lib/rpc";
import { BUILTIN_AGENTS, type Agent } from "@/modules/ai/lib/agents";
import { useAgentsStore } from "@/modules/ai/store/agentsStore";

import {
  EFFORT_LEVELS,
  PERMISSION_MODES,
  RUNNER_CAPS,
  SERVER_PICK,
  buildInitialTemplate,
  onlyMockEnabled,
  runnerLabel,
  slugifyName,
} from "./runnerCaps";
import {
  NO_POLICY,
  POLICY_CUSTOM,
  POLICY_PRESETS,
  pickerFromPolicy,
  policyFromPicker,
} from "./policyPresets";

// Sentinel for "no persona" in the dropdown. `useForJobs`-filtered
// personas appear below this option; the job runs with the server's
// default system prompt and the composer's manually-picked model.
const NO_PERSONA = "__no_persona__";

// Seed values the planner can hand to a `draft_job` action card.
// Everything optional — anything omitted falls back to the same
// default `SubmitJobDialog` uses today. Fields the user later
// edits in the composer override the seed.
export interface JobComposerInitial {
  name?: string;
  branch?: string;
  runner?: string;
  workspaceMode?: "in-repo" | "worktree";
  costCapUsd?: string;
  wallClockMin?: string;
  policy?: AutoBypassPolicy | null;
  model?: string;
  permissionMode?: string;
  effort?: string;
  personaId?: string;
  runImmediately?: boolean;
}

// Single value object the hook returns. Carries every field plus
// the derived validity flags the submit button reads. Held on the
// caller's side so a dialog shell, a chat action card and a future
// settings panel can all read the same state without wiring a
// dozen callback props.
export interface JobComposerState {
  name: string;
  setName(v: string): void;
  branch: string;
  setBranch(v: string): void;
  branchTouched: boolean;
  setBranchTouched(v: boolean): void;
  runner: string;
  setRunner(v: string): void;
  workspaceMode: "in-repo" | "worktree";
  setWorkspaceMode(v: "in-repo" | "worktree"): void;
  costCapUsd: string;
  setCostCapUsd(v: string): void;
  wallClockMin: string;
  setWallClockMin(v: string): void;
  policyKind: string;
  setPolicyKind(v: string): void;
  customComment: string;
  setCustomComment(v: string): void;
  model: string;
  setModel(v: string): void;
  permissionMode: string;
  setPermissionMode(v: string): void;
  effort: string;
  setEffort(v: string): void;
  personaId: string;
  setPersonaId(v: string): void;
  runImmediately: boolean;
  setRunImmediately(v: boolean): void;

  info: ServerInfo | null;
  setInfo(i: ServerInfo | null): void;

  personasForJobs: Agent[];
  selectedPersona: Agent | null;

  // Derived.
  nameSlug: string;
  nameValid: boolean;
  branchClashesDefault: boolean;
  costCapValid: boolean;
  wallClockValid: boolean;
  policyCustomValid: boolean;
  policy: AutoBypassPolicy | null;
  /** Final gate for the submit button. */
  canSubmit: boolean;

  repo: Repo;
}

// Owns the field state for a new-job form. Lives outside the
// component body so a dialog shell and an assistant draft card
// can both render `<JobComposer state={state} />` and submit by
// reading the same state. Stage initial values from a planner
// proposal via `initial`; the user's edits override.
export function useJobComposerState(opts: {
  repo: Repo;
  initial?: JobComposerInitial;
}): JobComposerState {
  const { repo, initial } = opts;
  const initialPolicy = pickerFromPolicy(initial?.policy ?? null);

  const [name, setName] = useState<string>(initial?.name ?? "");
  const [branch, setBranch] = useState<string>(initial?.branch ?? "");
  const [branchTouched, setBranchTouched] = useState<boolean>(
    Boolean(initial?.branch),
  );
  const [runner, setRunner] = useState<string>(
    initial?.runner ?? repo.default_runner ?? "",
  );
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [workspaceMode, setWorkspaceMode] = useState<"in-repo" | "worktree">(
    initial?.workspaceMode ?? "in-repo",
  );
  const [costCapUsd, setCostCapUsd] = useState<string>(
    initial?.costCapUsd ?? "5",
  );
  const [wallClockMin, setWallClockMin] = useState<string>(
    initial?.wallClockMin ?? "30",
  );
  const [policyKind, setPolicyKind] = useState<string>(initialPolicy.kind);
  const [customComment, setCustomComment] = useState<string>(
    initialPolicy.customComment,
  );
  const [model, setModel] = useState<string>(initial?.model ?? SERVER_PICK);
  const [permissionMode, setPermissionMode] = useState<string>(
    initial?.permissionMode ?? SERVER_PICK,
  );
  const [effort, setEffort] = useState<string>(initial?.effort ?? SERVER_PICK);
  const [personaId, setPersonaId] = useState<string>(
    initial?.personaId ?? NO_PERSONA,
  );
  const [runImmediately, setRunImmediately] = useState<boolean>(
    initial?.runImmediately ?? false,
  );

  // Personas live in the same KV store the chat panel reads. Pull
  // through the zustand store so a cross-window edit (Settings →
  // Agents) refreshes the dropdown without remounting the host.
  const customAgents = useAgentsStore((s) => s.customAgents);
  const hydrateAgents = useAgentsStore((s) => s.hydrate);
  useEffect(() => {
    void hydrateAgents();
  }, [hydrateAgents]);
  const personasForJobs = useMemo(
    () => [...BUILTIN_AGENTS, ...customAgents].filter((a) => a.useForJobs),
    [customAgents],
  );
  const selectedPersona =
    personaId === NO_PERSONA
      ? null
      : (personasForJobs.find((a) => a.id === personaId) ?? null);

  // Keep the branch synced with the slugified name unless the user
  // has hand-edited the branch — typing a name shouldn't lose their
  // bespoke branch, but typing nothing shouldn't leave the branch
  // empty either.
  const nameSlug = slugifyName(name);
  useEffect(() => {
    if (branchTouched) return;
    setBranch(nameSlug ? `codeless/${nameSlug}` : "");
  }, [nameSlug, branchTouched]);

  const caps = RUNNER_CAPS[runner];

  // When the user switches runners, reset overrides so a Claude-only
  // permission_mode never bleeds into a future runner that can't
  // honour it. Seed the permission default from the new runner's
  // spec.
  useEffect(() => {
    setModel(SERVER_PICK);
    setEffort(SERVER_PICK);
    setPermissionMode(caps?.defaultPermissionMode ?? SERVER_PICK);
  }, [runner, caps]);

  // Seed the Model dropdown from the persona's `defaultModel` when
  // the user picks a persona AND the runner exposes that exact id.
  // Override remains free afterwards — the seed only fires when the
  // persona changes. A persona with no defaultModel (or whose
  // preferred model isn't in the current runner's catalogue) leaves
  // the field on whatever the user last picked.
  useEffect(() => {
    if (!selectedPersona?.defaultModel || !caps?.supportsModel) return;
    const match = caps.models.find((m) => m.id === selectedPersona.defaultModel);
    if (match) setModel(match.id);
  }, [selectedPersona, caps]);

  const nameValid = nameSlug.length > 0 && nameSlug === name.trim();
  // Refuse the repo's default branch (`main`, `master`, whatever
  // the repo declares). Submitting on the default branch lets the
  // agent commit + push directly to it — almost never what the user
  // wants (the worktree manager would also fail to allocate a
  // worktree because the default branch is already checked out
  // elsewhere). Common protected names are also rejected as
  // defence-in-depth.
  const branchTrimmed = branch.trim();
  const branchClashesDefault =
    branchTrimmed.length > 0 &&
    (branchTrimmed === repo.default_branch ||
      branchTrimmed === "main" ||
      branchTrimmed === "master");
  const costCapValid =
    Number.isFinite(parseFloat(costCapUsd)) && parseFloat(costCapUsd) > 0;
  const wallClockValid =
    Number.isFinite(parseFloat(wallClockMin)) && parseFloat(wallClockMin) > 0;
  const policyCustomValid =
    policyKind !== POLICY_CUSTOM || customComment.trim().length > 0;
  const policy = policyFromPicker(policyKind, customComment);
  const canSubmit =
    nameValid &&
    !branchClashesDefault &&
    costCapValid &&
    wallClockValid &&
    policyCustomValid;

  return {
    name,
    setName,
    branch,
    setBranch,
    branchTouched,
    setBranchTouched,
    runner,
    setRunner,
    workspaceMode,
    setWorkspaceMode,
    costCapUsd,
    setCostCapUsd,
    wallClockMin,
    setWallClockMin,
    policyKind,
    setPolicyKind,
    customComment,
    setCustomComment,
    model,
    setModel,
    permissionMode,
    setPermissionMode,
    effort,
    setEffort,
    personaId,
    setPersonaId,
    runImmediately,
    setRunImmediately,

    info,
    setInfo,

    personasForJobs,
    selectedPersona,

    nameSlug,
    nameValid,
    branchClashesDefault,
    costCapValid,
    wallClockValid,
    policyCustomValid,
    policy,
    canSubmit,

    repo,
  };
}

// Pure mapping from composer state → `submit_job` wire args. The
// shell calls this when the user confirms; the assistant card
// calls it after the user edits a planner-seeded form. Keeping the
// mapping pure (no rpc, no setState) is what lets both surfaces
// share validation without each re-deriving cents/ms/null overrides.
export function composerToSubmitArgs(state: JobComposerState): SubmitJobArgs {
  const caps = RUNNER_CAPS[state.runner];
  const costCapCents = Math.max(
    1,
    Math.round(parseFloat(state.costCapUsd || "0") * 100),
  );
  const wallClockMs = Math.max(
    1,
    Math.round(parseFloat(state.wallClockMin || "0") * 60_000),
  );
  return {
    repo_id: state.repo.id,
    prompt: null,
    // Prompt-only submits are a CLI concern now; the UI always
    // sends a template so the spec exists from second one.
    template_yaml: buildInitialTemplate(state.nameSlug),
    runner: state.runner,
    branch: state.branch,
    workspace_mode: state.workspaceMode,
    cost_cap_cents: costCapCents,
    wall_clock_cap_ms: wallClockMs,
    // `SERVER_PICK` means "no override" — send null so the adapter
    // default applies. Fields the runner doesn't support stay null
    // even if their state has a stale value.
    model:
      caps?.supportsModel && state.model !== SERVER_PICK ? state.model : null,
    permission_mode:
      caps?.supportsPermission && state.permissionMode !== SERVER_PICK
        ? state.permissionMode
        : null,
    effort:
      caps?.supportsEffort && state.effort !== SERVER_PICK
        ? state.effort
        : null,
    // The selected persona's `instructions` become the per-job
    // system prompt the runtime applies on top of the server's
    // baseline. Personas are pure config: the UI hands the composed
    // text to the runtime, which composes the final prompt
    // server-side. `null` (no persona picked) keeps the server's
    // configured default unchanged.
    system_prompt: state.selectedPersona
      ? state.selectedPersona.instructions
      : null,
    persona_id: state.selectedPersona ? state.selectedPersona.id : null,
    auto_bypass_policy: state.policy,
    start_immediately: state.runImmediately,
  };
}

export interface JobComposerProps {
  state: JobComposerState;
  /**
   * Hide the "Run immediately" checkbox. The assistant's
   * `draft_job` card never wants the legacy submit-and-run mode
   * because the planner's draft is always for human review.
   */
  hideRunImmediately?: boolean;
}

// Pure render component. Owns no state of its own — every field
// reads / writes through the `JobComposerState` the hook returns.
// Two surfaces use this: `SubmitJobDialog` (the new-job dialog
// shell) and the assistant `draft_job` action card. Both render
// the same fields, the same validation hints and the same picker
// copy — divergence here is the bug `SCOPE-ASSISTANT-PARITY` W2
// closes.
export function JobComposer({ state, hideRunImmediately }: JobComposerProps) {
  const {
    repo,
    name,
    nameSlug,
    nameValid,
    branch,
    branchClashesDefault,
    runner,
    info,
    workspaceMode,
    costCapUsd,
    wallClockMin,
    policyKind,
    customComment,
    policyCustomValid,
    model,
    permissionMode,
    effort,
    personaId,
    personasForJobs,
    selectedPersona,
    runImmediately,
  } = state;
  const caps = RUNNER_CAPS[runner];

  return (
    <div className="grid gap-3">
      <div className="grid gap-1.5">
        <Label htmlFor="name">Name</Label>
        <Input
          id="name"
          value={name}
          onChange={(e) => state.setName(e.target.value)}
          placeholder="user-profile"
          spellCheck={false}
          autoFocus
        />
        <span className="text-muted-foreground text-[10px]">
          Folder name under <code>.codeless/jobs/</code>; lowercase
          letters, digits, and dashes.{" "}
          {name && !nameValid && nameSlug && (
            <span className="text-amber-700 dark:text-amber-400">
              Will be saved as <code>{nameSlug}</code>.
            </span>
          )}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="branch">Branch</Label>
          <Input
            id="branch"
            value={branch}
            onChange={(e) => {
              state.setBranchTouched(true);
              state.setBranch(e.target.value);
            }}
            aria-invalid={branchClashesDefault}
          />
          {branchClashesDefault && (
            <span className="text-destructive text-[10px]">
              refused: that's the repo's default branch (
              <code>{repo.default_branch}</code>). Pick a unique branch
              like <code>codeless/{nameSlug || "<name>"}</code>.
            </span>
          )}
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="runner">Runner</Label>
          <Select
            value={runner}
            onValueChange={state.setRunner}
            disabled={!info || info.runners.length === 0}
          >
            <SelectTrigger id="runner">
              <SelectValue placeholder={info ? "Select runner" : "Loading…"} />
            </SelectTrigger>
            <SelectContent>
              {info?.runners.map((r) => (
                <SelectItem key={r.id} value={r.id}>
                  {runnerLabel(r)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      {personasForJobs.length > 0 && (
        <div className="grid gap-1.5">
          <Label htmlFor="persona">Persona</Label>
          <Select value={personaId} onValueChange={state.setPersonaId}>
            <SelectTrigger id="persona">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={NO_PERSONA}>None — use server default</SelectItem>
              {personasForJobs.map((p) => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                  {p.builtIn ? " (built-in)" : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {selectedPersona && (
            <span className="text-muted-foreground text-[10px]">
              Persona instructions are appended to the runner system
              prompt for every stage. Toggle a persona's “Use for
              jobs” in Settings → Agents to add or remove it here.
            </span>
          )}
        </div>
      )}
      {info && onlyMockEnabled(info.runners) && (
        <div className="rounded border border-yellow-500/40 bg-yellow-500/10 px-2 py-1.5 text-[11px] text-yellow-700 dark:text-yellow-300">
          Only the demo `mock` runner is enabled. Restart the server
          with <code>--enable-claude</code> (or <code>--enable-anthropic</code>)
          to submit real coding jobs.
        </div>
      )}
      {caps && (caps.supportsModel || caps.supportsPermission || caps.supportsEffort) && (
        <div className="grid grid-cols-2 gap-3">
          {caps.supportsModel && (
            <div className="grid gap-1.5">
              <Label htmlFor="model">Model</Label>
              <Select value={model} onValueChange={state.setModel}>
                <SelectTrigger id="model">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={SERVER_PICK}>Default (server picks)</SelectItem>
                  {caps.models.map((m) => (
                    <SelectItem key={m.id} value={m.id}>
                      {m.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          {caps.supportsPermission && (
            <div className="grid gap-1.5">
              <Label htmlFor="permission">Permission</Label>
              <Select value={permissionMode} onValueChange={state.setPermissionMode}>
                <SelectTrigger id="permission">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={SERVER_PICK}>Default (server picks)</SelectItem>
                  {PERMISSION_MODES.map((p) => (
                    <SelectItem key={p.id} value={p.id}>
                      {p.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          {caps.supportsEffort && (
            <div className="grid gap-1.5">
              <Label htmlFor="effort">Effort</Label>
              <Select value={effort} onValueChange={state.setEffort}>
                <SelectTrigger id="effort">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={SERVER_PICK}>Default (no extra thinking)</SelectItem>
                  {EFFORT_LEVELS.map((e) => (
                    <SelectItem key={e.id} value={e.id}>
                      {e.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
        </div>
      )}
      <div className="grid grid-cols-2 gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="cost-cap">Cost cap (USD)</Label>
          <input
            id="cost-cap"
            type="number"
            min="0.01"
            step="0.5"
            value={costCapUsd}
            onChange={(e) => state.setCostCapUsd(e.target.value)}
            className="border-input bg-background h-8 rounded-md border px-2 text-sm"
          />
          <span className="text-muted-foreground text-[10px]">
            Driver stops the job when spend exceeds this. Resume to
            bump it from the job page.
          </span>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="wall-clock">Wall-clock cap (minutes)</Label>
          <input
            id="wall-clock"
            type="number"
            min="1"
            step="5"
            value={wallClockMin}
            onChange={(e) => state.setWallClockMin(e.target.value)}
            className="border-input bg-background h-8 rounded-md border px-2 text-sm"
          />
          <span className="text-muted-foreground text-[10px]">
            Driver stops the job after this many minutes of run time.
          </span>
        </div>
      </div>
      <div className="grid gap-1.5">
        <Label>Workspace mode</Label>
        <div className="flex gap-4">
          <label className="flex cursor-pointer items-center gap-1.5 text-xs">
            <input
              type="radio"
              name="workspace_mode"
              checked={workspaceMode === "in-repo"}
              onChange={() => state.setWorkspaceMode("in-repo")}
            />
            <span className="font-medium">In-repo</span>
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs">
            <input
              type="radio"
              name="workspace_mode"
              checked={workspaceMode === "worktree"}
              onChange={() => state.setWorkspaceMode("worktree")}
            />
            <span className="font-medium">Worktree</span>
          </label>
        </div>
        <span className="text-muted-foreground text-[10px]">
          {workspaceMode === "in-repo"
            ? "Agent edits your local clone directly — git log, IDE, dev server all see changes live."
            : "Agent edits a separate worktree checkout — isolates concurrent jobs but edits live in /tmp."}
        </span>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="auto-bypass-policy">Auto-bypass on stage failure</Label>
        <Select value={policyKind} onValueChange={state.setPolicyKind}>
          <SelectTrigger id="auto-bypass-policy">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NO_POLICY}>None — halt on stage failure</SelectItem>
            {POLICY_PRESETS.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.label}
              </SelectItem>
            ))}
            <SelectItem value={POLICY_CUSTOM}>Custom — write your own comment</SelectItem>
          </SelectContent>
        </Select>
        {policyKind === NO_POLICY && (
          <span className="text-muted-foreground text-[10px]">
            Default. A failed stage halts the job; you decide how to
            recover from the job page.
          </span>
        )}
        {policyKind !== NO_POLICY && policyKind !== POLICY_CUSTOM && (
          <span className="text-muted-foreground text-[10px]">
            {POLICY_PRESETS.find((p) => p.id === policyKind)?.hint}{" "}
            Cap-breach failures still halt regardless.
          </span>
        )}
        {policyKind === POLICY_CUSTOM && (
          <>
            <textarea
              id="auto-bypass-custom"
              value={customComment}
              onChange={(e) => state.setCustomComment(e.target.value)}
              rows={3}
              placeholder="One paragraph of guidance threaded into the next stage when a failure is auto-bypassed."
              aria-invalid={!policyCustomValid}
              className="border-input bg-background rounded-md border px-2 py-1.5 text-xs"
            />
            <span className="text-muted-foreground text-[10px]">
              Wrapped in an <code>Operator comment</code> envelope
              and prepended to the next stage's prompt verbatim.
              Cap-breach failures still halt regardless.
            </span>
          </>
        )}
      </div>
      {!hideRunImmediately && (
        <label className="hover:bg-accent/40 flex cursor-pointer items-start gap-2 rounded p-2 text-xs">
          <input
            type="checkbox"
            checked={runImmediately}
            onChange={(e) => state.setRunImmediately(e.target.checked)}
            className="mt-0.5"
          />
          <span>
            <span className="font-medium">Run immediately</span>
            <span className="text-muted-foreground block">
              Off (default): the job lands as a <code>draft</code> so you
              can edit SCOPE.md / WORKFLOW.md / per-stage docs before it
              runs. Click <code>run</code> on the job page when ready.
            </span>
          </span>
        </label>
      )}
    </div>
  );
}
