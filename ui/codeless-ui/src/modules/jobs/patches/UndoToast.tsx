import { Button } from "@/components/ui/button";

// Transient 10-second toast shown after a plain Approve. Surfaces the
// commit sha and an Undo button that runs `revert_scope_patch`. The
// parent owns the visibility timer; this component is presentation
// only.

interface Props {
  commitSha: string;
  onUndo: () => void;
  onDismiss: () => void;
}

export function UndoToast({ commitSha, onUndo, onDismiss }: Props) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="border-border/60 absolute bottom-4 right-4 flex items-center gap-3 rounded-md border bg-popover px-3 py-2 shadow-md"
    >
      <span className="text-xs">
        Approved{" "}
        <span className="font-mono text-foreground">{commitSha.slice(0, 7)}</span>
      </span>
      <Button
        size="sm"
        variant="outline"
        className="h-6 px-2 text-xs"
        onClick={onUndo}
      >
        undo
      </Button>
      <Button
        size="sm"
        variant="ghost"
        className="h-6 px-1.5 text-xs"
        onClick={onDismiss}
        aria-label="Dismiss"
      >
        ✕
      </Button>
    </div>
  );
}
