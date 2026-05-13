// Surgical mutators for `template.yaml`. The Spec pane's summary
// view shows fields read-only; the small picker UIs (global docs
// order, per-stage docs) need to change a single block of the YAML
// without touching anything else (formatting, comments, key order).
//
// We deliberately do not parse → object → re-serialise. That round
// trip would normalise the user's hand-written YAML in ways they did
// not ask for (drop comments, change quoting, reorder keys, collapse
// flow style). Instead, each mutator finds the target block by line
// pattern and rewrites only those lines, preserving everything else.
//
// The runtime parses the result with serde_yaml authoritatively on
// save. If a mutator produces something that does not parse, the RPC
// returns InvalidArgument and the user sees the error inline. We
// optimise for "common edits don't churn the file"; we do not
// optimise for "always emits valid YAML no matter what" — that's the
// runtime's job.

// Replace (or insert, when missing) the top-level `docs:` block with
// the supplied ordering. `null` removes the block entirely so the
// template falls back to auto-discover. Empty array writes
// `docs: []` — explicit empty, distinct from auto-discover.
export function setGlobalDocs(
  yaml: string,
  docs: string[] | null,
): string {
  const removed = removeTopLevelBlock(yaml, "docs");
  if (docs === null) return removed;
  const block = renderDocsBlock(docs, 0);
  return insertBeforeStages(removed, block);
}

// Replace the per-stage `docs:` list on the stage at the given
// 0-based ordinal. `null` removes that stage's `docs:` entirely.
// Empty array writes `docs: []`.
//
// Stages can be either bare-string form (`- "do thing"`) or mapping
// form (`- title: ...\n  docs: [...]\n`). To attach docs we always
// promote to mapping form; the title comes through unchanged either
// way. This is a one-way promotion (we never demote back to bare
// strings) which is fine: the runtime accepts both shapes
// interchangeably.
export function setStageDocs(
  yaml: string,
  ordinal: number,
  docs: string[] | null,
): string {
  const stagesBlock = locateStagesBlock(yaml);
  if (!stagesBlock) return yaml;
  const stageRanges = locateStageItems(yaml, stagesBlock);
  if (ordinal < 0 || ordinal >= stageRanges.length) return yaml;

  const target = stageRanges[ordinal];
  const stageText = yaml.slice(target.start, target.end);
  const indent = " ".repeat(stagesBlock.indent + 2);
  const itemIndent = " ".repeat(stagesBlock.indent + 4);
  const firstLine = stageText.split("\n")[0];
  const dashMatch = firstLine.match(/^(\s*-\s+)(.*)$/);
  if (!dashMatch) return yaml;
  const head = dashMatch[2].trim();

  // Build the replacement stage text in mapping form.
  let mappedTitle: string;
  let extras: string[] = [];
  if (head.startsWith("title:") || head.startsWith("review:") || head.startsWith("docs:")) {
    // Already mapping form — preserve every line except the existing
    // `docs:` (single-line or block form).
    const innerLines = stageText.split("\n");
    const titleLine = innerLines[0];
    mappedTitle = titleLine;
    const rest = innerLines.slice(1);
    extras = stripDocsLines(rest, stagesBlock.indent + 4);
  } else {
    // Bare string form — promote to mapping. Quote the title
    // defensively (it survives unchanged inside double quotes).
    const titleLiteral = quoteScalar(head);
    mappedTitle = `${dashMatch[1]}title: ${titleLiteral}`;
  }

  const docsBlock =
    docs === null
      ? ""
      : docs.length === 0
        ? `${itemIndent}docs: []\n`
        : `${itemIndent}docs:\n${docs
            .map((d) => `${itemIndent}  - ${quoteScalar(d)}`)
            .join("\n")}\n`;

  // Reassemble: title line, then extras (preserving review: etc),
  // then docs block. Trailing newline behaviour matches the input —
  // each stage range ends with its newline so we do too.
  const body =
    mappedTitle +
    "\n" +
    (extras.length > 0 ? extras.join("\n") + "\n" : "") +
    docsBlock;
  void indent;

  return yaml.slice(0, target.start) + body + yaml.slice(target.end);
}

