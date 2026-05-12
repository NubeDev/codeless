import { useState } from "react";

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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { navigate } from "@/lib/route";
import { useRpc, type Repo } from "@/lib/rpc";

interface Props {
  repos: Repo[];
}

// Top-level "new job" entry that does not require the user to first
// scroll to a specific repo's card. The per-repo `SubmitJobDialog`
// still exists for the full form (branch, runner, caps); this dialog
// is deliberately minimal: goal + repo, hard-coded mock runner, fresh
// branch, $5 cost cap, 30m wall-clock cap. Real planner integration
// is later (SCOPE.md Phase 4 "Planner"); the user-visible surface is
// what we're after now. Navigates to the new job's detail on success
// so the loop closes — click, see the run start streaming.
export function NewJobDialog({ repos }: Props) {
  const rpc = useRpc();
  const [open, setOpen] = useState(false);
  const [goal, setGoal] = useState("");
  const [repoId, setRepoId] = useState<string | null>(
    repos[0]?.id ?? null,
  );
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!repoId) {
      setError("Pick a repo");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const job = await rpc.call("submit_job", {
        repo_id: repoId,
        prompt: goal.trim() || "demo: ad-hoc mock job",
        template_yaml: null,
        runner: "mock",
        branch: `codeless/job-${freshSuffix()}`,
        cost_cap_cents: 500,
        wall_clock_cap_ms: 30 * 60 * 1000,
      });
      setOpen(false);
      setGoal("");
      navigate(`/jobs/${job.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm" disabled={repos.length === 0}>
          New job
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New job</DialogTitle>
          <DialogDescription>
            Queue a mock-runner job against any registered repo. The full
            form (branch, runner, caps) lives on each repo's card.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="new-job-repo">Repo</Label>
            <Select
              value={repoId ?? ""}
              onValueChange={(v) => setRepoId(v)}
              disabled={repos.length === 0}
            >
              <SelectTrigger id="new-job-repo">
                <SelectValue placeholder="Select a repo" />
              </SelectTrigger>
              <SelectContent>
                {repos.map((r) => (
                  <SelectItem key={r.id} value={r.id}>
                    {r.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="new-job-goal">Goal</Label>
            <Textarea
              id="new-job-goal"
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder="What should the agent do? (defaults to a mock demo run)"
              rows={4}
            />
          </div>
          {error && <div className="text-destructive text-xs">{error}</div>}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            cancel
          </Button>
          <Button onClick={submit} disabled={submitting || !repoId}>
            {submitting ? "queuing…" : "queue mock job"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function freshSuffix(): string {
  return Math.floor(Math.random() * 36 ** 6)
    .toString(36)
    .padStart(6, "0");
}
