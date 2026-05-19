import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
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
import { useRpc, type AutoBypassPolicy, type Job, type ServerInfo } from "@/lib/rpc";
import {
  EFFORT_LEVELS as COMPOSER_EFFORT_LEVELS,
  PERMISSION_MODES,
  RUNNER_CAPS,
  SERVER_PICK,
} from "./composer/runnerCaps";

// `runnerCaps.EFFORT_LEVELS` is the canonical low/medium/high list
// shared with `SubmitJobDialog`. EditJob also needs a "no override"
// row so the user can clear an effort previously set on the job —
// that row is local to this dialog because Submit defaults to
// "server picks" via `SERVER_PICK` instead.
const NO_EFFORT = "__none__";
const EFFORT_LEVELS: { id: string; label: string }[] = [
  { id: NO_EFFORT, label: "Default (no extra thinking)" },
  ...COMPOSER_EFFORT_LEVELS,
];

// Auto-bypass policy picker — mirrors `SubmitJobDialog` so the
// operator sees the same menu whether they are creating the job
// or amending it. The dialog's `editable` gate already restricts
// to Draft / Stopped / Failed / Completed, which is strictly
// inside the `set_job_policy` accept set (Q5: refuses on Running
// / AwaitingReview), so the call is always allowed when the form
// is visible.
const NO_POLICY = "__no_policy__";
const POLICY_CUSTOM = "custom";

type PresetPolicyKind = Exclude<AutoBypassPolicy["type"], "custom">;

const POLICY_PRESETS: { id: PresetPolicyKind; label: string; hint: string }[] = [
  {
    id: "quick",
    label: "Quick",
    hint: "Smallest change that works; skip nice-to-haves and refactors.",
  },
  {
    id: "long-term",
    label: "Long-term",
    hint: "Prefer the durable fix; tests stay in sync with behaviour.",
  },
  {
    id: "cheap",
    label: "Cheap",
    hint: "Minimise tokens and tool calls; one-line fixes ship.",
  },
  {
    id: "best-judgement",
    label: "Best judgement",
    hint: "Let the runner decide quality-vs-speed with no operator present.",
  },
  {
    id: "just-code",
    label: "Just code",
    hint: "Pick a reasonable approach and ship it; do not block on questions.",
  },
  {
    id: "relentless",
    label: "Relentless",
    hint: "Never stops on stage failure; only the cost cap and wall-clock cap halt the job.",
  },
];

function policyToFormState(p: AutoBypassPolicy | null | undefined): {
  kind: string;
  comment: string;
} {
  if (!p) return { kind: NO_POLICY, comment: "" };
  if (p.type === "custom") return { kind: POLICY_CUSTOM, comment: p.comment };
  return { kind: p.type, comment: "" };
}

function policiesEqual(
  a: AutoBypassPolicy | null,
  b: AutoBypassPolicy | null,
): boolean {
  if (!a && !b) return true;
  if (!a || !b) return false;
  if (a.type !== b.type) return false;
  if (a.type === "custom" && b.type === "custom") return a.comment === b.comment;
  return true;
}

interface Props {
  job: Job;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: (updated: Job) => void;
}

