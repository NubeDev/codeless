import { useEffect, useRef } from "react";

import { markdown } from "@codemirror/lang-markdown";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";

import { buildSharedExtensions } from "@/modules/editor/lib/extensions";

// Minimal CodeMirror buffer for the in-card patch editor. Not the
// full `EditorPane` from `modules/editor/` because that surface is
// tied to file-backed open/save flow; a patch edit is a transient
// in-memory string until the parent calls `edit_scope_patch` /
// `approve_scope_patch`. Sharing the theme extension from
// `editor/lib/extensions` keeps the look consistent with the main
// editor.

interface Props {
  value: string;
  onChange: (next: string) => void;
}

export function PatchEditor({ value, onChange }: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  // Track the latest `onChange` in a ref so the `updateListener`
  // closure (built once on mount) does not capture a stale callback.
  const onChangeRef = useRef(onChange);
  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    if (!hostRef.current) return;
    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        markdown(),
        ...buildSharedExtensions(),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) {
            onChangeRef.current(u.state.doc.toString());
          }
        }),
      ],
    });
    const view = new EditorView({ state, parent: hostRef.current });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Mount-only: we do not rebuild the editor on each `value` prop
    // change because the user is the source of truth while editing.
    // External resets happen by remounting (key change in parent).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div
      ref={hostRef}
      className="border-border/60 max-h-80 min-h-48 overflow-auto rounded border bg-background"
    />
  );
}
