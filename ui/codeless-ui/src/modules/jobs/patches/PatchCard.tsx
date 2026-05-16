import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { PatchEditor } from "./PatchEditor";
import {
  parseProposalMarkdown,
  renderProposalMarkdown,
  type PatchProposal,
  type PatchResolution,
} from "./proposal";

// One inbox card. The visual matches the ASCII mockup in
// `DOCS/SCOPE-MUTABLE-UI.md` Surface B: kind badge, target file,
// rationale, evidence stage link, predicate-shipped flag, proposed
// timestamp, and the three action buttons.
//
// Resolved rows render a collapsed summary instead of the action
// buttons: a one-line "<approved|rejected> in <sha-short>" with a
// link to the resolution commit. The card stays in the inbox until
// the page reloads — the runtime's queue file no longer carries it,
// but surfacing the recent resolution lets the editor confirm what
// they (or a sibling window) just did.

interface Props {
  proposal: PatchProposal;
  proposedAt: number;
  resolution: PatchResolution | null;
  onApprove: () => void | Promise<void>;
  onReject: () => void | Promise<void>;
  onApproveAfterEdit: (editedRendered: string) => void;
  onEditSaved: (updated: PatchProposal) => void;
}

export function PatchCard({
  proposal,
  proposedAt,
  resolution,
  onApprove,
  onReject,
  onApproveAfterEdit,
  onEditSaved,
}: Props) {
  // Local edit-mode flag. Edit replaces the card body with a
  // CodeMirror buffer initialised to the rendered proposal. Save
  // re-parses inline; Approve from the edit pane runs the
  // approve-after-edit dialog instead of plain Approve.
  const [editing, setEditing] = useState(false);
  const [editBuffer, setEditBuffer] = useState<string | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"approve" | "reject" | null>(null);

  const startEdit = () => {
    setEditBuffer(renderProposalMarkdown(proposal));
    setEditing(true);
    setParseError(null);
  };

  const cancelEdit = () => {
    setEditing(false);
    setEditBuffer(null);
    setParseError(null);
  };

  const saveEdit = () => {
    if (editBuffer === null) return;
    const parsed = parseProposalMarkdown(editBuffer, proposal);
    if (!parsed.ok) {
      setParseError(parsed.error);
      return;
    }
    onEditSaved(parsed.proposal);
    setEditing(false);
    setEditBuffer(null);
    setParseError(null);
  };

  const approveFromEdit = () => {
    if (editBuffer === null) return;
    // Client-side pre-flight: surface the parse error before sending
    // a buffer the server will reject anyway. The runtime re-parses
    // for the authoritative check.
    const parsed = parseProposalMarkdown(editBuffer, proposal);
    if (!parsed.ok) {
      setParseError(parsed.error);
      return;
    }
    // Hand the buffer to the parent's approve-after-edit handler so
    // the diff dialog opens with the original-vs-edited delta.
    onApproveAfterEdit(editBuffer);
  };

  return (
    <div
      className={cn(
        "border-border/60 rounded-md border bg-card/40 shadow-sm",
        resolution !== null && "opacity-70",
      )}
    >
      <Header
        kind={proposal.kind}
        targetPath={proposal.target_path}
        idShort={shortId(proposal.id)}
      />

      <div className="space-y-3 px-4 py-3">
        {editing ? (
          <PatchEditor
            value={editBuffer ?? ""}
            onChange={(v) => {
              setEditBuffer(v);
              if (parseError !== null) setParseError(null);
            }}
          />
        ) : (
          <Rationale text={proposal.rationale} />
        )}

        <MetaRow proposal={proposal} proposedAt={proposedAt} />

        {parseError && (
          <p className="text-destructive text-xs font-mono">{parseError}</p>
        )}

        {resolution !== null ? (
          <ResolvedRow resolution={resolution} />
        ) : editing ? (
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="default"
              className="h-7 px-2.5 text-xs"
              onClick={saveEdit}
            >
              save
            </Button>
            <Button
              size="sm"
              variant="default"
              className="h-7 px-2.5 text-xs"
              onClick={approveFromEdit}
            >
              approve (after edit)
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 px-2.5 text-xs"
              onClick={cancelEdit}
            >
              cancel
            </Button>
          </div>
        ) : (
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="default"
              className="h-7 px-2.5 text-xs"
              disabled={busy !== null}
              onClick={async () => {
                setBusy("approve");
                try {
                  await onApprove();
                } finally {
                  setBusy(null);
                }
              }}
            >
              {busy === "approve" ? "approving…" : "approve"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 px-2.5 text-xs"
              disabled={busy !== null}
              onClick={async () => {
                setBusy("reject");
                try {
                  await onReject();
                } finally {
                  setBusy(null);
                }
              }}
            >
              {busy === "reject" ? "rejecting…" : "reject"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-7 px-2.5 text-xs"
              disabled={busy !== null}
              onClick={startEdit}
            >
              edit
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

function Header({
  kind,
  targetPath,
  idShort,
}: {
  kind: "tighten" | "loosen";
  targetPath: string;
  idShort: string;
}) {
  return (
    <div className="border-border/50 flex flex-wrap items-center gap-2 border-b px-4 py-2.5">
      <Badge
        variant={kind === "tighten" ? "default" : "secondary"}
        className="uppercase tracking-wider text-[10px]"
      >
        {kind}
      </Badge>
      <span className="font-mono text-xs">{targetPath}</span>
      <span className="ml-auto font-mono text-[10px] text-muted-foreground">
        #{idShort}
      </span>
    </div>
  );
}

function Rationale({ text }: { text: string }) {
  if (text.trim() === "") {
    return (
      <p className="text-sm italic text-muted-foreground">
        (no rationale carried on the SSE envelope — open Edit to view or amend
        the full proposal block)
      </p>
    );
  }
  return <p className="whitespace-pre-wrap text-sm">{text}</p>;
}

function MetaRow({
  proposal,
  proposedAt,
}: {
  proposal: PatchProposal;
  proposedAt: number;
}) {
  return (
    <div className="space-y-1 font-mono text-[11px] text-muted-foreground">
      {proposal.evidence_stage_id && (
        <div>
          Evidence: stage{" "}
          <a
            href={`?tab=stage:${proposal.evidence_stage_id}`}
            className="underline hover:text-foreground"
          >
            {proposal.evidence_stage_id.slice(0, 8)}
          </a>
        </div>
      )}
      <div>
        Predicate:{" "}
        {proposal.has_predicate ? (
          <span className="text-emerald-600 dark:text-emerald-400">SHIPPED</span>
        ) : (
          <span className="text-amber-600 dark:text-amber-400">NOT SHIPPED</span>
        )}
        {!proposal.has_predicate && proposal.kind === "tighten" && (
          <span className="ml-1">
            ⚠ Tightening requires a predicate; approve will reject at parse
            time.
          </span>
        )}
      </div>
      <div>Proposed: {formatTimestamp(proposedAt)}</div>
    </div>
  );
}

function ResolvedRow({ resolution }: { resolution: PatchResolution }) {
  const verb =
    resolution.kind === "approved"
      ? "Approved"
      : resolution.kind === "rejected"
        ? "Rejected"
        : "Reverted";
  const short = resolution.commit_sha.slice(0, 7);
  return (
    <div className="text-xs font-mono text-muted-foreground">
      {verb} in <span className="text-foreground">{short}</span>
    </div>
  );
}

function shortId(id: string): string {
  return id.slice(0, 8);
}

function formatTimestamp(ms: number): string {
  const d = new Date(ms);
  // ISO local time without seconds, matching the ASCII mockup's
  // "2026-05-15 16:42" shape.
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
