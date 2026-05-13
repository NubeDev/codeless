import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useRpc, type JobId } from "@/lib/rpc";

interface Props {
  jobId: JobId;
  // The job's submitted prompt, if any. Folded into the seeded
  // template's `goal:` field so the user does not have to retype
  // intent that's already on the job row.
  prompt: string | null;
  // Called once `update_job_template` succeeds so the parent re-fetches
  // `list_job_files` and the Spec pane swaps to the real two-pane
  // editor on the freshly-created template.yaml.
  onPromoted: () => void;
}

// Empty state for prompt-only jobs in the Spec pane. JOB-MODEL.md
// treats `.codeless/jobs/<name>/template.yaml` as the source of truth
// for the iterate loop; a prompt-only job has nothing to edit until
// it is promoted to a template. This affordance does that promotion
// in one click via the existing `update_job_template` RPC, which
// validates the YAML, writes the file in the source repo, and commits
// it. After success the Spec pane reloads into its normal two-pane
// layout with the new template.yaml selected.
//
// Why ask for a name: job-dirs are addressed by name (a folder under
// `.codeless/jobs/<name>/`), and renames are refused once the
// directory exists. Picking a stable name up-front is the choice the
// user has to make; we suggest one based on the branch slug so the
// default is rarely wrong.
export function PromoteToTemplate({ jobId, prompt, onPromoted }: Props) {
  const rpc = useRpc();
  const [name, setName] = useState("");
  const [stages, setStages] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const goalDefault = useMemo(() => firstSentence(prompt ?? "") || "", [prompt]);
  const [goal, setGoal] = useState(goalDefault);

  const yamlPreview = useMemo(
    () => buildYaml(name.trim(), goal.trim(), splitStages(stages)),
    [name, goal, stages],
  );

  const promote = async () => {
    setBusy(true);
    setError(null);
    try {
      await rpc.call("update_job_template", {
        job_id: jobId,
        template_yaml: yamlPreview,
      });
      onPromoted();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = name.trim().length > 0 && goal.trim().length > 0 && splitStages(stages).length > 0;

  return (
    <div className="mx-auto flex h-full max-w-2xl flex-col gap-4 overflow-y-auto p-6 text-sm">
      <div className="space-y-1">
        <h2 className="text-base font-medium">No template yet</h2>
        <p className="text-muted-foreground text-xs leading-snug">
          This job was submitted as a single prompt. The Spec pane edits a{" "}
          <code>template.yaml</code> in <code>.codeless/jobs/&lt;name&gt;/</code>{" "}
          (see <code>JOB-MODEL.md</code>). Promote the job to a template to
          start iterating on the spec — the goal seeds from the current
          prompt; you add stages.
        </p>
      </div>

      <div className="grid gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="promote-name">Name</Label>
          <Input
            id="promote-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="user-profile"
            spellCheck={false}
          />
          <span className="text-muted-foreground text-[10px]">
            Folder name under <code>.codeless/jobs/</code>. Cannot be renamed
            later.
          </span>
        </div>

        <div className="grid gap-1.5">
          <Label htmlFor="promote-goal">Goal</Label>
          <textarea
            id="promote-goal"
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            rows={3}
            className="border-input bg-background focus-visible:ring-ring rounded-md border px-3 py-2 text-sm focus-visible:ring-2 focus-visible:outline-none"
            placeholder="One paragraph: what this job accomplishes."
            spellCheck={false}
          />
        </div>

        <div className="grid gap-1.5">
          <Label htmlFor="promote-stages">Stages</Label>
          <textarea
            id="promote-stages"
            value={stages}
            onChange={(e) => setStages(e.target.value)}
            rows={5}
            className="border-input bg-background focus-visible:ring-ring rounded-md border px-3 py-2 font-mono text-xs focus-visible:ring-2 focus-visible:outline-none"
            placeholder={"one stage title per line\nREVIEW prefix to mark a review gate\n…"}
            spellCheck={false}
          />
          <span className="text-muted-foreground text-[10px]">
            One title per line. Lines starting with <code>REVIEW</code> halt
            the loop and wait for a human.
          </span>
        </div>
      </div>

      <div className="space-y-1">
        <span className="text-muted-foreground text-[10px] uppercase tracking-wide">
          Will commit
        </span>
        <pre className="bg-muted/40 max-h-48 overflow-auto rounded p-2 font-mono text-[11px]">
          {yamlPreview}
        </pre>
      </div>

      {error && <div className="text-destructive text-xs">{error}</div>}

      <div className="flex items-center justify-end gap-2">
        <Button onClick={promote} disabled={busy || !ready}>
          {busy ? "promoting…" : "Promote to template"}
        </Button>
      </div>
    </div>
  );
}

// Trim a freeform prompt to a single short sentence so the seeded
// `goal:` line is meaningful without dumping multi-paragraph runner
// instructions into the YAML. Falls back to the truncated full text
// when no sentence-ending punctuation is found.
function firstSentence(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length === 0) return "";
  const m = trimmed.match(/^(.{8,200}?[.!?])(\s|$)/);
  if (m) return m[1];
  return trimmed.length > 160 ? `${trimmed.slice(0, 157)}…` : trimmed;
}

function splitStages(text: string): string[] {
  return text
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

// Hand-build the YAML rather than reaching for a serializer: the
// shape is tiny, the values are user-controlled (so we quote the goal
// to dodge YAML's flow indicators), and the on-disk format is more
// readable when we control the layout. The runtime parses with
// serde_yaml, which accepts this verbatim.
function buildYaml(name: string, goal: string, stages: string[]): string {
  const safeName = name || "<name>";
  const safeGoal = quoteYamlString(goal || "<goal>");
  const stageLines =
    stages.length === 0
      ? "  - <add a stage title>"
      : stages.map((s) => `  - ${quoteYamlString(s)}`).join("\n");
  return `name: ${safeName}\ngoal: ${safeGoal}\nstages:\n${stageLines}\n`;
}

// YAML string quoting that handles the values we generate: never
// contains literal double-quote characters in a goal/stage title in
// practice, but escape them just in case so the parsed YAML matches
// what the user typed.
function quoteYamlString(s: string): string {
  if (s.length === 0) return '""';
  // Plain-style scalars are safe when they don't start with an
  // indicator and don't contain `:` followed by space, `#`, etc. The
  // double-quoted style is always safe and unambiguous; cost is
  // visual noise. Use plain only when it's clearly clean.
  const needsQuote =
    /^[!&*\-?|>%@`,\[\]{}#"'\s]/.test(s) ||
    /:\s|\s#|^\s|\s$/.test(s) ||
    s.includes("\n");
  if (!needsQuote) return s;
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}
