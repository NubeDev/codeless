// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/editable-collection.tsx@a7fecef1c641cc8800aa2162f108131c6b426451
/**
 * `EditableCollection<T>` — declarative undo/redo/duplicate/copy/paste for
 * a homogeneous list or board rendered by a plugin UI.
 *
 * Plugin authors define the bare minimum the SDK needs to know about each
 * item (how to identify it, serialise it, create it, remove it) and get
 * undo / redo / duplicate / copy / paste wired against the surrounding
 * `<CommandScope>`'s command stack and a session-scoped item clipboard.
 *
 * ```tsx
 * <CommandScope id="plugin:com.acme.hello:/hello/gantt">
 *   <GanttView />
 * </CommandScope>
 *
 * function GanttView() {
 *   const tasks = useEditableCollection<Task>({
 *     identify: (t) => t.path,
 *     items: gantTasks,
 *     serialise: (t) => ({ kind: t.kind, name: t.name, settings: t.settings }),
 *     create: async (draft, parent) => client.tools.call("notes.create", { parent, ...draft }),
 *     remove: async (id) => client.tools.call("notes.remove", { id }),
 *     parentOf: (t) => t.parent,
 *   });
 * }
 * ```
 */
import { useCallback, useMemo, useRef, useState } from "react";
import {
  useCommandStack,
  useCommandStackStore,
} from "@codeless/ui-core";

/**
 * Minimal description of a draft item, used by `create()` and pasted-item
 * materialisation. The shape is intentionally small — the plugin decides
 * what its `kind` and `settings` actually mean.
 */
export interface ItemDraft {
  /** Kind id, e.g. `com.acme.hello.task`. Required so paste into a foreign collection can be rejected. */
  kind: string;
  /** Display name. */
  name: string;
  /** Settings payload — opaque to the SDK; the kind interprets it. */
  settings: Record<string, unknown>;
}

/**
 * Options for `useEditableCollection`. The four required functions form the
 * full contract; the rest are opt-in customisations with sensible defaults.
 */
export interface EditableCollectionOptions<T> {
  identify: (item: T) => string;
  items: ReadonlyArray<T>;
  serialise: (item: T) => ItemDraft;
  create: (draft: ItemDraft, parent: string) => Promise<string>;
  remove: (id: string) => Promise<void>;
  duplicateOf?: (item: T) => ItemDraft;
  parentOf: (item: T) => string;
  accepts?: string[];
}

export interface EditableCollectionApi<T> {
  canUndo: boolean;
  canRedo: boolean;
  canPaste: boolean;
  pending: boolean;

  selection: ReadonlySet<string>;
  setSelection: (next: ReadonlySet<string>) => void;

  create: (draft: ItemDraft, parent: string) => Promise<string>;
  remove: (item: T) => Promise<void>;
  undo: () => void;
  redo: () => void;
  duplicate: (item: T) => Promise<void>;
  copy: (items: ReadonlyArray<T>) => void;
  paste: (parent: string) => Promise<PasteResult>;

  getContextMenuItems: (item: T) => CollectionMenuItem[];
}

export interface PasteResult {
  created: string[];
  summary: string;
  warnings: PasteWarning[];
}

export interface PasteWarning {
  kind: "skipped_kind" | "skipped_role";
  reason: string;
  kind_id?: string;
}

export interface CollectionMenuItem {
  id: "duplicate" | "undo" | "redo" | "paste";
  label: string;
  onClick: () => void;
  variant?: "default" | "destructive";
  disabled?: boolean;
  separator?: boolean;
}

// ── Item clipboard (in-memory, session-scoped) ───────────────────────────────

interface ItemClipboardPayload {
  version: 1;
  kind: "us.clipboard/v1";
  items: ItemDraft[];
  kinds: string[];
}

let itemClipboard: ItemClipboardPayload | null = null;

export function getItemClipboard(): ItemClipboardPayload | null {
  return itemClipboard;
}

export function clearItemClipboard(): void {
  itemClipboard = null;
}

// ── Hook ────────────────────────────────────────────────────────────────────

