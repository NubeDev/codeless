import { useCallback, useEffect, useRef, useState } from "react";

import { useRpc } from "@/lib/rpc/provider";

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

    rpc
      .call("fs_read_file", { path, byte_limit: null })
      .then((res) => {
        if (cancelled) return;
        if (res.kind === "text") {
          savedRef.current = res.content;
          bufferRef.current = res.content;
          setDoc({
            status: "ready",
            content: res.content,
            size: new TextEncoder().encode(res.content).length,
          });
        } else if (res.kind === "binary") {
          setDoc({
            status: "binary",
            size: Math.ceil((res.bytes_base64.length * 3) / 4),
          });
        } else if (res.kind === "toolarge") {
          setDoc({
            status: "toolarge",
            size: res.size,
            limit: res.limit,
          });
        }
      })
      .catch((e) => {
        if (!cancelled) setDoc({ status: "error", message: String(e) });
      });

    return () => {
      cancelled = true;
    };
  }, [path, reloadCounter, rpc]);

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
    const content = bufferRef.current;
    await rpc.call("fs_write_file", {
      path,
      content,
      create_parents: false,
    });
    savedRef.current = content;
    setDirty(false);
  }, [path, dirty, rpc]);

  return { doc, dirty, onChange, save, reload };
}
