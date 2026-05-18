// Surface F policy reminder. A small pill in the JobPage header that
// names the active auto-bypass policy ("policy: Quick", "policy:
// Custom", ...) so the operator never forgets the job is configured
// to advance through stage failures on its own. Click opens a
// modal that lets the operator change or clear the policy via
// `set_job_policy`; the runtime refuses the call while the job is
// Running or AwaitingReview, so the modal's save button is gated on
// the same statuses and surfaces the runtime's Conflict inline if a
// status transition races the dialog.

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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { useRpc, type AutoBypassPolicy, type Job } from "@/lib/rpc";

// Preset rows mirror the SubmitJobDialog picker so the badge's
// modal speaks the same vocabulary as the form that first set the
// policy. The hint text repeats the short description rather than
// referencing the canned-comment body (which lives in Rust source
// per AUTO-BYPASS-DECISIONS.md Q4).
type PresetKind = Exclude<AutoBypassPolicy["type"], "custom">;

const PRESETS: { id: PresetKind; label: string; hint: string }[] = [
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

const NO_POLICY = "__no_policy__";
const POLICY_CUSTOM = "custom";

// Short human label for the badge text and the modal heading. The
// preset names match `AutoBypassPolicy::policy_name()` on the Rust
// side; we hand-mirror them here rather than rely on a wire field
// so the badge does not have to wait for a follow-up RPC roundtrip
// to render after a policy change.
export function policyDisplayName(policy: AutoBypassPolicy): string {
  switch (policy.type) {
    case "quick":
      return "Quick";
    case "long-term":
      return "Long-term";
    case "cheap":
      return "Cheap";
    case "best-judgement":
      return "Best judgement";
    case "just-code":
      return "Just code";
    case "relentless":
      return "Relentless";
    case "custom":
      return "Custom";
  }
}

// Statuses the runtime accepts on `set_job_policy`. Mirrors the
// AUTO-BYPASS-DECISIONS.md Q5 server contract; the modal disables
// its save button rather than letting the operator click into a
// guaranteed Conflict.
function canEditPolicy(status: Job["status"]): boolean {
  return status !== "running" && status !== "awaiting-review";
}

interface BadgeProps {
  job: Job;
  onUpdated: () => void;
}

export function PolicyBadge({ job, onUpdated }: BadgeProps) {
  const [open, setOpen] = useState(false);
  const policy = job.auto_bypass_policy;
  const editable = canEditPolicy(job.status);
  // Render an "attach policy" affordance when none is set. Without
  // this the badge disappears entirely and the operator has no path
  // to opt a fresh job into hands-off advancement, which is the
  // whole point of Surface F.
  const label = policy
    ? `policy: ${policyDisplayName(policy)}`
    : "policy: none";
  const unsetTitle = editable
    ? "Click to set an auto-bypass policy"
    : "Policy is locked while the job is running — pause to set one";
  const setTitle = editable
    ? "Click to change or clear the auto-bypass policy"
    : "Policy is locked while the job is running — pause to change";
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title={policy ? setTitle : unsetTitle}
        className={cn(
          "inline-flex h-5 shrink-0 items-center rounded-full border px-2 text-[11px] font-medium transition-colors",
          policy
            ? "border-border/60 bg-muted/40 text-muted-foreground hover:border-border hover:text-foreground"
            : "border-dashed border-border/50 text-muted-foreground/70 hover:border-border hover:text-foreground",
        )}
      >
        {label}
      </button>
      <PolicyDialog
        job={job}
        open={open}
        onOpenChange={setOpen}
        onUpdated={onUpdated}
      />
    </>
  );
}

interface DialogProps {
  job: Job;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onUpdated: () => void;
}

