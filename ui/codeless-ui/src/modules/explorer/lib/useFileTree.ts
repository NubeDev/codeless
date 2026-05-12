import { useCallback, useEffect, useState } from "react";

import { useRpc } from "@/lib/rpc/provider";
import type { RpcClient } from "@/lib/rpc/client";

export type DirEntry = {
  name: string;
  kind: "file" | "dir" | "symlink";
  size: number;
  mtime: number;
};

type ChildrenState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "loaded"; entries: DirEntry[] }
  | { status: "error"; message: string };

type TreeState = Record<string, ChildrenState>;

export type PendingCreate = {
  parentPath: string;
  kind: "file" | "dir";
};

export function joinPath(parent: string, name: string): string {
  if (parent.endsWith("/")) return `${parent}${name}`;
  return `${parent}/${name}`;
}

export function dirname(path: string): string {
  const i = path.lastIndexOf("/");
  if (i <= 0) return "/";
  return path.slice(0, i);
}

type Options = {
  onPathRenamed?: (from: string, to: string) => void;
  onPathDeleted?: (path: string) => void;
};

async function listDir(rpc: RpcClient, path: string): Promise<DirEntry[]> {
  const { entries } = await rpc.call("fs_read_dir", { path });
  return entries.map((e) => ({
    name: e.name,
    kind: e.kind,
    size: e.size ?? 0,
    mtime: e.mtime ?? 0,
  }));
}

export function useFileTree(rootPath: string | null, options?: Options) {
  const rpc = useRpc();
  const [nodes, setNodes] = useState<TreeState>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [pendingCreate, setPendingCreate] = useState<PendingCreate | null>(
    null,
  );
  const [renaming, setRenaming] = useState<string | null>(null);

  const fetchChildren = useCallback(
    async (path: string) => {
      setNodes((s) => ({ ...s, [path]: { status: "loading" } }));
      try {
        const entries = await listDir(rpc, path);
        setNodes((s) => ({ ...s, [path]: { status: "loaded", entries } }));
      } catch (e) {
        setNodes((s) => ({
          ...s,
          [path]: { status: "error", message: String(e) },
        }));
      }
    },
    [rpc],
  );

  useEffect(() => {
    if (!rootPath) {
      setNodes({});
      setExpanded(new Set());
      setPendingCreate(null);
      setRenaming(null);
      return;
    }
    setPendingCreate(null);
    setRenaming(null);
    setExpanded(new Set());
    setNodes({});
    void fetchChildren(rootPath);
  }, [rootPath, fetchChildren]);

  const toggle = useCallback(
    (path: string) => {
      setExpanded((curr) => {
        const next = new Set(curr);
        if (next.has(path)) next.delete(path);
        else next.add(path);
        return next;
      });
      setNodes((curr) => {
        if (!curr[path] || curr[path].status === "error") {
          void fetchChildren(path);
        }
        return curr;
      });
    },
    [fetchChildren],
  );

  const expand = useCallback(
    (path: string) => {
      setExpanded((curr) => {
        if (curr.has(path)) return curr;
        const next = new Set(curr);
        next.add(path);
        return next;
      });
      setNodes((curr) => {
        if (!curr[path]) void fetchChildren(path);
        return curr;
      });
    },
    [fetchChildren],
  );

  const refresh = useCallback(
    (path: string) => {
      void fetchChildren(path);
    },
    [fetchChildren],
  );

  const beginCreate = useCallback(
    (parentPath: string, kind: "file" | "dir") => {
      setRenaming(null);
      setPendingCreate({ parentPath, kind });
      if (rootPath && parentPath !== rootPath) {
        setExpanded((curr) => {
          if (curr.has(parentPath)) return curr;
          const next = new Set(curr);
          next.add(parentPath);
          return next;
        });
      }
      setNodes((curr) => {
        if (!curr[parentPath]) void fetchChildren(parentPath);
        return curr;
      });
    },
    [rootPath, fetchChildren],
  );

  const cancelCreate = useCallback(() => setPendingCreate(null), []);

  const commitCreate = useCallback(
    async (name: string) => {
      if (!pendingCreate) return;
      const trimmed = name.trim();
      if (!trimmed) {
        setPendingCreate(null);
        return;
      }
      const path = joinPath(pendingCreate.parentPath, trimmed);
      try {
        if (pendingCreate.kind === "dir") {
          await rpc.call("fs_create_dir", { path, recursive: false });
        } else {
          await rpc.call("fs_create_file", {
            path,
            content: null,
            overwrite: false,
          });
        }
        await fetchChildren(pendingCreate.parentPath);
      } catch (e) {
        console.error("fs_create failed:", e);
      } finally {
        setPendingCreate(null);
      }
    },
    [pendingCreate, fetchChildren, rpc],
  );

  const beginRename = useCallback((path: string) => {
    setPendingCreate(null);
    setRenaming(path);
  }, []);

  const cancelRename = useCallback(() => setRenaming(null), []);

  const commitRename = useCallback(
    async (newName: string) => {
      if (!renaming) return;
      const trimmed = newName.trim();
      const parent = dirname(renaming);
      const oldName = renaming.slice(parent === "/" ? 1 : parent.length + 1);
      if (!trimmed || trimmed === oldName) {
        setRenaming(null);
        return;
      }
      const to = joinPath(parent, trimmed);
      try {
        await rpc.call("fs_move", {
          from: renaming,
          to,
          overwrite: false,
        });
        options?.onPathRenamed?.(renaming, to);
        await fetchChildren(parent);
      } catch (e) {
        console.error("fs_move (rename) failed:", e);
      } finally {
        setRenaming(null);
      }
    },
    [renaming, fetchChildren, options, rpc],
  );

  const deletePath = useCallback(
    async (path: string) => {
      try {
        await rpc.call("fs_delete", { path, recursive: true });
        options?.onPathDeleted?.(path);
        await fetchChildren(dirname(path));
      } catch (e) {
        console.error("fs_delete failed:", e);
      }
    },
    [fetchChildren, options, rpc],
  );

  return {
    nodes,
    expanded,
    pendingCreate,
    renaming,
    toggle,
    expand,
    refresh,
    beginCreate,
    cancelCreate,
    commitCreate,
    beginRename,
    cancelRename,
    commitRename,
    deletePath,
    joinPath,
  };
}
