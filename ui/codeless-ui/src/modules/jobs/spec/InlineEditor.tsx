import CodeMirror from "@uiw/react-codemirror";
import { StreamLanguage } from "@codemirror/language";
import { yaml } from "@codemirror/legacy-modes/mode/yaml";
import { markdown } from "@codemirror/lang-markdown";
import { useMemo } from "react";

// Lightweight CodeMirror instance for editing a single string buffer
// inline inside a Spec section. Distinct from the full `EditorPane`
// (file-backed, vim-mode, autocomplete, themes) — those features
// would dominate a small inline editor and tie the buffer to a file
// path. The Spec sections own the file lifecycle themselves; this
// component only renders + edits a string.
export function InlineEditor({
  value,
  onChange,
  language,
  readOnly = false,
  minHeight = "180px",
  maxHeight = "60vh",
}: {
  value: string;
  onChange: (next: string) => void;
  language: "yaml" | "markdown";
  readOnly?: boolean;
  minHeight?: string;
  maxHeight?: string;
}) {
  const ext = useMemo(
    () => (language === "yaml" ? [StreamLanguage.define(yaml)] : [markdown()]),
    [language],
  );
  return (
    <div className="border-border/50 overflow-hidden rounded border">
      <CodeMirror
        value={value}
        onChange={onChange}
        readOnly={readOnly}
        extensions={ext}
        basicSetup={{
          lineNumbers: false,
          foldGutter: false,
          highlightActiveLine: !readOnly,
          highlightActiveLineGutter: false,
        }}
        style={{
          fontSize: "12px",
          fontFamily: "JetBrains Mono, monospace",
        }}
        minHeight={minHeight}
        maxHeight={maxHeight}
      />
    </div>
  );
}
