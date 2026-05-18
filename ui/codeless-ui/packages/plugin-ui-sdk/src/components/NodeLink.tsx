// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/components/NodeLink.tsx@a7fecef1c641cc8800aa2162f108131c6b426451
import React from "react";
import { useNode } from "../hooks/useNode";

interface NodeLinkProps {
  path: string;
  label?: string;
  onClick?: (path: string) => void;
  className?: string;
}

/**
 * NodeLink — a clickable reference chip that resolves the node's
 * display name. Renders a button-style anchor; if the node is not yet
 * loaded it shows the last path segment as a fallback label.
 */
export function NodeLink({ path, label, onClick, className = "" }: NodeLinkProps) {
  const node = useNode(path);
  const displayLabel = label ?? node?.path.split("/").pop() ?? path.split("/").pop() ?? path;

  return (
    <button
      type="button"
      onClick={() => onClick?.(path)}
      className={`inline-flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-xs font-medium hover:bg-accent transition-colors ${className}`}
    >
      {displayLabel}
    </button>
  );
}
