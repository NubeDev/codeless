// Detach-workspace modal. Implements the two-shape flow from
// §"Detach modal" of DOCS/WORKSPACE-ATTACH.md:
//
//   * First click sends `detach_workspace` with the doc's default
//     `Refuse` policy. If the server returns no error, the row is
//     gone and we apply the detach to the store and close.
//
//   * If the server reports `RunningJobs { jobs }`, the modal expands
//     to the "Leave running" / "Stop them" radio. The user picks a
//     policy and resubmits with `Stop` or `LeaveRunning`. We never
//     silently stop jobs — the radio is the explicit choice the doc
//     requires.

import { useCallback, useEffect, useState } from "react";

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
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useRpc } from "@/lib/rpc";
import type {
  AttachedWorkspace,
  DetachPolicy,
  JobId,
} from "@/lib/rpc/wire";

import { useWorkspacesStore } from "./store";

interface DetachWorkspaceDialogProps {
  workspace: AttachedWorkspace | null;
  onClose(): void;
}

export function DetachWorkspaceDialog({
  workspace,
  onClose,
}: DetachWorkspaceDialogProps) {
  const rpc = useRpc();
  const [policy, setPolicy] = useState<"stop" | "leave-running">("stop");
  const [runningJobs, setRunningJobs] = useState<JobId[] | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!workspace) return;
    setPolicy("stop");
    setRunningJobs(null);
    setError(null);
    setSubmitting(false);
  }, [workspace]);

  const submit = useCallback(
    async (onJobs: DetachPolicy) => {
      if (!workspace) return;
      setSubmitting(true);
      setError(null);
      try {
        await rpc.call("detach_workspace", {
          repo_id: workspace.repo_id,
          on_running_jobs: onJobs,
        });
        useWorkspacesStore.getState().applyDetached(workspace.repo_id);
        onClose();
      } catch (e) {
        const parsed = parseRunningJobs(e);
        if (parsed) {
          setRunningJobs(parsed);
          setError(null);
        } else {
          setError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        setSubmitting(false);
      }
    },
    [onClose, rpc, workspace],
  );

  const open = workspace !== null;
  const needsChoice = runningJobs !== null && runningJobs.length > 0;

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
    >
      <DialogContent data-testid="detach-workspace-dialog">
        <DialogHeader>
          <DialogTitle>
            Detach {workspace ? `\`${workspace.repo_name}\`` : "workspace"}
          </DialogTitle>
          {!needsChoice ? (
            <DialogDescription>
              The editor will lose access to this directory. The workspace
              row stays registered and can be re-attached later.
            </DialogDescription>
          ) : null}
        </DialogHeader>

        {needsChoice ? (
          <div className="flex flex-col gap-4">
            <div>
              <p className="text-sm font-medium">
                The following jobs are running against this workspace:
              </p>
              <ul
                className="mt-1 list-disc pl-5 text-xs text-muted-foreground"
                data-testid="detach-running-jobs"
              >
                {runningJobs!.map((id) => (
                  <li key={id}>{id}</li>
                ))}
              </ul>
            </div>
            <RadioGroup
              value={policy}
              onValueChange={(v) => setPolicy(v as "stop" | "leave-running")}
              data-testid="detach-policy-group"
            >
              <Label className="flex items-start gap-2 text-xs font-normal">
                <RadioGroupItem
                  value="leave-running"
                  data-testid="detach-policy-leave-running"
                  className="mt-0.5"
                />
                <span>
                  <span className="font-medium">Leave running.</span> The
                  runner keeps writing in the worktree, but the job page can't
                  show file diffs until you re-attach.
                </span>
              </Label>
              <Label className="flex items-start gap-2 text-xs font-normal">
                <RadioGroupItem
                  value="stop"
                  data-testid="detach-policy-stop"
                  className="mt-0.5"
                />
                <span>
                  <span className="font-medium">Stop them.</span> Cancel each
                  running job before detaching.
                </span>
              </Label>
            </RadioGroup>
          </div>
        ) : null}

        {error ? (
          <p
            className="text-xs text-destructive"
            data-testid="detach-ws-error"
          >
            {error}
          </p>
        ) : null}

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={onClose}
            disabled={submitting}
            data-testid="detach-ws-cancel-button"
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={() => submit(needsChoice ? policy : "refuse")}
            disabled={submitting}
            data-testid="detach-ws-submit-button"
          >
            {submitting ? "Detaching…" : "Detach"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// `detach_workspace` surfaces the `RunningJobs` variant of
// `WorkspaceError` as an RpcError payload (the wire string-tags the
// variant; specta serialises the kebab-case form). We sniff for it
// in the thrown error so the modal can flip to the radio variant
// without parsing a generic `Conflict` message.
function parseRunningJobs(e: unknown): JobId[] | null {
  if (e === null || typeof e !== "object") return null;
  const msg = e instanceof Error ? e.message : String(e);
  const tag = "running-jobs";
  if (!msg.includes(tag)) return null;
  // Best-effort extraction of any `jobs: [...]` array from the
  // serialised error. The runtime tests pin the exact shape; here
  // we fall back to a non-empty marker so the modal still flips
  // even if the message format drifts.
  const m = msg.match(/jobs"?\s*:\s*\[([^\]]*)\]/);
  if (!m) return ["job"] as JobId[];
  const inner = m[1]!.trim();
  if (inner === "") return [];
  return inner
    .split(",")
    .map((s) => s.trim().replace(/^["']|["']$/g, ""))
    .filter(Boolean) as JobId[];
}
