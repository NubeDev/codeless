// Minimal best-effort parser for the read-only template summary in
// the Spec pane. The runtime parses authoritatively with serde_yaml
// on save; this exists only so the summary view has fields to show.
//
// Failure mode is intentional: if the YAML is unusual, the parsed
// struct comes back empty / partial, the summary degrades, and the
// user falls through to the raw YAML editor where they can fix it.
// We never reject content the runtime would accept; we never accept
// content the runtime would reject (validation is server-side).

export interface ParsedTemplate {
  name: string | null;
  goal: string | null;
  stages: ParsedStage[];
  // True when the YAML had a `docs:` block at all. The summary
  // surfaces "auto-discover" vs "pinned" — pinned means the user
  // explicitly listed the docs and order.
  docsPinned: boolean;
  pinnedDocs: string[];
}

export interface ParsedStage {
  title: string;
  isReview: boolean;
  // Per-stage docs explicitly listed under this stage. `null` means
  // the stage opted out (no `docs:` key — the template's global docs
  // are still applied). `[]` is explicit empty (still no per-stage
  // docs, but the user signalled the choice).
  docs: string[] | null;
}

export function parseTemplate(yaml: string): ParsedTemplate {
  const name = scalarValue(yaml, "name");
  const goal = scalarValue(yaml, "goal");
  const docsBlock = listBlock(yaml, "docs");
  const stages = parseStages(yaml);
  return {
    name,
    goal,
    stages,
    docsPinned: docsBlock !== null,
    pinnedDocs: docsBlock ?? [],
  };
}

// `key: value` on a single line. Strips inline comments and quotes.
// Multi-line scalars are not supported — they're rare in template.yaml
// and the raw editor handles them when they exist.
function scalarValue(yaml: string, key: string): string | null {
  const re = new RegExp(`^\\s*${escapeRe(key)}\\s*:\\s*(.+)$`, "m");
  const m = re.exec(yaml);
  if (!m) return null;
  let v = m[1].trim();
  // Strip a trailing comment, ignoring `#` inside a quoted string.
  if (!v.startsWith('"') && !v.startsWith("'")) {
    const hashIdx = v.indexOf(" #");
    if (hashIdx >= 0) v = v.slice(0, hashIdx).trim();
  }
  return unquote(v);
}

function listBlock(yaml: string, key: string): string[] | null {
  const headerRe = new RegExp(`^(\\s*)${escapeRe(key)}\\s*:\\s*$`, "m");
  const m = headerRe.exec(yaml);
  if (!m) return null;
  const baseIndent = m[1].length;
  const after = yaml.slice((m.index ?? 0) + m[0].length + 1);
  const lines = after.split("\n");
  const items: string[] = [];
  for (const raw of lines) {
    if (raw.trim() === "") continue;
    const indent = raw.match(/^\s*/)?.[0].length ?? 0;
    if (indent <= baseIndent) break;
    const bullet = raw.match(/^\s*-\s+(.*)$/);
    if (bullet) {
      items.push(unquote(bullet[1].trim()));
      continue;
    }
    // Non-bullet child line under `docs:` is unusual; skip rather
    // than guess. The raw editor will show what's actually there.
    break;
  }
  return items;
}

function parseStages(yaml: string): ParsedStage[] {
  const headerRe = /^(\s*)stages\s*:\s*$/m;
  const m = headerRe.exec(yaml);
  if (!m) return [];
  const baseIndent = m[1].length;
  const after = yaml.slice((m.index ?? 0) + m[0].length + 1);
  const lines = after.split("\n");
  const stages: ParsedStage[] = [];
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    if (raw.trim() === "") continue;
    const indent = raw.match(/^\s*/)?.[0].length ?? 0;
    if (indent <= baseIndent) break;
    const bullet = raw.match(/^\s*-\s+(.*)$/);
    if (!bullet) continue;
    const head = bullet[1].trim();
    // Stages can be either bare strings (`- "do thing"`) or mappings
    // (`- title: "do thing"\n    review: true\n    docs: [...]`). For
    // the summary we only need the title; the runtime handles the
    // mapping shape on save.
    if (head.startsWith("title:")) {
      const title = unquote(head.slice("title:".length).trim());
      // Look ahead through the mapping body for `review: true` and
      // any `docs:` block. We only consume continuation lines whose
      // indent is greater than the bullet's.
      let isReview = false;
      let stageDocs: string[] | null = null;
      let j = i + 1;
      while (j < lines.length) {
        const ln = lines[j];
        if (ln.trim() === "") {
          j++;
          continue;
        }
        const ind = ln.match(/^\s*/)?.[0].length ?? 0;
        if (ind <= indent) break;
        if (/^\s*review\s*:\s*true\s*$/.test(ln)) isReview = true;
        const docsMatch = ln.match(/^\s*docs\s*:\s*(.*)$/);
        if (docsMatch) {
          const inline = docsMatch[1].trim();
          if (inline === "[]") {
            stageDocs = [];
          } else if (inline.startsWith("[") && inline.endsWith("]")) {
            // Flow-style: `docs: [a.md, b.md]`.
            stageDocs = inline
              .slice(1, -1)
              .split(",")
              .map((s) => unquote(s.trim()))
              .filter((s) => s.length > 0);
          } else if (inline === "") {
            // Block-style: collect bullets that follow.
            stageDocs = [];
            let k = j + 1;
            while (k < lines.length) {
              const child = lines[k];
              if (child.trim() === "") {
                k++;
                continue;
              }
              const cind = child.match(/^\s*/)?.[0].length ?? 0;
              if (cind <= ind) break;
              const bullet = child.match(/^\s*-\s+(.*)$/);
              if (bullet) stageDocs.push(unquote(bullet[1].trim()));
              k++;
            }
          }
        }
        j++;
      }
      stages.push({
        title: stripReview(title),
        isReview: isReview || isReviewPrefix(title),
        docs: stageDocs,
      });
      continue;
    }
    const cleaned = unquote(head);
    stages.push({
      title: stripReview(cleaned),
      isReview: isReviewPrefix(cleaned),
      docs: null,
    });
  }
  return stages;
}

function isReviewPrefix(title: string): boolean {
  return /^REVIEW\b/.test(title);
}

function stripReview(title: string): string {
  return title.replace(/^REVIEW\s+/, "");
}

function unquote(s: string): string {
  if (s.length >= 2) {
    if (s.startsWith('"') && s.endsWith('"')) {
      return s.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, "\\");
    }
    if (s.startsWith("'") && s.endsWith("'")) {
      return s.slice(1, -1).replace(/''/g, "'");
    }
  }
  return s;
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