interface BlockLoc {
  // Line index where the `key:` header sits.
  headerLine: number;
  // Indent of the header (spaces before the key).
  indent: number;
  // Char index of header start (for slicing).
  headerStart: number;
  // Char index right after the block ends (for slicing).
  blockEnd: number;
}

function locateBlock(yaml: string, key: string): BlockLoc | null {
  const lines = yaml.split("\n");
  const reHeader = new RegExp(`^(\\s*)${escapeRe(key)}\\s*:\\s*(.*)$`);
  let cursor = 0;
  for (let i = 0; i < lines.length; i++) {
    const m = reHeader.exec(lines[i]);
    if (m) {
      const indent = m[1].length;
      // Walk forward over the block body — every continuation line
      // has indent > header.indent. Empty lines are part of the
      // block until the next non-empty line at indent <= header.
      let j = i + 1;
      while (j < lines.length) {
        const ln = lines[j];
        if (ln.trim() === "") {
          j++;
          continue;
        }
        const ind = ln.match(/^\s*/)?.[0].length ?? 0;
        if (ind <= indent) break;
        j++;
      }
      // Compute char ranges by accumulating prior line lengths.
      let headerStart = 0;
      for (let k = 0; k < i; k++) headerStart += lines[k].length + 1;
      let blockEnd = headerStart + lines[i].length + 1;
      for (let k = i + 1; k < j; k++) blockEnd += lines[k].length + 1;
      return { headerLine: i, indent, headerStart, blockEnd };
    }
    cursor += lines[i].length + 1;
    void cursor;
  }
  return null;
}

// Remove a top-level block (its header line and every following line
// that belongs to it). Returns the YAML with the block stripped.
function removeTopLevelBlock(yaml: string, key: string): string {
  const loc = locateBlock(yaml, key);
  if (!loc || loc.indent !== 0) return yaml;
  return yaml.slice(0, loc.headerStart) + yaml.slice(loc.blockEnd);
}

// Insert `block` (already newline-terminated) before the top-level
// `stages:` block, or at the end of the document if there is none.
// The `docs:` block must precede `stages:` to read naturally; the
// runtime accepts either order.
function insertBeforeStages(yaml: string, block: string): string {
  const stagesLoc = locateBlock(yaml, "stages");
  if (stagesLoc && stagesLoc.indent === 0) {
    return yaml.slice(0, stagesLoc.headerStart) + block + yaml.slice(stagesLoc.headerStart);
  }
  // No stages: block; append at end with a separating newline if
  // needed.
  const sep = yaml.endsWith("\n") || yaml.length === 0 ? "" : "\n";
  return yaml + sep + block;
}

function renderDocsBlock(docs: string[], baseIndent: number): string {
  const indent = " ".repeat(baseIndent);
  if (docs.length === 0) return `${indent}docs: []\n`;
  const lines = docs.map((d) => `${indent}  - ${quoteScalar(d)}`);
  return `${indent}docs:\n${lines.join("\n")}\n`;
}

interface StageBlockLoc {
  headerLine: number;
  indent: number;
  bodyStartChar: number;
  bodyEndChar: number;
}

// Find the `stages:` header and return the byte range of its body
// (lines under it).
function locateStagesBlock(yaml: string): StageBlockLoc | null {
  const loc = locateBlock(yaml, "stages");
  if (!loc || loc.indent !== 0) return null;
  // Body starts on the line *after* the header.
  const lines = yaml.split("\n");
  let headerLineEnd = loc.headerStart + lines[loc.headerLine].length + 1;
  return {
    headerLine: loc.headerLine,
    indent: loc.indent,
    bodyStartChar: headerLineEnd,
    bodyEndChar: loc.blockEnd,
  };
}

interface StageRange {
  start: number;
  end: number;
}