export function useEditableCollection<T>(
  opts: EditableCollectionOptions<T>,
): EditableCollectionApi<T> {
  const { canUndo, canRedo, undo, redo, pending } = useCommandStack();
  const store = useCommandStackStore();

  const [selection, setSelectionState] = useState<ReadonlySet<string>>(
    () => new Set(),
  );

  // Keep latest opts in a ref so closures captured into the command stack
  // call the current `create`/`remove`/`identify` even if the caller's
  // hook re-renders with new props between record and undo.
  const optsRef = useRef(opts);
  optsRef.current = opts;

  const acceptedKinds = useMemo<Set<string>>(() => {
    if (opts.accepts) return new Set(opts.accepts);
    return new Set(opts.items.map((i) => optsRef.current.serialise(i).kind));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opts.accepts, opts.items]);

  const canPaste = useMemo(() => {
    const cb = itemClipboard;
    if (!cb || cb.items.length === 0) return false;
    return cb.kinds.some((k) => acceptedKinds.has(k));
  }, [acceptedKinds]);

  const setSelection = useCallback((next: ReadonlySet<string>) => {
    setSelectionState(new Set(next));
  }, []);

  const recordedCreate = useCallback(
    async (draft: ItemDraft, parent: string, label: string): Promise<string> => {
      const o = optsRef.current;
      const id = await o.create(draft, parent);
      store.getState().record({
        label,
        undo: async () => {
          await optsRef.current.remove(id);
        },
        redo: async () => {
          await optsRef.current.create(draft, parent);
        },
      });
      return id;
    },
    [store],
  );

  const create = useCallback(
    async (draft: ItemDraft, parent: string): Promise<string> => {
      return recordedCreate(draft, parent, `Create ${draft.name}`);
    },
    [recordedCreate],
  );

  const recordedRemove = useCallback(
    async (item: T): Promise<void> => {
      const o = optsRef.current;
      const id = o.identify(item);
      const draft = o.serialise(item);
      const parent = o.parentOf(item);
      await o.remove(id);
      store.getState().record({
        label: `Delete ${draft.name}`,
        undo: async () => {
          await optsRef.current.create(draft, parent);
        },
        redo: async () => {
          await optsRef.current.remove(id);
        },
      });
    },
    [store],
  );

  const duplicate = useCallback(
    async (item: T): Promise<void> => {
      const o = optsRef.current;
      const draft = o.duplicateOf ? o.duplicateOf(item) : o.serialise(item);
      const parent = o.parentOf(item);
      await recordedCreate(draft, parent, `Duplicate ${draft.name}`);
    },
    [recordedCreate],
  );

  const copy = useCallback((items: ReadonlyArray<T>): void => {
    const drafts = items.map((i) => optsRef.current.serialise(i));
    itemClipboard = {
      version: 1,
      kind: "us.clipboard/v1",
      items: drafts,
      kinds: Array.from(new Set(drafts.map((d) => d.kind))),
    };
  }, []);

  const paste = useCallback(
    async (parent: string): Promise<PasteResult> => {
      const cb = itemClipboard;
      if (!cb || cb.items.length === 0) {
        return { created: [], summary: "Clipboard is empty.", warnings: [] };
      }
      const accepted: ItemDraft[] = [];
      const warnings: PasteWarning[] = [];
      for (const draft of cb.items) {
        if (!acceptedKinds.has(draft.kind)) {
          warnings.push({
            kind: "skipped_kind",
            reason: "not_in_collection_accepts",
            kind_id: draft.kind,
          });
          continue;
        }
        accepted.push(draft);
      }
      const created: string[] = [];
      for (const draft of accepted) {
        const id = await recordedCreate(draft, parent, `Paste ${draft.name}`);
        created.push(id);
      }
      const skipped = warnings.length;
      const summary =
        skipped === 0
          ? `Pasted ${created.length} item${created.length === 1 ? "" : "s"}.`
          : `Pasted ${created.length} item${created.length === 1 ? "" : "s"}; ${skipped} skipped.`;
      return { created, summary, warnings };
    },
    [acceptedKinds, recordedCreate],
  );

  const getContextMenuItems = useCallback(
    (item: T): CollectionMenuItem[] => {
      const o = optsRef.current;
      const items: CollectionMenuItem[] = [
        {
          id: "duplicate",
          label: "Duplicate",
          onClick: () => void duplicate(item),
          separator: true,
        },
      ];
      if (canUndo) {
        items.push({
          id: "undo",
          label: "Undo",
          onClick: () => undo(),
          separator: true,
        });
      }
      if (canRedo) {
        items.push({
          id: "redo",
          label: "Redo",
          onClick: () => redo(),
        });
      }
      if (canPaste) {
        items.push({
          id: "paste",
          label: "Paste",
          onClick: () => void paste(o.parentOf(item)),
          separator: true,
        });
      }
      return items;
    },
    [duplicate, undo, redo, paste, canUndo, canRedo, canPaste],
  );

  return {
    canUndo,
    canRedo,
    canPaste,
    pending,
    selection,
    setSelection,
    create,
    remove: recordedRemove,
    undo,
    redo,
    duplicate,
    copy,
    paste,
    getContextMenuItems,
  };
}
