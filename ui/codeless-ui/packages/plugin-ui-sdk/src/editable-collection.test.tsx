// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/editable-collection.test.tsx@a7fecef1c641cc8800aa2162f108131c6b426451
/**
 * Tests for `useEditableCollection` — the declarative undo/redo/
 * duplicate/copy/paste adapter that plugin UIs wire against the
 * surrounding `<CommandScope>`.
 *
 * Exercises the public surface of `EditableCollectionApi<T>`:
 * `create`, `remove`, `duplicate`, `copy`, `paste`, `undo`, `redo`,
 * `canUndo`, `canRedo`, `canPaste`, and `getContextMenuItems`.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { type ReactNode, createElement } from "react";
import { CommandScope } from "@codeless/ui-core";
import {
  useEditableCollection,
  clearItemClipboard,
  getItemClipboard,
  type EditableCollectionOptions,
  type ItemDraft,
} from "./editable-collection";

// ── Test fixtures ──────────────────────────────────────────────────────────

interface FakeItem {
  path: string;
  parent: string;
  kind: string;
  name: string;
  settings: Record<string, unknown>;
}

function fakeItem(n: number, parent = "/flow-1"): FakeItem {
  return {
    path: `${parent}/item-${n}`,
    parent,
    kind: "com.test.item",
    name: `item-${n}`,
    settings: { idx: n },
  };
}

function buildOpts(items: FakeItem[]) {
  const created: Array<{ draft: ItemDraft; parent: string }> = [];
  const removed: string[] = [];
  let nextId = 100;

  const opts: EditableCollectionOptions<FakeItem> = {
    identify: (i) => i.path,
    items,
    serialise: (i) => ({ kind: i.kind, name: i.name, settings: i.settings }),
    create: async (draft, parent) => {
      const id = `${parent}/new-${nextId++}`;
      created.push({ draft, parent });
      return id;
    },
    remove: async (id) => {
      removed.push(id);
    },
    parentOf: (i) => i.parent,
  };

  return { opts, created, removed };
}

function scopeWrapper({ children }: { children: ReactNode }) {
  return createElement(CommandScope, { id: "test-scope", children });
}

beforeEach(() => {
  clearItemClipboard();
});

describe("useEditableCollection", () => {
  it("starts with canUndo=false, canRedo=false, canPaste=false", () => {
    const { opts } = buildOpts([fakeItem(1)]);
    const { result } = renderHook(() => useEditableCollection(opts), {
      wrapper: scopeWrapper,
    });
    expect(result.current.canUndo).toBe(false);
    expect(result.current.canRedo).toBe(false);
    expect(result.current.canPaste).toBe(false);
    expect(result.current.pending).toBe(false);
  });

  describe("create", () => {
    it("calls the user-supplied create and returns the new id", async () => {
      const { opts, created } = buildOpts([fakeItem(1)]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      let id: string | undefined;
      await act(async () => {
        id = await result.current.create(
          { kind: "com.test.item", name: "new", settings: {} },
          "/flow-1",
        );
      });

      expect(id).toMatch(/^\/flow-1\/new-/);
      expect(created).toHaveLength(1);
      expect(created[0]!.draft.name).toBe("new");
      expect(created[0]!.parent).toBe("/flow-1");
    });

    it("makes canUndo true after create", async () => {
      const { opts } = buildOpts([fakeItem(1)]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.create(
          { kind: "com.test.item", name: "a", settings: {} },
          "/flow-1",
        );
      });

      expect(result.current.canUndo).toBe(true);
    });

    it("undo after create calls remove with the created id", async () => {
      const { opts, removed } = buildOpts([fakeItem(1)]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.create(
          { kind: "com.test.item", name: "a", settings: {} },
          "/flow-1",
        );
      });

      await act(async () => {
        result.current.undo();
      });

      await act(async () => {});

      expect(removed).toHaveLength(1);
      expect(removed[0]).toMatch(/^\/flow-1\/new-/);
    });

    it("redo after undo of create calls create again", async () => {
      const { opts, created } = buildOpts([fakeItem(1)]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.create(
          { kind: "com.test.item", name: "a", settings: {} },
          "/flow-1",
        );
      });

      await act(async () => {
        result.current.undo();
      });
      await act(async () => {});

      await act(async () => {
        result.current.redo();
      });
      await act(async () => {});

      expect(created).toHaveLength(2);
    });
  });

  describe("remove", () => {
    it("calls user-supplied remove with the item's id", async () => {
      const item = fakeItem(1);
      const { opts, removed } = buildOpts([item]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.remove(item);
      });

      expect(removed).toHaveLength(1);
      expect(removed[0]).toBe("/flow-1/item-1");
    });

    it("undo after remove calls create with the snapshot draft", async () => {
      const item = fakeItem(1);
      const { opts, created } = buildOpts([item]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.remove(item);
      });

      expect(result.current.canUndo).toBe(true);

      await act(async () => {
        result.current.undo();
      });
      await act(async () => {});

      expect(created).toHaveLength(1);
      expect(created[0]!.draft.kind).toBe("com.test.item");
      expect(created[0]!.draft.name).toBe("item-1");
      expect(created[0]!.parent).toBe("/flow-1");
    });

    it("redo after undo of remove re-deletes", async () => {
      const item = fakeItem(1);
      const { opts, removed } = buildOpts([item]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.remove(item);
      });

      await act(async () => {
        result.current.undo();
      });
      await act(async () => {});

      await act(async () => {
        result.current.redo();
      });
      await act(async () => {});

      expect(removed).toHaveLength(2);
    });
  });

  describe("duplicate", () => {
    it("creates a copy via the user-supplied create", async () => {
      const item = fakeItem(1);
      const { opts, created } = buildOpts([item]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.duplicate(item);
      });

      expect(created).toHaveLength(1);
      expect(created[0]!.draft.kind).toBe("com.test.item");
      expect(created[0]!.parent).toBe("/flow-1");
    });

    it("uses duplicateOf when provided", async () => {
      const item = fakeItem(1);
      const { opts, created } = buildOpts([item]);
      opts.duplicateOf = (i) => ({
        kind: i.kind,
        name: `${i.name}-copy`,
        settings: { ...i.settings, copied: true },
      });
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.duplicate(item);
      });

      expect(created[0]!.draft.name).toBe("item-1-copy");
      expect(created[0]!.draft.settings).toEqual({ idx: 1, copied: true });
    });

    it("is undoable — undo after duplicate removes the copy", async () => {
      const item = fakeItem(1);
      const { opts, removed } = buildOpts([item]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      await act(async () => {
        await result.current.duplicate(item);
      });

      await act(async () => {
        result.current.undo();
      });
      await act(async () => {});

      expect(removed).toHaveLength(1);
    });
  });

  describe("copy and paste", () => {
    it("copy populates the item clipboard", () => {
      const items = [fakeItem(1), fakeItem(2)];
      const { opts } = buildOpts(items);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      act(() => {
        result.current.copy(items);
      });

      const cb = getItemClipboard();
      expect(cb).not.toBeNull();
      expect(cb!.items).toHaveLength(2);
      expect(cb!.kinds).toEqual(["com.test.item"]);
    });

    it("canPaste becomes true after a compatible copy", () => {
      const items = [fakeItem(1)];
      const { opts } = buildOpts(items);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      expect(result.current.canPaste).toBe(false);

      act(() => {
        result.current.copy(items);
      });

      // canPaste is derived from clipboard + acceptedKinds. Since the
      // hook doesn't automatically re-render when the module-level
      // clipboard changes, re-render via a fresh mount.
      const { result: result2 } = renderHook(
        () => useEditableCollection(opts),
        { wrapper: scopeWrapper },
      );
      expect(result2.current.canPaste).toBe(true);
    });

    it("paste creates items from the clipboard", async () => {
      const items = [fakeItem(1), fakeItem(2)];
      const { opts, created } = buildOpts(items);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      act(() => {
        result.current.copy(items);
      });

      let pasteResult: Awaited<ReturnType<typeof result.current.paste>> | undefined;
      await act(async () => {
        pasteResult = await result.current.paste("/flow-2");
      });

      expect(pasteResult!.created).toHaveLength(2);
      expect(pasteResult!.warnings).toHaveLength(0);
      expect(pasteResult!.summary).toContain("2 items");
      expect(created).toHaveLength(2);
      expect(created[0]!.parent).toBe("/flow-2");
    });

    it("paste skips items with unaccepted kinds", async () => {
      const items = [fakeItem(1)];
      const { opts, created } = buildOpts(items);
      opts.accepts = ["com.other.kind"];
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      act(() => {
        result.current.copy(items);
      });

      let pasteResult: Awaited<ReturnType<typeof result.current.paste>> | undefined;
      await act(async () => {
        pasteResult = await result.current.paste("/flow-1");
      });

      expect(pasteResult!.created).toHaveLength(0);
      expect(pasteResult!.warnings).toHaveLength(1);
      expect(pasteResult!.warnings[0]!.kind).toBe("skipped_kind");
      expect(created).toHaveLength(0);
    });

    it("paste on empty clipboard returns empty result", async () => {
      const { opts } = buildOpts([fakeItem(1)]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      let pasteResult: Awaited<ReturnType<typeof result.current.paste>> | undefined;
      await act(async () => {
        pasteResult = await result.current.paste("/flow-1");
      });

      expect(pasteResult!.created).toHaveLength(0);
      expect(pasteResult!.summary).toContain("empty");
    });
  });

  describe("getContextMenuItems", () => {
    it("always includes duplicate", () => {
      const item = fakeItem(1);
      const { opts } = buildOpts([item]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      const items = result.current.getContextMenuItems(item);
      expect(items.some((i) => i.id === "duplicate")).toBe(true);
    });

    it("includes undo/redo when stack is populated", async () => {
      const item = fakeItem(1);
      const { opts } = buildOpts([item]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      let items = result.current.getContextMenuItems(item);
      expect(items.some((i) => i.id === "undo")).toBe(false);

      await act(async () => {
        await result.current.create(
          { kind: "com.test.item", name: "x", settings: {} },
          "/flow-1",
        );
      });

      items = result.current.getContextMenuItems(item);
      expect(items.some((i) => i.id === "undo")).toBe(true);
    });

    it("includes paste when clipboard is populated with accepted kinds", () => {
      const item = fakeItem(1);
      const items = [item];
      const { opts } = buildOpts(items);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      act(() => {
        result.current.copy(items);
      });

      const { result: result2 } = renderHook(
        () => useEditableCollection(opts),
        { wrapper: scopeWrapper },
      );
      const menuItems = result2.current.getContextMenuItems(item);
      expect(menuItems.some((i) => i.id === "paste")).toBe(true);
    });
  });

  describe("selection", () => {
    it("starts empty", () => {
      const { opts } = buildOpts([fakeItem(1)]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });
      expect(result.current.selection.size).toBe(0);
    });

    it("can be updated via setSelection", () => {
      const { opts } = buildOpts([fakeItem(1)]);
      const { result } = renderHook(() => useEditableCollection(opts), {
        wrapper: scopeWrapper,
      });

      act(() => {
        result.current.setSelection(new Set(["/flow-1/item-1"]));
      });

      expect(result.current.selection.has("/flow-1/item-1")).toBe(true);
    });
  });
});
