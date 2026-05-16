import type {
  ScopePatchId,
  ScopePatchKind,
  ScopePatchTarget,
  StageId,
  ReviewId,
} from "@/lib/rpc";

// The fields the inbox renders per proposal. Mirrors the runtime's
// `Proposal` struct in `crates/codeless-runtime/src/scope_patch_queue.rs`
// minus the `span` (file-offset) and parser-only metadata. `rationale`
// and `body` are empty until the operator opens Edit (the SSE event
// carries the metadata fields but not the body — Stage 9's
// `list_proposed_patches` RPC will populate them; until then Edit
// starts from a stub block keyed by `id`/`target_path`).
export interface PatchProposal {
  id: ScopePatchId;
  review_id: ReviewId;
  stage_id: StageId;
  kind: ScopePatchKind;
  target: ScopePatchTarget;
  target_path: string;
  evidence_stage_id: StageId | null;
  has_predicate: boolean;
  rationale: string;
  body: string;
}

export type PatchResolution =
  | { kind: "approved"; commit_sha: string }
  | { kind: "rejected"; commit_sha: string }
  | { kind: "reverted"; commit_sha: string };

// Render a proposal as the markdown block that would appear in
// `DOCS/SCOPE-PROPOSED.md`. Matches the runtime's `Proposal::render`
// output so the edit round-trip stays loss-free: the parser on the
// runtime side re-reads this exact format and re-emits the same
// fields.
//
// Note: `kind` and `target` are rendered as the wire-format snake-case
// strings the parser accepts (`tighten`/`loosen`,
// `claude-md`/`job-scope-md`/`job-workflow-md`/`job-claude-md`).
export function renderProposalMarkdown(p: PatchProposal): string {
  const target = targetWire(p.target);
  const lines: string[] = [];
  lines.push(`## ${p.id}`);
  lines.push("");
  lines.push(`- kind: ${p.kind}`);
  lines.push(`- target: ${target}`);
  lines.push(`- target-path: ${p.target_path}`);
  lines.push(`- has_predicate: ${p.has_predicate}`);
  if (p.evidence_stage_id !== null) {
    lines.push(`- evidence_stage_id: ${p.evidence_stage_id}`);
  }
  lines.push("");
  lines.push("### Rationale");
  lines.push("");
  lines.push(p.rationale.trimEnd());
  lines.push("");
  lines.push("### Body");
  lines.push("");
  lines.push(p.body.trimEnd());
  return lines.join("\n") + "\n";
}

// Parse the inbox's CodeMirror buffer back into a `PatchProposal`.
// The runtime owns the authoritative parser (used by
// `edit_scope_patch` for validation); this client-side parse is a
// best-effort pre-flight that catches obvious mistakes (id changed,
// missing `### Rationale` heading) before the round-trip. The server
// will re-reject anything subtler.
//
// Returns `{ ok: false, error }` on shape problems. The caller surfaces
// the error inline beside the editor.
export function parseProposalMarkdown(
  text: string,
  expected: PatchProposal,
): { ok: true; proposal: PatchProposal } | { ok: false; error: string } {
  const lines = text.split("\n");
  if (lines.length === 0) return { ok: false, error: "empty patch buffer" };

  // First non-empty line must be `## <id>`.
  let i = 0;
  while (i < lines.length && lines[i].trim() === "") i += 1;
  const head = lines[i] ?? "";
  const headMatch = /^##\s+(\S+)\s*$/.exec(head);
  if (!headMatch) {
    return { ok: false, error: "first non-empty line is not `## <patch-id>`" };
  }
  const id = headMatch[1];
  if (id !== expected.id) {
    return {
      ok: false,
      error: `patch id changed from \`${expected.id}\` to \`${id}\` — Edit must preserve the id`,
    };
  }
  i += 1;

  // Skip blank lines, then read the bullet metadata block until the
  // first non-bullet line.
  while (i < lines.length && lines[i].trim() === "") i += 1;
  const meta = new Map<string, string>();
  while (i < lines.length && lines[i].startsWith("- ")) {
    const m = /^-\s*([A-Za-z_-]+)\s*:\s*(.*)\s*$/.exec(lines[i]);
    if (!m) {
      return { ok: false, error: `unparseable metadata line: ${lines[i]}` };
    }
    meta.set(m[1].toLowerCase(), m[2]);
    i += 1;
  }

  const kindStr = meta.get("kind");
  if (kindStr !== "tighten" && kindStr !== "loosen") {
    return {
      ok: false,
      error: `kind must be \`tighten\` or \`loosen\` (got \`${kindStr ?? "missing"}\`)`,
    };
  }
  const targetStr = meta.get("target");
  if (
    targetStr !== "claude-md" &&
    targetStr !== "job-scope-md" &&
    targetStr !== "job-workflow-md" &&
    targetStr !== "job-claude-md"
  ) {
    return {
      ok: false,
      error: `target must be one of claude-md, job-scope-md, job-workflow-md, job-claude-md (got \`${targetStr ?? "missing"}\`)`,
    };
  }
  const targetPath = meta.get("target-path") ?? meta.get("target_path");
  if (!targetPath) {
    return { ok: false, error: "missing `target-path`" };
  }
  const hasPredicateRaw =
    meta.get("has_predicate") ?? meta.get("has-predicate") ?? meta.get("predicate");
  const hasPredicate = hasPredicateRaw === "true";

  const evidenceRaw =
    meta.get("evidence_stage_id") ??
    meta.get("evidence-stage-id") ??
    meta.get("evidence");
  const evidenceStageId =
    evidenceRaw !== undefined && evidenceRaw !== "" ? (evidenceRaw as StageId) : null;

  // Expect `### Rationale`.
  while (i < lines.length && lines[i].trim() === "") i += 1;
  if (!lines[i] || !/^###\s+Rationale\s*$/i.test(lines[i])) {
    return { ok: false, error: "missing `### Rationale` heading after metadata" };
  }
  i += 1;
  while (i < lines.length && lines[i].trim() === "") i += 1;

  const rationaleLines: string[] = [];
  while (i < lines.length && !/^###\s+Body\s*$/i.test(lines[i])) {
    rationaleLines.push(lines[i]);
    i += 1;
  }
  if (i >= lines.length) {
    return { ok: false, error: "missing `### Body` heading after rationale" };
  }
  i += 1;
  while (i < lines.length && lines[i].trim() === "") i += 1;
  const bodyLines = lines.slice(i);

  return {
    ok: true,
    proposal: {
      id,
      review_id: expected.review_id,
      stage_id: expected.stage_id,
      kind: kindStr,
      target: targetStr,
      target_path: targetPath,
      evidence_stage_id: evidenceStageId,
      has_predicate: hasPredicate,
      rationale: rationaleLines.join("\n").trim(),
      body: bodyLines.join("\n").trimEnd(),
    },
  };
}

function targetWire(t: ScopePatchTarget): string {
  switch (t) {
    case "claude-md":
      return "claude-md";
    case "job-scope-md":
      return "job-scope-md";
    case "job-workflow-md":
      return "job-workflow-md";
    case "job-claude-md":
      return "job-claude-md";
  }
}
