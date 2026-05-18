// Empty-state surface rendered when `list_workspaces` returns `[]`.
// Replaces the current "fs_root not set" silent failure mode
// (§"Empty state" in DOCS/WORKSPACE-ATTACH.md). The attach modal
// itself lands in M4b — this component only owns the call-to-action
// button; the parent surface wires the click through to the modal so
// the same button is reusable from the Settings tab and the future
// `/workspaces` route.

import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";

interface EmptyWorkspacesStateProps {
  onAttachClick?: () => void;
}

export function EmptyWorkspacesState({ onAttachClick }: EmptyWorkspacesStateProps) {
  return (
    <Empty data-testid="workspaces-empty-state">
      <EmptyHeader>
        <EmptyTitle>No workspaces attached.</EmptyTitle>
        <EmptyDescription>
          Attach a directory on this machine to start working with codeless.
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button
          type="button"
          onClick={onAttachClick}
          disabled={onAttachClick === undefined}
          data-testid="workspaces-empty-attach-button"
        >
          + Attach a workspace
        </Button>
      </EmptyContent>
    </Empty>
  );
}
