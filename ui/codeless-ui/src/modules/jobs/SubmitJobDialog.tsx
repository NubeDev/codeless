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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useRpc, type Repo } from "@/lib/rpc";

interface Props {
  repo: Repo;
  // Defaults pulled from the repo row; tunable in the form.
  trigger?: React.ReactNode;
}

export function SubmitJobDialog({ repo, trigger }: Props) {
  const rpc = useRpc();
  const [open, setOpen] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [branch, setBranch] = useState(repo.default_branch);
  const [runner, setRunner] = useState(repo.default_runner ?? "claude");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setSubmitting(true);
    setError(null);
    try {
      await rpc.call("submit_job", {
        repo_id: repo.id,
        prompt: prompt || null,
        template_yaml: null,
        runner,
        branch,
        // Conservative defaults until we have a settings surface.
        cost_cap_cents: 500,
        wall_clock_cap_ms: 30 * 60 * 1000,
      });
      setOpen(false);
      setPrompt("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
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
            Queue a new job in this repo. The core will provision a worktree
            and run it on the chosen runner.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="prompt">Prompt</Label>
            <Textarea
              id="prompt"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Describe what the agent should do…"
              rows={5}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="branch">Branch</Label>
              <Input
                id="branch"
                value={branch}
                onChange={(e) => setBranch(e.target.value)}
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="runner">Runner</Label>
              <Input
                id="runner"
                value={runner}
                onChange={(e) => setRunner(e.target.value)}
              />
            </div>
          </div>
          {error && <div className="text-destructive text-xs">{error}</div>}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            cancel
          </Button>
          <Button onClick={submit} disabled={submitting}>
            {submitting ? "submitting…" : "submit"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
