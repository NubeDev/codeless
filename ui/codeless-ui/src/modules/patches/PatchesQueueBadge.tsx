import { useMemo } from "react";

import { Button } from "@/components/ui/button";

import { applyAllFilters } from "./filters";
import { usePatchQueue } from "./usePatchQueue";

// Count of unresolved scope-patches awaiting an editor decision,
// rendered in the global nav. Surfaces only the entries the worklist
// would show under its default filters (open + newer than 14 days) so
// the badge does not lure the user into a worklist that quietly
// hides every row behind a decay rule.
//
// Returns `null` when the count is zero so the chrome stays clean.
// The doc's "deliberately not included" list calls out that there
// should be no JobsDashboard widget for patches — the count badge in
// the global nav is the only persistent reminder.

interface Props {
  onOpen: () => void;
}

export function PatchesQueueBadge({ onOpen }: Props) {
  const { entries } = usePatchQueue(null);
  const now = useMemo(() => Date.now(), [entries]);
  const count = useMemo(() => {
    if (entries === null) return 0;
    const visible = applyAllFilters(entries, {
      kinds: new Set(),
      target: "",
      showOlderThan14Days: false,
      now,
    });
    return visible.length;
  }, [entries, now]);

  if (count === 0) return null;
  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={onOpen}
      className="h-7 shrink-0 gap-1.5 rounded-md px-2 text-[11px] text-muted-foreground hover:text-foreground"
      title={`${count} proposed scope-patch${count === 1 ? "" : "es"} awaiting decision`}
    >
      <span className="bg-violet-500/15 text-violet-500 rounded px-1.5 py-0.5 font-mono">
        {count}
      </span>
      <span>patch{count === 1 ? "" : "es"}</span>
    </Button>
  );
}
