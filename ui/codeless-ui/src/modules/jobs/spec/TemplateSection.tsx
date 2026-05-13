import { useEffect, useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useRpc, type JobId } from "@/lib/rpc";

import { InlineEditor } from "./InlineEditor";
import { parseTemplate, type ParsedStage } from "./parseTemplate";
import { setGlobalDocs, setStageDocs } from "./mutateTemplate";

// The template.yaml section. Two modes:
//   1. Summary (default) — read-only, parsed view of name/goal/stages
//      so the user can scan the spec without dropping into the YAML.
//      Also where [Edit YAML] toggles the editor on.
//   2. Editor — raw CodeMirror in YAML mode. The file IS the editor.
//      No form; no per-stage docs picker; no auto/serialise round-trip.
//      Save calls update_job_template; the runtime is the YAML
//      validator and the source of truth for what's on disk.
//
// This split is the load-bearing decision: the user sees structure
// when they want to scan, raw text when they want to change. We never
// show both, and we never try to be clever about merging form edits
// into raw edits.
export function TemplateSection({
  jobId,
  templateYaml,
  availableDocs,
  onSaved,
}: {
  jobId: JobId;
  templateYaml: string | null;
  // Names of `.md` files in the job dir, used to populate the
  // global-docs and per-stage-docs pickers. Excludes `template.yaml`.
  // Empty list ⇒ pickers tell the user to add a doc first.
  availableDocs: string[];
  onSaved: () => void;
}) {
  const rpc = useRpc();
  const [editing, setEditing] = useState(false);
  const [buffer, setBuffer] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Local mirror of the on-disk YAML, used by the picker UIs as the
  // base for surgical mutations. The parent's `templateYaml` prop
  // comes from `useJob`, which is fetch-once — without this mirror,
  // ticking a second checkbox would compose against a stale snapshot
  // and silently overwrite the first tick. We update `localYaml` the
  // moment a picker save returns OK; `templateYaml` catches up later
  // when the parent re-fetches the job (or stays slightly behind, but
  // that's invisible because the picker reads localYaml).
  const [localYaml, setLocalYaml] = useState<string>(templateYaml ?? "");

  // Re-seed the buffer + the local mirror when the on-disk YAML
  // changes. The dirty check is "buffer !== templateYaml"; once we
  // save and the parent refreshes templateYaml to match the buffer,
  // both go quiet.
  useEffect(() => {
    setBuffer(templateYaml ?? "");
    setLocalYaml(templateYaml ?? "");
    setError(null);
  }, [templateYaml]);

  const dirty = editing && buffer !== (templateYaml ?? "");
  const parsed = useMemo(() => parseTemplate(localYaml), [localYaml]);

  const onSave = async () => {
    setBusy(true);
    setError(null);
    try {
      await rpc.call("update_job_template", {
        job_id: jobId,
        template_yaml: buffer,
      });
      setEditing(false);
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const onDiscard = () => {
    setBuffer(templateYaml ?? "");
    setEditing(false);
    setError(null);
  };

  // Used by the inline pickers (global docs order, per-stage docs).
  // They mutate the YAML surgically, then save through the same RPC
  // the raw editor uses. Errors surface in the same `error` slot.
  // On success we update `localYaml` immediately so the picker's
  // next mutation composes against the just-saved content, not the
  // parent's stale prop. `onSaved()` is still called so siblings can
  // refresh (file list, etc.); the parent's `useJob` will catch up
  // on the next get_job round-trip.
  const persistYaml = async (nextYaml: string) => {
    setBusy(true);
    setError(null);
    try {
      await rpc.call("update_job_template", {
        job_id: jobId,
        template_yaml: nextYaml,
      });
      setLocalYaml(nextYaml);
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="border-border/50 rounded border">
      <SectionHeader
        title="template.yaml"
        subtitle="name, goal, stages — what the runner will do"
        dirty={dirty}
        actions={
          editing ? (
            <>
              <Button
                size="sm"
                variant="ghost"
                onClick={onDiscard}
                disabled={busy}
                className="h-7 px-2 text-xs"
              >
                discard
              </Button>
              <Button
                size="sm"
                onClick={() => void onSave()}
                disabled={busy || !dirty}
                className="h-7 px-2 text-xs"
              >
                {busy ? "saving…" : "save"}
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setEditing(true)}
              className="h-7 px-2 text-xs"
            >
              edit YAML
            </Button>
          )
        }
      />
      {error && (
        <div className="text-destructive border-destructive/40 bg-destructive/5 border-t px-3 py-2 text-xs">
          {error}
        </div>
      )}
      {editing ? (
        <div className="p-3">
          <InlineEditor
            value={buffer}
            onChange={setBuffer}
            language="yaml"
            minHeight="280px"
          />
        </div>
      ) : (
        <Summary
          parsed={parsed}
          yaml={localYaml}
          availableDocs={availableDocs}
          busy={busy}
          onPersist={(next) => void persistYaml(next)}
        />
      )}
    </section>
  );
}

function Summary({
  parsed,
  yaml,
  availableDocs,
  busy,
  onPersist,
}: {
  parsed: ReturnType<typeof parseTemplate>;
  yaml: string;
  availableDocs: string[];
  busy: boolean;
  onPersist: (nextYaml: string) => void;
}) {
  return (
    <div className="space-y-3 p-3 text-sm">
      <Field label="name">
        {parsed.name ?? <span className="text-muted-foreground italic">unset</span>}
      </Field>
      <Field label="goal">
        {parsed.goal ?? <span className="text-muted-foreground italic">unset</span>}
      </Field>
      <Field label="stages">
        {parsed.stages.length === 0 ? (
          <span className="text-muted-foreground italic">no stages</span>
        ) : (
          <ol className="list-decimal space-y-1 pl-5 text-xs">
            {parsed.stages.map((s, i) => (
              <StageRow
                key={i}
                stage={s}
                ordinal={i}
                availableDocs={availableDocs}
                busy={busy}
                onChangeDocs={(docs) => onPersist(setStageDocs(yaml, i, docs))}
              />
            ))}
          </ol>
        )}
      </Field>
      <Field label="docs order">
        <DocsOrderField
          parsed={parsed}
          yaml={yaml}
          availableDocs={availableDocs}
          busy={busy}
          onPersist={onPersist}
        />
      </Field>
    </div>
  );
}

function StageRow({
  stage,
  ordinal,
  availableDocs,
  busy,
  onChangeDocs,
}: {
  stage: ParsedStage;
  ordinal: number;
  availableDocs: string[];
  busy: boolean;
  onChangeDocs: (docs: string[] | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const summary =
    stage.docs === null
      ? null
      : stage.docs.length === 0
        ? "opted out"
        : stage.docs.join(" + ");
  return (
    <li className="leading-snug">
      <div className="flex items-baseline gap-2">
        {stage.isReview && (
          <Badge variant="outline" className="text-[9px]">
            REVIEW
          </Badge>
        )}
        <span className="min-w-0 flex-1">{stage.title}</span>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="text-muted-foreground hover:text-foreground shrink-0 text-[10px] underline-offset-2 hover:underline"
          title="Pick which markdown docs this stage reads (in addition to the global docs)"
        >
          {summary ? `docs: ${summary}` : "+ stage docs"} {open ? "▴" : "▾"}
        </button>
      </div>
      {open && (
        <div className="border-border/40 mt-1.5 space-y-2 rounded border p-2">
          {availableDocs.length === 0 ? (
            <span className="text-muted-foreground text-[11px]">
              No markdown docs in this job dir yet — add one with{" "}
              <span className="font-mono">+ add markdown doc</span> above.
            </span>
          ) : (
            <>
              <div className="space-y-1">
                {availableDocs.map((doc) => {
                  const checked = stage.docs?.includes(doc) ?? false;
                  return (
                    <label
                      key={doc}
                      className="hover:bg-accent/40 flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-xs"
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        disabled={busy}
                        onChange={(e) => {
                          const next = stage.docs ? [...stage.docs] : [];
                          if (e.target.checked) {
                            if (!next.includes(doc)) next.push(doc);
                          } else {
                            const idx = next.indexOf(doc);
                            if (idx >= 0) next.splice(idx, 1);
                          }
                          onChangeDocs(next);
                        }}
                      />
                      <span className="font-mono">{doc}</span>
                    </label>
                  );
                })}
              </div>
              {stage.docs !== null && (
                <button
                  type="button"
                  onClick={() => onChangeDocs(null)}
                  disabled={busy}
                  className="text-muted-foreground hover:text-foreground text-[10px] underline-offset-2 hover:underline"
                  title="Remove this stage's docs: line — falls back to the template's global docs only"
                >
                  clear (use global docs only)
                </button>
              )}
            </>
          )}
          <p className="text-muted-foreground text-[10px] leading-snug">
            Stage {ordinal + 1} reads global docs first, then the files ticked
            here. Order: as ticked.
          </p>
        </div>
      )}
    </li>
  );
}

function DocsOrderField({
  parsed,
  yaml,
  availableDocs,
  busy,
  onPersist,
}: {
  parsed: ReturnType<typeof parseTemplate>;
  yaml: string;
  availableDocs: string[];
  busy: boolean;
  onPersist: (nextYaml: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const summary = parsed.docsPinned ? (
    <>
      <span className="text-xs">
        pinned —{" "}
        {parsed.pinnedDocs.length === 0
          ? "(empty list)"
          : parsed.pinnedDocs.join(" → ")}
      </span>
    </>
  ) : (
    <span className="text-muted-foreground text-xs">
      auto-discover (every <code>*.md</code> in the job dir, <code>SCOPE.md</code>{" "}
      first, then <code>WORKFLOW.md</code>, then alpha)
    </span>
  );
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline gap-2">
        <span className="min-w-0 flex-1">{summary}</span>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="text-muted-foreground hover:text-foreground shrink-0 text-[10px] underline-offset-2 hover:underline"
          title="Pin the global docs order — overrides auto-discover"
        >
          {parsed.docsPinned ? "edit order" : "pin order"} {open ? "▴" : "▾"}
        </button>
      </div>
      {open && (
        <DocsOrderEditor
          parsed={parsed}
          yaml={yaml}
          availableDocs={availableDocs}
          busy={busy}
          onPersist={onPersist}
        />
      )}
    </div>
  );
}

function DocsOrderEditor({
  parsed,
  yaml,
  availableDocs,
  busy,
  onPersist,
}: {
  parsed: ReturnType<typeof parseTemplate>;
  yaml: string;
  availableDocs: string[];
  busy: boolean;
  onPersist: (nextYaml: string) => void;
}) {
  const pinned = parsed.pinnedDocs;
  // Available list ordered by: pinned items in their pinned order
  // first, then anything else alpha-sorted. Keeps the visible order
  // consistent with what the user sees in the summary.
  const ordered = [
    ...pinned.filter((d) => availableDocs.includes(d)),
    ...availableDocs.filter((d) => !pinned.includes(d)).sort(),
  ];

  const move = (idx: number, delta: -1 | 1) => {
    const next = [...pinned];
    const target = idx + delta;
    if (target < 0 || target >= next.length) return;
    [next[idx], next[target]] = [next[target], next[idx]];
    onPersist(setGlobalDocs(yaml, next));
  };

  const toggle = (doc: string) => {
    const next = parsed.docsPinned ? [...pinned] : [];
    const idx = next.indexOf(doc);
    if (idx >= 0) next.splice(idx, 1);
    else next.push(doc);
    onPersist(setGlobalDocs(yaml, next));
  };

  const clear = () => onPersist(setGlobalDocs(yaml, null));

  return (
    <div className="border-border/40 space-y-2 rounded border p-2">
      {availableDocs.length === 0 ? (
        <span className="text-muted-foreground text-[11px]">
          No markdown docs in this job dir yet — add one with{" "}
          <span className="font-mono">+ add markdown doc</span> above.
        </span>
      ) : (
        <>
          <div className="space-y-1">
            {ordered.map((doc) => {
              const isPinned = pinned.includes(doc);
              const pinnedIdx = pinned.indexOf(doc);
              return (
                <div
                  key={doc}
                  className="hover:bg-accent/40 flex items-center gap-2 rounded px-1.5 py-1 text-xs"
                >
                  <input
                    type="checkbox"
                    checked={isPinned}
                    disabled={busy}
                    onChange={() => toggle(doc)}
                  />
                  <span className="font-mono flex-1">{doc}</span>
                  {isPinned && (
                    <>
                      <button
                        type="button"
                        onClick={() => move(pinnedIdx, -1)}
                        disabled={busy || pinnedIdx === 0}
                        className="text-muted-foreground hover:text-foreground disabled:opacity-30 px-1 text-[10px]"
                        title="move up"
                      >
                        ↑
                      </button>
                      <button
                        type="button"
                        onClick={() => move(pinnedIdx, 1)}
                        disabled={busy || pinnedIdx === pinned.length - 1}
                        className="text-muted-foreground hover:text-foreground disabled:opacity-30 px-1 text-[10px]"
                        title="move down"
                      >
                        ↓
                      </button>
                    </>
                  )}
                </div>
              );
            })}
          </div>
          {parsed.docsPinned && (
            <button
              type="button"
              onClick={clear}
              disabled={busy}
              className="text-muted-foreground hover:text-foreground text-[10px] underline-offset-2 hover:underline"
              title="Remove the docs: block — falls back to auto-discover"
            >
              clear pin (back to auto-discover)
            </button>
          )}
        </>
      )}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[88px_1fr] items-start gap-3">
      <div className="text-muted-foreground text-[10px] uppercase tracking-wide">
        {label}
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

function SectionHeader({
  title,
  subtitle,
  dirty,
  actions,
}: {
  title: string;
  subtitle?: string;
  dirty?: boolean;
  actions?: React.ReactNode;
}) {
  return (
    <header className="border-border/40 flex items-center gap-2 border-b px-3 py-2">
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="font-mono text-sm">{title}</span>
          {dirty && (
            <span
              className="text-[10px] text-amber-600 dark:text-amber-400"
              title="unsaved changes"
            >
              · edited
            </span>
          )}
        </div>
        {subtitle && (
          <div className="text-muted-foreground text-[10px]">{subtitle}</div>
        )}
      </div>
      <div className="flex items-center gap-1">{actions}</div>
    </header>
  );
}