export function EditJobDialog({ job, open, onOpenChange, onSaved }: Props) {
  const rpc = useRpc();
  const [info, setInfo] = useState<ServerInfo | null>(null);

  const [runner, setRunner] = useState(job.runner);
  const [model, setModel] = useState(job.model ?? SERVER_PICK);
  const [permissionMode, setPermissionMode] = useState(
    job.permission_mode ?? SERVER_PICK,
  );
  const [effort, setEffort] = useState(job.effort || NO_EFFORT);
  const [branch, setBranch] = useState(job.branch);
  const [costCapUsd, setCostCapUsd] = useState(
    String(job.cost_cap_cents / 100),
  );
  const [wallClockMin, setWallClockMin] = useState(
    String(Math.round(job.wall_clock_cap_ms / 60_000)),
  );
  const initialPolicyForm = policyToFormState(job.auto_bypass_policy);
  const [policyKind, setPolicyKind] = useState<string>(initialPolicyForm.kind);
  const [customComment, setCustomComment] = useState<string>(initialPolicyForm.comment);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const costCapCents = Math.max(
    1,
    Math.round(parseFloat(costCapUsd || "0") * 100),
  );
  const wallClockMs = Math.max(
    1,
    Math.round(parseFloat(wallClockMin || "0") * 60_000),
  );

  const caps = RUNNER_CAPS[runner];
  const models = caps?.models ?? [];

  const policyCustomTrimmed = customComment.trim();
  const policyCustomValid =
    policyKind !== POLICY_CUSTOM || policyCustomTrimmed.length > 0;
  const nextPolicy: AutoBypassPolicy | null = (() => {
    if (policyKind === NO_POLICY) return null;
    if (policyKind === POLICY_CUSTOM) {
      return policyCustomTrimmed.length > 0
        ? { type: "custom", comment: policyCustomTrimmed }
        : null;
    }
    return { type: policyKind as PresetPolicyKind };
  })();

  // Re-seed form when the dialog opens or the user switches to a
  // different job. Depending on the `job` *object* (not its id) made
  // every parent SSE-driven refetch stomp user edits mid-typing —
  // pick "Quick" in the policy dropdown, parent re-renders with a
  // fresh job object identity, this effect re-runs and snaps the
  // dropdown back to its persisted value before Save can fire.
  useEffect(() => {
    if (!open) return;
    setRunner(job.runner);
    setModel(job.model ?? SERVER_PICK);
    setPermissionMode(job.permission_mode ?? SERVER_PICK);
    setEffort(job.effort || NO_EFFORT);
    setBranch(job.branch);
    setCostCapUsd(String(job.cost_cap_cents / 100));
    setWallClockMin(String(Math.round(job.wall_clock_cap_ms / 60_000)));
    const next = policyToFormState(job.auto_bypass_policy);
    setPolicyKind(next.kind);
    setCustomComment(next.comment);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, job.id]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    rpc.serverInfo().then((i) => {
      if (!cancelled) setInfo(i);
    });
    return () => {
      cancelled = true;
    };
  }, [open, rpc]);

  const editable =
    job.status === "draft" ||
    job.status === "stopped" ||
    job.status === "failed" ||
    job.status === "completed";

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      let updated = await rpc.call("update_job", {
        job_id: job.id,
        runner,
        model: model === SERVER_PICK ? "" : model,
        permission_mode:
          permissionMode === SERVER_PICK ? "" : permissionMode,
        effort: effort === NO_EFFORT ? "" : effort,
        cost_cap_cents: costCapCents,
        wall_clock_cap_ms: wallClockMs,
        branch,
      });
      // `update_job` does not carry the policy field (it pre-dates
      // Surface F and goes through a different gate). The policy
      // hop is a separate RPC so the audit event
      // (`JobPolicyChanged`) keeps a clean signal of operator
      // intent rather than getting folded into a generic
      // `JobUpdated`.
      if (!policiesEqual(nextPolicy, job.auto_bypass_policy ?? null)) {
        updated = await rpc.call("set_job_policy", {
          job_id: job.id,
          policy: nextPolicy,
        });
      }
      onSaved(updated);
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit job</DialogTitle>
          <DialogDescription>
            {editable
              ? "Change runner settings, caps, or branch."
              : "This job is currently active and cannot be edited."}
          </DialogDescription>
        </DialogHeader>

        {!editable ? (
          <p className="text-muted-foreground py-4 text-sm">
            Stop the job first to edit its settings.
          </p>
        ) : (
          <div className="grid gap-4 py-2">
            {/* Runner */}
            <div className="grid gap-1.5">
              <Label>Runner</Label>
              <Select value={runner} onValueChange={setRunner}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {(info?.runners ?? [{ id: runner, default: true }]).map(
                    (r) => (
                      <SelectItem key={r.id} value={r.id}>
                        {r.id === "mock" ? "mock (demo)" : r.id}
                      </SelectItem>
                    ),
                  )}
                </SelectContent>
              </Select>
            </div>

            {/* Model */}
            {caps?.supportsModel && models.length > 0 && (
              <div className="grid gap-1.5">
                <Label>Model</Label>
                <Select value={model} onValueChange={setModel}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={SERVER_PICK}>
                      Default (server picks)
                    </SelectItem>
                    {models.map((m) => (
                      <SelectItem key={m.id} value={m.id}>
                        {m.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* Permission */}
            {caps?.supportsPermission && (
              <div className="grid gap-1.5">
                <Label>Permission</Label>
                <Select
                  value={permissionMode}
                  onValueChange={setPermissionMode}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={SERVER_PICK}>
                      Default (server picks)
                    </SelectItem>
                    {PERMISSION_MODES.map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        {p.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* Effort */}
            {caps?.supportsEffort && (
              <div className="grid gap-1.5">
                <Label>Effort</Label>
                <Select value={effort} onValueChange={setEffort}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {EFFORT_LEVELS.map((e) => (
                      <SelectItem key={e.id} value={e.id}>
                        {e.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* Branch */}
            <div className="grid gap-1.5">
              <Label>Branch</Label>
              <Input value={branch} onChange={(e) => setBranch(e.target.value)} />
            </div>

            {/* Caps */}
            <div className="grid grid-cols-2 gap-3">
              <div className="grid gap-1.5">
                <Label>Cost cap (USD)</Label>
                <Input
                  type="number"
                  min="0.01"
                  step="0.01"
                  value={costCapUsd}
                  onChange={(e) => setCostCapUsd(e.target.value)}
                />
              </div>
              <div className="grid gap-1.5">
                <Label>Wall-clock cap (min)</Label>
                <Input
                  type="number"
                  min="1"
                  step="1"
                  value={wallClockMin}
                  onChange={(e) => setWallClockMin(e.target.value)}
                />
              </div>
            </div>

            {/* Auto-bypass policy */}
            <div className="grid gap-1.5">
              <Label htmlFor="edit-auto-bypass-policy">
                Auto-bypass on stage failure
              </Label>
              <Select value={policyKind} onValueChange={setPolicyKind}>
                <SelectTrigger id="edit-auto-bypass-policy">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_POLICY}>
                    None — halt on stage failure
                  </SelectItem>
                  {POLICY_PRESETS.map((p) => (
                    <SelectItem key={p.id} value={p.id}>
                      {p.label}
                    </SelectItem>
                  ))}
                  <SelectItem value={POLICY_CUSTOM}>
                    Custom — write your own comment
                  </SelectItem>
                </SelectContent>
              </Select>
              {policyKind === NO_POLICY && (
                <span className="text-muted-foreground text-[10px]">
                  A failed stage halts the job; you decide how to recover
                  from the job page.
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
                    id="edit-auto-bypass-custom"
                    value={customComment}
                    onChange={(e) => setCustomComment(e.target.value)}
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

            {error && (
              <p className="text-destructive text-sm">{error}</p>
            )}
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            cancel
          </Button>
          {editable && (
            <Button onClick={handleSave} disabled={saving || !policyCustomValid}>
              {saving ? "saving…" : "save"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
