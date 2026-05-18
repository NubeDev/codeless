// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/components/BlockShell.tsx@a7fecef1c641cc8800aa2162f108131c6b426451
import React from "react";

interface BlockShellProps {
  title?: string;
  className?: string;
  children: React.ReactNode;
}

/**
 * BlockShell — standard panel wrapper for plugin micro-frontend bundles.
 *
 * Provides consistent padding, border, and optional title header so all
 * plugins share a uniform layout chrome without depending on host
 * internals.
 */
export function BlockShell({ title, className = "", children }: BlockShellProps) {
  return (
    <div
      className={`rounded-lg border border-border bg-card text-card-foreground shadow-sm ${className}`}
    >
      {title && (
        <div className="border-b border-border px-4 py-3">
          <h3 className="text-sm font-semibold">{title}</h3>
        </div>
      )}
      <div className="p-4">{children}</div>
    </div>
  );
}
