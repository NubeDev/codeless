import { useState } from "react";

import { Button } from "@/components/ui/button";
import { useRpc, type Repo } from "@/lib/rpc";

interface Props {
  repo: Repo;
}

// One-click affordance that submits a `mock` runner job with a canned
// prompt and a fresh branch. The detailed-form path is `SubmitJobDialog`;
// this exists so a demo viewer can watch the click-to-stream loop close
// without filling in five fields. Disabled while a submit is in flight
// to stop double-clicks from queuing duplicates.
export function RunMockJobButton({ repo }: Props) {
  const rpc = useRpc();
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setSubmitting(true);
    setError(null);
    try {
      await rpc.call("submit_job", {
        repo_id: repo.id,
        prompt: "demo: run a mock job and stream its events",
        template_yaml: null,
        runner: "mock",
        branch: `codeless/mock-${freshSuffix()}`,
        cost_cap_cents: 500,
        wall_clock_cap_ms: 30 * 60 * 1000,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="flex flex-col items-end gap-0.5">
      <Button
        size="sm"
        variant="secondary"
        onClick={run}
        disabled={submitting}
        title="Queue a fresh mock-runner job and stream its events"
      >
        {submitting ? "starting…" : "run mock job"}
      </Button>
      {error && (
        <span className="text-destructive text-[10px]" title={error}>
          {error}
        </span>
      )}
    </div>
  );
}

function freshSuffix(): string {
  return Math.floor(Math.random() * 36 ** 6)
    .toString(36)
    .padStart(6, "0");
}
