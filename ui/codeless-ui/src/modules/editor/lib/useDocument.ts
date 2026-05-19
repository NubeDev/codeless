import { useCallback, useEffect, useRef, useState } from "react";

import { useRpc } from "@/lib/rpc/provider";
import { useWorkspacesStore } from "@/modules/workspaces/store";

export type DocumentState =
  | { status: "loading" }
  | { status: "ready"; content: string; size: number }
  | { status: "binary"; size: number }
  | { status: "toolarge"; size: number; limit: number }
  | { status: "error"; message: string };

type Options = {
  path: string;
  onDirtyChange?: (dirty: boolean) => void;
};

export function useDocument({ path, onDirtyChange }: Options) {
  const rpc = useRpc();
  const activeRepoId = useWorkspacesStore((s) => s.activeRepoId);
  const [doc, setDoc] = useState<DocumentState>({ status: "loading" });
  const [dirty, setDirty] = useState(false);
  const [reloadCounter, setReloadCounter] = useState(0);

  const savedRef = useRef<string>("");
  const bufferRef = useRef<string>("");
  const dirtyRef = useRef(false);
  useEffect(() => {
    dirtyRef.current = dirty;
  }, [dirty]);

  const onDirtyChangeRef = useRef(onDirtyChange);
  useEffect(() => {
    onDirtyChangeRef.current = onDirtyChange;
  }, [onDirtyChange]);
  useEffect(() => {
    onDirtyChangeRef.current?.(dirty);
  }, [dirty]);

  useEffect(() => {
    let cancelled = false;
    setDoc({ status: "loading" });
    setDirty(false);

    if (!activeRepoId) {
      setDoc({ status: "error", message: "no active workspace" });
      return () => {};
    }

    rpc
      .call("fs_read_file", { repo_id: activeRepoId, path })
      .then((res) => {
        if (cancelled) return;
        // The wire result is `{ content }` — binary and over-limit
        // variants are not yet on the Rust side. The `binary` and
        // `toolarge` legs of `DocumentState` stay because the editor
        // needs them once those variants land; until then any non-
        // utf8 file surfaces as `InvalidArgument` from the server
        // and falls through the `catch` below.
        const content = (res as { content: string }).content;
        savedRef.current = content;
        bufferRef.current = content;
        setDoc({
          status: "ready",
          content,
          size: new TextEncoder().encode(content).length,
        });
      })
      .catch((e) => {
        if (!cancelled) setDoc({ status: "error", message: String(e) });
      });

    return () => {
      cancelled = true;
    };
  }, [path, reloadCounter, rpc, activeRepoId]);

  // Refetches from the source of truth. Silently no-ops if the local
  // buffer is dirty so a background event cannot clobber unsaved
  // user edits. Returns whether the reload was actually queued.
  const reload = useCallback((): boolean => {
    if (dirtyRef.current) return false;
    setReloadCounter((n) => n + 1);
    return true;
  }, []);

  const onChange = useCallback((next: string) => {
    bufferRef.current = next;
    setDirty(next !== savedRef.current);
  }, []);

  const save = useCallback(async () => {
    if (!dirty) return;
    if (!activeRepoId) throw new Error("no active workspace");
    const content = bufferRef.current;
    await rpc.call("fs_write_file", {
      repo_id: activeRepoId,
      path,
      content,
    });
    savedRef.current = content;
    setDirty(false);
  }, [path, dirty, rpc, activeRepoId]);

  return { doc, dirty, onChange, save, reload };
}