// Split the stages-block body into one range per top-level stage
// item. A stage item is a `- …` bullet at the *first* indent we
// encounter inside the block (typically `stages:` at col 0 →
// `  - …` at col 2). Bullets at deeper indents (e.g. `      - SCOPE.md`
// inside a stage's `docs:` block) are continuation lines, NOT new
// stages — without this distinction, stripping/replacing a stage's
// docs would split that stage in two and silently corrupt the YAML.
function locateStageItems(yaml: string, block: StageBlockLoc): StageRange[] {
  const body = yaml.slice(block.bodyStartChar, block.bodyEndChar);
  const lines = body.split("\n");
  const items: StageRange[] = [];

  // First pass: discover the canonical stage-bullet indent — the
  // smallest indent at which a `- ` bullet appears inside the block.
  // Locking to this indent prevents nested docs bullets from being
  // mis-classified as new stages.
  let stageBulletIndent: number | null = null;
  for (const ln of lines) {
    if (ln.trim() === "") continue;
    const ind = ln.match(/^\s*/)?.[0].length ?? 0;
    if (ind <= block.indent) continue;
    if (!/^\s*-\s+/.test(ln)) continue;
    if (stageBulletIndent === null || ind < stageBulletIndent) {
      stageBulletIndent = ind;
    }
  }
  if (stageBulletIndent === null) return items;

  let cursorChar = block.bodyStartChar;
  let currentStart: number | null = null;
  const flush = (endChar: number) => {
    if (currentStart !== null) {
      items.push({ start: currentStart, end: endChar });
      currentStart = null;
    }
  };
  for (let i = 0; i < lines.length; i++) {
    const ln = lines[i];
    const lnLen = ln.length + (i < lines.length - 1 ? 1 : 0);
    if (ln.trim() === "") {
      cursorChar += lnLen;
      continue;
    }
    const ind = ln.match(/^\s*/)?.[0].length ?? 0;
    const isBullet = /^\s*-\s+/.test(ln);
    // Only bullets at exactly `stageBulletIndent` start a new stage.
    if (isBullet && ind === stageBulletIndent) {
      flush(cursorChar);
      currentStart = cursorChar;
      cursorChar += lnLen;
      continue;
    }
    // Anything indented deeper than the stage bullet is a continuation
    // line of the current stage (mapping fields, nested docs lists,
    // anything the user hand-wrote).
    if (currentStart !== null && ind > stageBulletIndent) {
      cursorChar += lnLen;
      continue;
    }
    // A non-bullet line at or shallower than the stage indent ends
    // the current stage; skip it (it shouldn't appear in well-formed
    // YAML inside the stages block, but be defensive).
    flush(cursorChar);
    cursorChar += lnLen;
  }
  flush(cursorChar);
  return items;
}

// Strip an existing `docs:` line (single-line or block form) from
// the per-stage continuation lines. Used when we're about to
// re-insert a fresh docs block.
function stripDocsLines(restLines: string[], expectedIndent: number): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < restLines.length) {
    const ln = restLines[i];
    const ind = ln.match(/^\s*/)?.[0].length ?? 0;
    if (ind === expectedIndent && /^\s*docs\s*:/.test(ln)) {
      // Skip the `docs:` line and any indented children.
      i++;
      while (i < restLines.length) {
        const child = restLines[i];
        const childInd = child.match(/^\s*/)?.[0].length ?? 0;
        if (child.trim() === "") {
          i++;
          continue;
        }
        if (childInd > expectedIndent) {
          i++;
          continue;
        }
        break;
      }
      continue;
    }
    if (ln.trim() === "") {
      i++;
      continue;
    }
    out.push(ln);
    i++;
  }
  return out;
}

function quoteScalar(s: string): string {
  if (s.length === 0) return '""';
  const needsQuote =
    /^[!&*\-?|>%@`,\[\]{}#"'\s]/.test(s) ||
    /:\s|\s#|^\s|\s$/.test(s) ||
    s.includes("\n");
  if (!needsQuote) return s;
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
