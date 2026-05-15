import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useRpc, type Repo, type RunnerInfo, type ServerInfo } from "@/lib/rpc";
import { BUILTIN_AGENTS } from "@/modules/ai/lib/agents";
import { useAgentsStore } from "@/modules/ai/store/agentsStore";

// Sentinel for "no persona" in the dropdown. `useForJobs`-filtered
// personas appear below this option; the job runs with the server's
// default system prompt and the form's manually-picked model.
const NO_PERSONA = "__no_persona__";

interface Props {
  repo: Repo;
  // Defaults pulled from the repo row; tunable in the form.
  trigger?: React.ReactNode;
}

// Constrain the job name to a slug — it becomes a folder name on
// disk (`.codeless/jobs/<name>/`), so we lower-case + collapse
// non-alphanumeric runs into single dashes + trim leading/trailing
// dashes. Returning the cleaned form also lets us preview "this is
// what your job will be called" under the input.
function slugifyName(raw: string): string {
  return raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

// Minimal template YAML for a fresh job. The runtime rejects empty
// `goal` and empty `stages`, so we seed with explicit placeholder
// strings the user can't miss. They edit these in the SPEC pane
// before clicking [run]. Both keep the YAML parseable; both render
// in the template summary as obvious "fill me in" cues.
function buildInitialTemplate(name: string): string {
  return `name: ${name}
goal: "TODO: describe what success looks like for this job."
stages:
  - "TODO: rename me to the first stage's title"
`;
}

// `mock` is the no-prereqs no-side-effects runner; tagging it as "demo"
// in the dropdown stops users from picking it expecting real edits.
function runnerLabel(r: RunnerInfo): string {
  return r.id === "mock" ? "mock (demo)" : r.id;
}

// Server published only the built-in mock factory (no `--enable-claude`,
// no `--enable-anthropic`). We surface a one-line hint so the user
// understands why every job they submit will be a no-op.
function onlyMockEnabled(runners: RunnerInfo[]): boolean {
  return runners.length === 1 && runners[0].id === "mock";
}

// Per-runner capability spec. Drives which knobs the Submit dialog
// shows — runners absent from this map (or with flags off) hide the
// corresponding fields. New runners gain UI surface by adding a row
// here, not by editing the dialog body. Model lists are intentionally
// short and curated; users pick "Default (server picks)" when in doubt.
type RunnerCapabilities = {
  supportsModel: boolean;
  supportsPermission: boolean;
  supportsEffort: boolean;
  models: { id: string; label: string }[];
  defaultPermissionMode: string | null;
};

const RUNNER_CAPS: Record<string, RunnerCapabilities> = {
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
    // there is no TTY user to answer mid-run prompts. Preselect the same
    // value so the UI agrees with what would actually run.
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

const PERMISSION_MODES: { id: string; label: string; hint: string }[] = [
  { id: "bypass", label: "Bypass", hint: "Skip every permission gate" },
  { id: "accept_edits", label: "Accept edits", hint: "Auto-approve file edits, prompt for shell" },
  { id: "plan", label: "Plan", hint: "Plan-only; no tools run" },
  { id: "default", label: "Default", hint: "Interactive — may stall headless jobs" },
];

const EFFORT_LEVELS: { id: string; label: string }[] = [
  { id: "low", label: "Low — think" },
  { id: "medium", label: "Medium — think hard" },
  { id: "high", label: "High — ultrathink" },
];

const SERVER_PICK = "__server_default__";

export function SubmitJobDialog({ repo, trigger }: Props) {
  const rpc = useRpc();
  // Personas live in the same KV store the chat panel reads. We pull
  // through the zustand store so a cross-window edit (Settings →
  // Agents) refreshes the dropdown without remounting the dialog.
  const customAgents = useAgentsStore((s) => s.customAgents);
  const hydrateAgents = useAgentsStore((s) => s.hydrate);
  useEffect(() => {
    void hydrateAgents();
  }, [hydrateAgents]);
  const personasForJobs = useMemo(
    () => [...BUILTIN_AGENTS, ...customAgents].filter((a) => a.useForJobs),
    [customAgents],
  );
  const [personaId, setPersonaId] = useState<string>(NO_PERSONA);
  const selectedPersona =
    personaId === NO_PERSONA
      ? null
      : (personasForJobs.find((a) => a.id === personaId) ?? null);
  const [open, setOpen] = useState(false);
  // The job's name is the only required field. It becomes the
  // `.codeless/jobs/<name>/` folder, so it has to be a slug — letters,
  // digits, dashes. We validate live and surface the rule below the
  // input so the user never types something the server will reject.
  const [name, setName] = useState("");
  const [branch, setBranch] = useState("");
  // Track whether the user has hand-edited the branch. If they
  // haven't, we keep the branch in sync with the slugified name so
  // submitting a job doesn't require touching two fields.
  const [branchTouched, setBranchTouched] = useState(false);
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [runner, setRunner] = useState<string>(repo.default_runner ?? "");
  // Per-job runner overrides. `SERVER_PICK` is the sentinel for "no
  // override" so the `<Select>` has a real value to render — empty
  // strings would collapse into the placeholder slot.
  const [model, setModel] = useState<string>(SERVER_PICK);
  const [permissionMode, setPermissionMode] = useState<string>(SERVER_PICK);
  const [effort, setEffort] = useState<string>(SERVER_PICK);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Default OFF — landing in `Draft` lets the user edit SCOPE.md /
  // WORKFLOW.md / per-stage docs before the driver picks the job up.
  // Power users who want the legacy submit-and-run can flip this on.
  const [runImmediately, setRunImmediately] = useState(false);
  // Workspace mode: where the agent's edits land. in-repo (default)
  // edits the user's actual local clone; worktree creates a separate
  // git worktree checkout for isolation.
  const [workspaceMode, setWorkspaceMode] = useState<"in-repo" | "worktree">("in-repo");
  // Cost + wall-clock caps. The driver stops a job the moment either
  // cap is breached, so a too-low default has surfaced as "conflict:
  // job is Stopped" the user has to resume past. We expose them in
  // user units (dollars, minutes) and convert at submit so the wire
  // shape (cents, milliseconds) is unchanged.
  const [costCapUsd, setCostCapUsd] = useState<string>("5");
  const [wallClockMin, setWallClockMin] = useState<string>("30");
  const costCapCents = Math.max(1, Math.round(parseFloat(costCapUsd || "0") * 100));
  const wallClockMs = Math.max(1, Math.round(parseFloat(wallClockMin || "0") * 60_000));
  const costCapValid = Number.isFinite(parseFloat(costCapUsd)) && parseFloat(costCapUsd) > 0;
  const wallClockValid = Number.isFinite(parseFloat(wallClockMin)) && parseFloat(wallClockMin) > 0;

  const caps = RUNNER_CAPS[runner];
  const nameSlug = slugifyName(name);
  const nameValid = nameSlug.length > 0 && nameSlug === name.trim();
  // Refuse the repo's default branch (`main`, `master`, whatever the
  // repo declares). Submitting on the default branch lets the agent
  // commit + push directly to it — almost never what the user wants
  // (the worktree manager would also fail to allocate a worktree
  // because the default branch is already checked out elsewhere).
  // Common protected names are also rejected as a defence-in-depth.
  const branchTrimmed = branch.trim();
  const branchClashesDefault =
    branchTrimmed.length > 0 &&
    (branchTrimmed === repo.default_branch ||
      branchTrimmed === "main" ||
      branchTrimmed === "master");

  // Keep the branch synced with the slugified name unless the user
  // has hand-edited the branch — typing a name shouldn't lose their
  // bespoke branch, but typing nothing shouldn't leave the branch
  // empty either.
  useEffect(() => {
    if (branchTouched) return;
    setBranch(nameSlug ? `codeless/${nameSlug}` : "");
  }, [nameSlug, branchTouched]);

  // When the user switches runners, reset overrides so we never carry a
  // Claude-only permission_mode into a future runner that can't honour
  // it. Also seed the permission default from the new runner's spec.
  useEffect(() => {
    setModel(SERVER_PICK);
    setEffort(SERVER_PICK);
    setPermissionMode(caps?.defaultPermissionMode ?? SERVER_PICK);
  }, [runner, caps]);

  // Seed the Model dropdown from the persona's `defaultModel` when the
  // user picks a persona AND the runner exposes that exact model id.
  // We leave the user free to override afterwards — the seed only fires
  // when the persona changes. A persona id that has no defaultModel
  // (or whose preferred model isn't in the current runner's catalogue)
  // leaves the field on whatever the user last picked.
  useEffect(() => {
    if (!selectedPersona?.defaultModel || !caps?.supportsModel) return;
    const match = caps.models.find((m) => m.id === selectedPersona.defaultModel);
    if (match) setModel(match.id);
  }, [selectedPersona, caps]);

  // Fetch /server/info once per dialog-open. Re-running on each open
  // is cheap (a single unauthenticated GET) and reflects post-boot
  // state changes — e.g. operator restarted with --enable-claude.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    rpc
      .serverInfo()
      .then((i) => {
        if (cancelled) return;
        setInfo(i);
        // Prefer the repo's saved default when the server still
        // advertises that runner; otherwise honour the server's
        // own default flag. This keeps repo-level preferences
        // sticky while not silently submitting jobs against a
        // runner the operator has since disabled.
        const repoPick = repo.default_runner
          ? i.runners.find((r) => r.id === repo.default_runner)
          : undefined;
        const serverDefault = i.runners.find((r) => r.default);
        setRunner(repoPick?.id ?? serverDefault?.id ?? i.runners[0]?.id ?? "");
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setError(
          `could not load runner list: ${e instanceof Error ? e.message : String(e)}`,
        );
      });
    return () => {
      cancelled = true;
    };
  }, [open, rpc, repo.default_runner]);

  const submit = async () => {
    if (!nameValid) {
      setError("name must be a slug: lowercase letters, digits, and dashes");
      return;
    }
    if (branchClashesDefault) {
      setError(
        `branch must not be ${branchTrimmed} — that's the repo's default branch; pick something like codeless/${nameSlug}`,
      );
      return;
    }
    setSubmitting(true);
    setError(null);
    // Hard timeout so a hung transport (server down mid-submit, an
    // unreachable mock client, an SSE proxy stalling the fetch) does
    // not leave the button frozen on "submitting…". 10s is well above
    // the trait method's normal latency on a healthy core.
    const timer = window.setTimeout(() => {
      setError("submit timed out after 10s — check the server is reachable");
      setSubmitting(false);
    }, 10_000);
    try {
      // Minimal valid template YAML. The server's submit_job parses
      // this, scaffolds `.codeless/jobs/<name>/template.yaml`,
      // SCOPE.md, and WORKFLOW.md, and commits all three. Goal +
      // stages start empty; the user fills them in via the SPEC pane
      // before clicking `[run]`.
      const templateYaml = buildInitialTemplate(nameSlug);
      const job = await rpc.call("submit_job", {
        repo_id: repo.id,
        // Prompt-only submits are a CLI concern now; the UI always
        // sends a template so the spec exists from second one.
        prompt: null,
        template_yaml: templateYaml,
        runner,
        branch,
        workspace_mode: workspaceMode,
        cost_cap_cents: costCapCents,
        wall_clock_cap_ms: wallClockMs,
        // `SERVER_PICK` means "no override" — send null so the
        // adapter's default applies. Fields the runner doesn't
        // support stay null even if their state has a stale value.
        model: caps?.supportsModel && model !== SERVER_PICK ? model : null,
        permission_mode:
          caps?.supportsPermission && permissionMode !== SERVER_PICK
            ? permissionMode
            : null,
        effort: caps?.supportsEffort && effort !== SERVER_PICK ? effort : null,
        // The selected persona's `instructions` become the per-job
        // system prompt the runtime applies on top of the server's
        // baseline. Personas are pure config: the UI just hands the
        // composed text to the runtime, which composes the final
        // prompt server-side. `null` (no persona picked) keeps the
        // server's configured default unchanged.
        system_prompt: selectedPersona ? selectedPersona.instructions : null,
        start_immediately: runImmediately,
      });
      // eslint-disable-next-line no-console
      console.log("submit_job ok", job);
      setOpen(false);
      setName("");
      setBranch("");
      setBranchTouched(false);
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error("submit_job failed", e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      window.clearTimeout(timer);
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        {trigger ?? <Button size="sm">new job</Button>}
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Submit job — {repo.name}</DialogTitle>
          <DialogDescription>
            Pick a name. The job lands as a draft with{" "}
            <code>template.yaml</code>, <code>SCOPE.md</code>, and{" "}
            <code>WORKFLOW.md</code> already on disk — edit them in the
            SPEC pane, then click <code>run</code>.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="name">Name</Label>
            <Input
              id="name"
              value={name}
              onChange={(e) => setName(e.target.value)}
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
                  setBranchTouched(true);
                  setBranch(e.target.value);
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
                onValueChange={setRunner}
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
              <Select value={personaId} onValueChange={setPersonaId}>
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
                  <Select value={model} onValueChange={setModel}>
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
                  <Select value={permissionMode} onValueChange={setPermissionMode}>
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
                  <Select value={effort} onValueChange={setEffort}>
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
                onChange={(e) => setCostCapUsd(e.target.value)}
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
                onChange={(e) => setWallClockMin(e.target.value)}
                className="border-input bg-background h-8 rounded-md border px-2 text-sm"
              />
              <span className="text-muted-foreground text-[10px]">
                Driver stops the job after this many minutes of run time.
              </span>
            </div>
          </div>
          {error && <div className="text-destructive text-xs">{error}</div>}
          <div className="grid gap-1.5">
            <Label>Workspace mode</Label>
            <div className="flex gap-4">
              <label className="flex cursor-pointer items-center gap-1.5 text-xs">
                <input
                  type="radio"
                  name="workspace_mode"
                  checked={workspaceMode === "in-repo"}
                  onChange={() => setWorkspaceMode("in-repo")}
                />
                <span className="font-medium">In-repo</span>
              </label>
              <label className="flex cursor-pointer items-center gap-1.5 text-xs">
                <input
                  type="radio"
                  name="workspace_mode"
                  checked={workspaceMode === "worktree"}
                  onChange={() => setWorkspaceMode("worktree")}
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
          <label className="hover:bg-accent/40 flex cursor-pointer items-start gap-2 rounded p-2 text-xs">
            <input
              type="checkbox"
              checked={runImmediately}
              onChange={(e) => setRunImmediately(e.target.checked)}
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
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            cancel
          </Button>
          <Button
            onClick={submit}
            disabled={
              submitting ||
              nameSlug.length === 0 ||
              branchClashesDefault ||
              !costCapValid ||
              !wallClockValid
            }
          >
            {submitting
              ? "submitting…"
              : runImmediately
                ? "submit + run"
                : "save as draft"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