// The change-policy modal. Mounted unconditionally by `PolicyBadge`
// so the badge button is the only thing the operator has to click;
// the dialog itself is a controlled component.
function PolicyDialog({ job, open, onOpenChange, onUpdated }: DialogProps) {
  const rpc = useRpc();
  const current = job.auto_bypass_policy;
  const initialKind = current?.type ?? NO_POLICY;
  const initialCustom = current?.type === "custom" ? current.comment : "";
  const [kind, setKind] = useState<string>(initialKind);
  const [customComment, setCustomComment] = useState<string>(initialCustom);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-seed the form whenever the modal opens. Without this a stale
  // dropdown state from a previous open would survive — a subtle
  // bug because the dialog stays mounted to keep the close animation
  // smooth.
  useEffect(() => {
    if (!open) return;
    setKind(current?.type ?? NO_POLICY);
    setCustomComment(current?.type === "custom" ? current.comment : "");
    setError(null);
  }, [open, current]);

  const customTrimmed = customComment.trim();
  const customValid = kind !== POLICY_CUSTOM || customTrimmed.length > 0;
  const editable = canEditPolicy(job.status);
  const nextPolicy: AutoBypassPolicy | null = (() => {
    if (kind === NO_POLICY) return null;
    if (kind === POLICY_CUSTOM) {
      return customTrimmed.length > 0
        ? { type: "custom", comment: customTrimmed }
        : null;
    }
    return { type: kind as PresetKind };
  })();

  // Equality test for the save-gate: don't ship a no-op RPC call
  // when the operator opened the modal, glanced, and closed it.
  // Custom comments require deep comparison; presets are pure
  // variants.
  const unchanged = (() => {
    if (!nextPolicy && !current) return true;
    if (!nextPolicy || !current) return false;
    if (nextPolicy.type !== current.type) return false;
    if (nextPolicy.type === "custom" && current.type === "custom") {
      return nextPolicy.comment === current.comment;
    }
    return true;
  })();

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await rpc.call("set_job_policy", {
        job_id: job.id,
        policy: nextPolicy,
      });
      onUpdated();
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Auto-bypass policy</DialogTitle>
          <DialogDescription>
            When a non-cap stage failure happens, the runtime
            advances to the next stage with the policy's guidance
            threaded into the prompt. Cap-breach failures always
            halt regardless.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="policy-kind">Policy</Label>
            <Select value={kind} onValueChange={setKind} disabled={!editable}>
              <SelectTrigger id="policy-kind">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NO_POLICY}>
                  None — halt on stage failure
                </SelectItem>
                {PRESETS.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.label}
                  </SelectItem>
                ))}
                <SelectItem value={POLICY_CUSTOM}>
                  Custom — write your own comment
                </SelectItem>
              </SelectContent>
            </Select>
            {kind === NO_POLICY && (
              <span className="text-muted-foreground text-[10px]">
                Default. A failed stage halts the job; you decide
                how to recover from the job page.
              </span>
            )}
            {kind !== NO_POLICY && kind !== POLICY_CUSTOM && (
              <span className="text-muted-foreground text-[10px]">
                {PRESETS.find((p) => p.id === kind)?.hint}
              </span>
            )}
            {kind === POLICY_CUSTOM && (
              <>
                <textarea
                  id="policy-custom"
                  value={customComment}
                  onChange={(e) => setCustomComment(e.target.value)}
                  rows={3}
                  placeholder="One paragraph of guidance threaded into the next stage when a failure is auto-bypassed."
                  aria-invalid={!customValid}
                  disabled={!editable}
                  className="border-input bg-background rounded-md border px-2 py-1.5 text-xs disabled:opacity-50"
                />
                <span className="text-muted-foreground text-[10px]">
                  Wrapped in an <code>Operator comment</code>
                  envelope and prepended to the next stage's prompt
                  verbatim.
                </span>
              </>
            )}
          </div>
          {!editable && (
            <div className="rounded border border-amber-500/40 bg-amber-500/10 px-2 py-1.5 text-[11px] text-amber-700 dark:text-amber-300">
              Policy is read-only while the job is{" "}
              <code>{job.status}</code>. Pause the job, change the
              policy, then resume.
            </div>
          )}
          {error && (
            <div className="text-destructive text-[11px]">{error}</div>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            cancel
          </Button>
          <Button
            onClick={() => void save()}
            disabled={saving || !editable || unchanged || !customValid}
          >
            {saving ? "saving…" : "save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
