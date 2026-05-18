// codeless-ported-from: rubix-workspace/extension-ui-sdk/src/components/SlotBadge.tsx@a7fecef1c641cc8800aa2162f108131c6b426451
import React from "react";
import { Badge } from "@codeless/ui-kit";
import { useSlot } from "../hooks/useSlot";

interface SlotBadgeProps {
  path: string;
  slotName: string;
  format?: (value: unknown) => string;
  variant?: "default" | "secondary" | "destructive" | "outline";
  className?: string;
}

/**
 * SlotBadge — renders a Badge whose label mirrors a live slot value.
 *
 * Automatically refreshes when the node's slot changes via SSE events
 * (via the `useSlot` → `useNode` subscription chain).
 */
export function SlotBadge({
  path,
  slotName,
  format,
  variant = "secondary",
  className,
}: SlotBadgeProps) {
  const slot = useSlot(path, slotName);
  const value = slot?.value;
  const label =
    value === undefined || value === null
      ? "—"
      : format
        ? format(value)
        : typeof value === "string"
          ? value
          : JSON.stringify(value);

  return (
    <Badge variant={variant} className={className}>
      {label}
    </Badge>
  );
}
