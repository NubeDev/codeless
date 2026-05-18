import { useCallback, useEffect, useMemo, useState } from "react";
import {
  useRpc,
  type AssistantAction,
  type AssistantActionCard,
  type AssistantActionStatus,
  type AssistantAttachmentCard,
  type AssistantMessage,
  type AssistantMessageId,
  type AssistantThread,
  type JobId,
  type Repo,
  type SubmitJobArgs,
} from "@/lib/rpc";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { navigate } from "@/lib/route";
import {
  ChatMessageList,
  MarkdownBubble,
  type ChatMessage,
} from "../chat";
import {
  JobComposer,
  composerToSubmitArgs,
  slugifyName,
  useJobComposerState,
  type JobComposerInitial,
} from "../jobs/composer";
import { useAssistantFocus } from "./focusStore";

// Stage-6 assistant view. Renders the persisted transcript for one
// thread plus a composer that appends a user turn and the no-op
// server-side responder. The renderer is deliberately minimal: full
// markdown / tool-call / attachment rendering reuses the JobChat
// machinery in later stages once the assistant grows the matching
// server-side capabilities. Keeping this surface small now means the
// rewire to share chrome with JobChat does not have to undo a richer
// renderer first.
export type AssistantThreadViewProps = {
  thread: AssistantThread;
  /**
   * Fired after a successful `append_assistant_message` so the parent
   * rail can refresh `updated_at` ordering. Optional — the view still
   * works without it; the rail just won't re-sort until the next
   * refresh.
   */
  onThreadTouched?: () => void;
};

export function AssistantThreadView({
  thread,
  onThreadTouched,
}: AssistantThreadViewProps) {
  const rpc = useRpc();
  const refreshTick = useAssistantFocus((s) => s.refreshTick);
  const [messages, setMessages] = useState<AssistantMessage[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Reload when the parent rail swaps in a different thread, *or*
  // when `refreshTick` bumps — the footer composer increments the
  // tick after a successful `append_assistant_message`, so a
  // message sent from the footer surfaces in this view on the next
  // render without a per-thread subscription channel.
  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setErr(null);
    void rpc
      .call("list_assistant_messages", { thread_id: thread.id })
      .then((res) => {
        if (cancelled) return;
        setMessages(res.messages);
        setLoaded(true);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setErr(e instanceof Error ? e.message : String(e));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, thread.id, refreshTick]);

  const onSubmit = useCallback(
    async (e?: React.FormEvent) => {
      e?.preventDefault();
      const content = input.trim();
      if (!content || sending) return;
      setSending(true);
      setErr(null);
      try {
        const res = await rpc.call("append_assistant_message", {
          thread_id: thread.id,
          content,
        });
        // The planner may emit one or more action-card rows alongside
        // the prose reply; they arrive in created_at order, so a plain
        // concatenation matches what a re-list would render. Empty for
        // a plain Q&A turn — slash commands route through the same
        // shape with `assistant_message` carrying the only card.
        setMessages((prev) => [
          ...prev,
          res.user_message,
          res.assistant_message,
          ...(res.cards ?? []),
        ]);
        setInput("");
        onThreadTouched?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      } finally {
        setSending(false);
      }
    },
    [rpc, thread.id, input, sending, onThreadTouched],
  );

  // Action-card resolution: confirm dispatches the proposed tool call
  // server-side and appends a `Tool`-role message with the structured
  // result; cancel only flips status. Both replace the card row in
  // place (same `id`, new `meta_json`) so React state stays consistent
  // with the rail without a full re-list.
  const onConfirmAction = useCallback(
    async (messageId: string) => {
      setErr(null);
      try {
        const res = await rpc.call("confirm_assistant_action", {
          thread_id: thread.id,
          message_id: messageId as AssistantMessage["id"],
        });
        setMessages((prev) => {
          const next = prev.map((m) =>
            m.id === res.card.id ? res.card : m,
          );
          next.push(res.tool_message);
          return next;
        });
        onThreadTouched?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      }
    },
    [rpc, thread.id, onThreadTouched],
  );

  const onCancelAction = useCallback(
    async (messageId: string) => {
      setErr(null);
      try {
        const res = await rpc.call("cancel_assistant_action", {
          thread_id: thread.id,
          message_id: messageId as AssistantMessage["id"],
        });
        setMessages((prev) =>
          prev.map((m) => (m.id === res.card.id ? res.card : m)),
        );
        onThreadTouched?.();
      } catch (e: unknown) {
        setErr(e instanceof Error ? e.message : String(e));
      }
    },
    [rpc, thread.id, onThreadTouched],
  );

  // `draft_job` cards confirm via `submit_job` directly so the
  // composer's user-edited values reach the runtime — the existing
  // `confirm_assistant_action` path dispatches `draft_job_from_conversation`
  // which reads the planner's original args back off the card. Once the
  // job exists, flip the card's locally-rendered status to "confirmed"
  // and append a synthetic tool row pointing at the new job so the
  // transcript reflects the outcome without a refetch. The persisted
  // card row stays `pending` on the server; a follow-up server endpoint
  // (parity-scope §W3) will accept the edited args alongside the card
  // id so reload sees the same confirmed state. Until then a thread
  // re-list would surface the card as pending — acceptable for the
  // W2-only UI cut.
  const onConfirmDraftJob = useCallback(
    async (messageId: string, args: SubmitJobArgs) => {
      setErr(null);
      const job = await rpc.call("submit_job", args);
      const now = Date.now();
      setMessages((prev) => {
        const next = prev.map((m) => {
          if (m.id !== messageId) return m;
          const card = parseActionCard(m.meta_json);
          if (!card) return m;
          const updated: AssistantActionCard = {
            ...card,
            status: "confirmed",
          };
          return { ...m, meta_json: JSON.stringify(updated) };
        });
        next.push({
          id: `local-${messageId}` as AssistantMessageId,
          thread_id: thread.id,
          role: "tool",
          content: `Drafted job \`${job.id}\` (status: ${job.status}).`,
          meta_json: JSON.stringify({ tool: "draft_job", job }),
          created_at: now,
        });
        return next;
      });
      onThreadTouched?.();
    },
    [rpc, thread.id, onThreadTouched],
  );

  // Project the assistant's native row type into the `ChatMessage`
  // shape `ChatMessageList` consumes. The wrapper supplies a custom
  // `renderMessage` below that reads `meta` back to dispatch on the
  // original — action cards, attachment cards, and `tool`-role
  // results need shapes `ChatMessage` (role + text + ts) cannot
  // encode. Keying by the persisted `AssistantMessage.id` lets the
  // status flip on a card (pending → confirmed) update in place
  // instead of unmounting and remounting the row.
  const history = useMemo<ChatMessage[]>(
    () =>
      messages.map((m) => ({
        role: m.role === "user" ? "user" : "assistant",
        text: m.content,
        ts: new Date(m.created_at).toISOString(),
        key: m.id,
        meta: m,
      })),
    [messages],
  );

  const renderMessage = useCallback(
    (msg: ChatMessage, key: string) => {
      const original = msg.meta as AssistantMessage | undefined;
      if (!original) return null;
      return (
        <MessageBubble
          key={key}
          message={original}
          onConfirmAction={onConfirmAction}
          onCancelAction={onCancelAction}
          onConfirmDraftJob={onConfirmDraftJob}
        />
      );
    },
    [onConfirmAction, onCancelAction, onConfirmDraftJob],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="border-b border-border/60 px-4 py-2">
        <h2 className="truncate text-sm font-medium">{thread.title}</h2>
        <p className="text-[11px] text-muted-foreground">
          {thread.id}
        </p>
      </div>

      <div className="min-h-0 flex-1 overflow-hidden p-4">
        {!loaded ? (
          <div className="text-xs text-muted-foreground">Loading…</div>
        ) : (
          <ChatMessageList
            // Planner publishes onto `(thread_id-as-job_id, task_id)`
            // (see `assistant_planner.rs:108`); the same SSE channel
            // powers job chats. `append_assistant_message` blocks and
            // never round-trips the task id to the client, so the
            // wildcard sentinel accepts the in-flight turn's tokens
            // — safe because the RPC contract pins one turn per
            // thread.
            filter={{ scope: "job", job_id: thread.id as unknown as JobId }}
            history={history}
            activeTaskId={sending ? "*" : null}
            renderMessage={renderMessage}
            emptyState={
              <li className="text-xs text-muted-foreground">
                No messages yet. Say hello to seed the thread.
              </li>
            }
            className="flex h-full min-h-0 flex-col gap-3 overflow-y-auto pr-1"
          />
        )}
      </div>

      {err && (
        <div className="border-t border-destructive/40 bg-destructive/10 px-4 py-2 text-xs text-destructive">
          {err}
        </div>
      )}

      <form
        onSubmit={onSubmit}
        className="flex items-end gap-2 border-t border-border/60 bg-card p-3"
      >
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends; Shift+Enter inserts a newline. Matches the
            // other chat composers in the app so muscle memory transfers.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void onSubmit();
            }
          }}
          placeholder="Message the assistant…"
          rows={2}
          disabled={sending}
          className="min-h-[44px] flex-1 resize-none rounded-md border border-border/60 bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <Button type="submit" size="sm" disabled={!input.trim() || sending}>
          {sending ? "Sending…" : "Send"}
        </Button>
      </form>
    </div>
  );
}

type MessageBubbleProps = {
  message: AssistantMessage;
  onConfirmAction: (messageId: string) => void;
  onCancelAction: (messageId: string) => void;
  onConfirmDraftJob: (messageId: string, args: SubmitJobArgs) => Promise<void>;
};

function MessageBubble({
  message,
  onConfirmAction,
  onCancelAction,
  onConfirmDraftJob,
}: MessageBubbleProps) {
  // Action cards are stored as `Assistant`-role messages whose
  // `meta_json` decodes to an `AssistantActionCard`. The role
  // discriminator stays "assistant" (not a new role) so renderers
  // that don't know about cards still see them as a normal turn
  // with a human-readable summary in `content`.
  const card = parseActionCard(message.meta_json);
  if (message.role === "assistant" && card) {
    return (
      <ActionCardView
        message={message}
        card={card}
        onConfirm={() => onConfirmAction(message.id)}
        onCancel={() => onCancelAction(message.id)}
        onConfirmDraftJob={(args) => onConfirmDraftJob(message.id, args)}
      />
    );
  }
  // PS7 (`DOCS/PLUGIN-SUBSTRATE.md` item 7): a `Tool`-role message
  // whose meta_json decodes to an `attachment_card` carries one or
  // more reconciled attachments produced by the tool call. Render
  // ahead of the generic `ToolResultView` so the download card shows
  // instead of a raw-JSON dump.
  const attachmentCard = parseAttachmentCard(message.meta_json);
  if (attachmentCard) {
    return (
      <AttachmentCardView message={message} card={attachmentCard} />
    );
  }
  if (message.role === "tool") {
    return <ToolResultView message={message} />;
  }
  // Plain prose turn. Routed through the shared MarkdownBubble so the
  // assistant transcript renders the same markdown surface area as the
  // job chat instead of dumping raw asterisks and fences as text.
  return (
    <MarkdownBubble
      role={message.role === "user" ? "user" : "assistant"}
      content={message.content}
    />
  );
}

// `meta_json` is the wire-typed `string | null`. Cards are JSON
// documents with `kind == "action_card"`; everything else is some
// other future meta shape and falls back to plain rendering.
function parseActionCard(raw: string | null): AssistantActionCard | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<AssistantActionCard>;
    if (parsed && parsed.kind === "action_card" && parsed.action && parsed.status) {
      return parsed as AssistantActionCard;
    }
    return null;
  } catch {
    return null;
  }
}

const STATUS_LABEL: Record<AssistantActionStatus, string> = {
  pending: "Pending",
  confirmed: "Confirmed",
  cancelled: "Cancelled",
  failed: "Failed",
};

const STATUS_TONE: Record<AssistantActionStatus, string> = {
  pending: "border-yellow-500/60 bg-yellow-500/5",
  confirmed: "border-emerald-500/60 bg-emerald-500/5",
  cancelled: "border-muted-foreground/40 bg-muted/40",
  failed: "border-destructive/60 bg-destructive/10",
};

type ActionCardViewProps = {
  message: AssistantMessage;
  card: AssistantActionCard;
  onConfirm: () => void;
  onCancel: () => void;
  onConfirmDraftJob: (args: SubmitJobArgs) => Promise<void>;
};

// Confirmation-gated action card. The user-facing "confirm" button is
// only live while `status == "pending"`; once a card is resolved the
// buttons retire so a re-render of the transcript cannot fire the
// same RPC twice (the server enforces this too — the UI is just
// cooperating). `draft_job` cards branch to the editable composer
// instead of the read-only preview while pending — the planner's
// proposed JobSpec is review-then-edit, not review-then-accept-only,
// so the composer surfaces every field the dialog shell does.
function ActionCardView({
  message,
  card,
  onConfirm,
  onCancel,
  onConfirmDraftJob,
}: ActionCardViewProps) {
  const isPending = card.status === "pending";
  const draftJobEditable =
    isPending && card.action.tool === "draft_job";
  return (
    <div className="flex justify-start">
      <div
        className={cn(
          "flex w-full max-w-[85%] flex-col gap-2 rounded-md border px-3 py-2 text-sm",
          STATUS_TONE[card.status],
        )}
      >
        <div className="flex items-center justify-between gap-2">
          <span className="text-[11px] font-mono uppercase tracking-wide text-muted-foreground">
            {actionLabel(card.action)}
          </span>
          <span className="text-[11px] uppercase text-muted-foreground">
            {STATUS_LABEL[card.status]}
          </span>
        </div>
        <div className="whitespace-pre-wrap">{message.content}</div>
        {card.action.tool === "draft_job" && !draftJobEditable && (
          <DraftJobPreview action={card.action} />
        )}
        {draftJobEditable && card.action.tool === "draft_job" && (
          <DraftJobComposerPanel
            action={card.action}
            onConfirm={onConfirmDraftJob}
            onCancel={onCancel}
          />
        )}
        {card.action.tool === "edit_scope" && (
          <EditScopePreview action={card.action} />
        )}
        {isPending && !draftJobEditable && (
          <div className="mt-1 flex justify-end gap-2">
            <Button
              size="sm"
              variant="ghost"
              onClick={onCancel}
              aria-label="Cancel action"
            >
              Cancel
            </Button>
            <Button
              size="sm"
              onClick={onConfirm}
              aria-label="Confirm action"
            >
              Confirm
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

// Editable companion to `DraftJobPreview`. Renders the same form
// `SubmitJobDialog` mounts (`JobComposer`) so the user can edit every
// field — runner, branch, caps, model overrides, auto-bypass policy —
// before confirming the planner's draft. `composerToSubmitArgs` maps
// the state to `submit_job` wire args verbatim, which is the parity
// guarantee §W2 of `DOCS/SCOPE-ASSISTANT-PARITY.md` exists to enforce:
// a slug or cap-cents bug fixed in the composer reaches both surfaces.
//
// The planner's `draft_job` action does not carry a job name (the
// composer derives the on-disk folder slug); the proposed branch is
// the strongest signal we have, so seed the name from the branch with
// the workspace's "codeless/" prefix stripped. The user can rename
// before confirming.
type DraftJobComposerPanelProps = {
  action: Extract<AssistantAction, { tool: "draft_job" }>;
  onConfirm: (args: SubmitJobArgs) => Promise<void>;
  onCancel: () => void;
};

function DraftJobComposerPanel({
  action,
  onConfirm,
  onCancel,
}: DraftJobComposerPanelProps) {
  const rpc = useRpc();
  const [repo, setRepo] = useState<Repo | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setRepo(null);
    setLoadErr(null);
    void rpc
      .call("list_repos", {})
      .then((res) => {
        if (cancelled) return;
        const found = res.repos.find((r) => r.id === action.repo_id) ?? null;
        if (!found) {
          setLoadErr(`repo ${action.repo_id} is no longer registered`);
        } else {
          setRepo(found);
        }
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadErr(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, action.repo_id]);

  if (loadErr) {
    return (
      <div className="mt-1 rounded border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
        {loadErr}
      </div>
    );
  }
  if (!repo) {
    return (
      <div className="mt-1 text-xs text-muted-foreground">
        Loading composer…
      </div>
    );
  }
  return (
    <DraftJobComposerPanelInner
      repo={repo}
      action={action}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />
  );
}

type DraftJobComposerPanelInnerProps = DraftJobComposerPanelProps & {
  repo: Repo;
};

function DraftJobComposerPanelInner({
  repo,
  action,
  onConfirm,
  onCancel,
}: DraftJobComposerPanelInnerProps) {
  const rpc = useRpc();
  const initial: JobComposerInitial = useMemo(
    () => ({
      // Planner emits a branch but no folder slug. Strip the conventional
      // `codeless/` prefix to reach a name the user is likely to want;
      // the field stays editable so any non-conforming branch can be
      // overridden before confirm.
      name: slugifyName(action.branch.replace(/^codeless\//, "")),
      branch: action.branch,
      runner: action.runner,
      workspaceMode: action.workspace_mode ?? undefined,
      costCapUsd: (action.cost_cap_cents / 100).toString(),
      wallClockMin: (action.wall_clock_cap_ms / 60_000).toString(),
      policy: action.auto_bypass_policy ?? null,
      model: action.model ?? undefined,
      permissionMode: action.permission_mode ?? undefined,
      effort: action.effort ?? undefined,
    }),
    [action],
  );
  const state = useJobComposerState({ repo, initial });
  const [submitting, setSubmitting] = useState(false);
  const [submitErr, setSubmitErr] = useState<string | null>(null);

  // `JobComposer` reads `state.info` to populate the runner dropdown.
  // The dialog shell fetches `/server/info` on each open; the card
  // mirrors that — one fetch per mount surfaces a server restarted with
  // `--enable-claude` between the planner's draft and the user's review.
  useEffect(() => {
    let cancelled = false;
    rpc
      .serverInfo()
      .then((i) => {
        if (cancelled) return;
        state.setInfo(i);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setSubmitErr(
          `could not load runner list: ${e instanceof Error ? e.message : String(e)}`,
        );
      });
    return () => {
      cancelled = true;
    };
    // `state` is held by the caller; we only want to fire this on mount
    // / rpc change. Re-running on every `state` identity flip would
    // cancel + re-fetch on each keystroke.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rpc]);

  const onConfirmClick = async () => {
    if (!state.canSubmit || submitting) return;
    setSubmitting(true);
    setSubmitErr(null);
    try {
      await onConfirm(composerToSubmitArgs(state));
    } catch (e: unknown) {
      setSubmitErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="mt-1 flex flex-col gap-2 rounded border border-border/40 bg-background/40 p-2 text-xs">
      <JobComposer state={state} hideRunImmediately />
      <details className="text-muted-foreground">
        <summary className="cursor-pointer select-none">prompt</summary>
        <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-muted/40 p-2">
          {action.prompt}
        </pre>
      </details>
      {submitErr && (
        <div className="text-destructive">{submitErr}</div>
      )}
      <div className="mt-1 flex justify-end gap-2">
        <Button
          size="sm"
          variant="ghost"
          onClick={onCancel}
          disabled={submitting}
          aria-label="Cancel action"
        >
          Cancel
        </Button>
        <Button
          size="sm"
          onClick={() => void onConfirmClick()}
          disabled={!state.canSubmit || submitting}
          aria-label="Confirm action"
        >
          {submitting ? "Submitting…" : "Confirm"}
        </Button>
      </div>
    </div>
  );
}

// Structured preview for the `draft_job` action card. The card's
// `content` already carries a human summary, but the draft-review card
// is the one place the user is committing to a multi-field mutation —
// rendering the proposed fields as a table makes the review honest.
// Optional fields (`workspace_mode`, `model`, …) are omitted when
// `null` so the table reflects exactly what `submit_job` will see.
function DraftJobPreview({
  action,
}: {
  action: Extract<AssistantAction, { tool: "draft_job" }>;
}) {
  const rows: Array<[string, string]> = [
    ["repo", action.repo_id],
    ["runner", action.runner],
    ["branch", action.branch],
    ["cost cap", `${action.cost_cap_cents}¢`],
    ["wall clock cap", `${action.wall_clock_cap_ms}ms`],
  ];
  if (action.workspace_mode) rows.push(["workspace", action.workspace_mode]);
  if (action.model) rows.push(["model", action.model]);
  if (action.permission_mode) rows.push(["permission", action.permission_mode]);
  if (action.effort) rows.push(["effort", action.effort]);
  return (
    <div className="mt-1 flex flex-col gap-1 rounded border border-border/40 bg-background/40 p-2 text-xs">
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
        {rows.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="font-mono text-muted-foreground">{k}</dt>
            <dd className="truncate font-mono">{v}</dd>
          </div>
        ))}
      </dl>
      <details className="text-muted-foreground">
        <summary className="cursor-pointer select-none">prompt</summary>
        <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-muted/40 p-2">
          {action.prompt}
        </pre>
      </details>
    </div>
  );
}

// Short `tool:method_name` label for the card header so the user can
// see what they are about to run without parsing the human summary.
function actionLabel(action: AssistantAction): string {
  return `tool:${action.tool}`;
}

// Structured preview for the `edit_scope` action card. Fetches the
// current on-disk file via `read_job_file` so the user can review the
// unified diff (computed in the browser to avoid round-tripping the
// proposed body twice) before confirming the rewrite. The diff is
// presentation-only — the server recomputes its own diff when it
// emits the trailing `Tool` message, which the user trusts because
// the server is the one that actually wrote the file.
function EditScopePreview({
  action,
}: {
  action: Extract<AssistantAction, { tool: "edit_scope" }>;
}) {
  const rpc = useRpc();
  const [current, setCurrent] = useState<string | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setCurrent(null);
    setLoadErr(null);
    void rpc
      .call("read_job_file", {
        job_id: action.job_id,
        filename: action.filename,
      })
      .then((res) => {
        if (cancelled) return;
        setCurrent(res.content);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        // NotFound is expected for first-time writes; render as an
        // empty current body so the diff shows every line as an
        // addition, mirroring how the server treats it.
        const msg = e instanceof Error ? e.message : String(e);
        if (msg.toLowerCase().includes("not found")) {
          setCurrent("");
        } else {
          setLoadErr(msg);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [rpc, action.job_id, action.filename]);

  const diffLines =
    current === null
      ? null
      : unifiedDiffLines(current, action.new_content);

  return (
    <div className="mt-1 flex flex-col gap-2 rounded border border-border/40 bg-background/40 p-2 text-xs">
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
        <dt className="font-mono text-muted-foreground">job</dt>
        <dd className="truncate font-mono">{action.job_id}</dd>
        <dt className="font-mono text-muted-foreground">file</dt>
        <dd className="truncate font-mono">{action.filename}</dd>
        <dt className="font-mono text-muted-foreground">new size</dt>
        <dd className="font-mono">{action.new_content.length} bytes</dd>
      </dl>
      <div className="flex items-center justify-end">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => navigate(`/jobs/${action.job_id}`)}
          aria-label="Open in editor"
        >
          Open in editor
        </Button>
      </div>
      {loadErr ? (
        <div className="text-destructive">
          Could not read current file: {loadErr}
        </div>
      ) : diffLines === null ? (
        <div className="text-muted-foreground">Loading current file…</div>
      ) : (
        <details open className="text-muted-foreground">
          <summary className="cursor-pointer select-none">unified diff</summary>
          <pre className="mt-1 max-h-64 overflow-auto rounded bg-muted/40 p-2 font-mono text-[11px] leading-tight">
            {diffLines.map((l, i) => (
              <div
                key={i}
                className={cn(
                  l.kind === "add" && "text-emerald-500",
                  l.kind === "del" && "text-destructive",
                  l.kind === "header" && "font-semibold text-muted-foreground",
                )}
              >
                {l.text}
              </div>
            ))}
          </pre>
        </details>
      )}
    </div>
  );
}

type DiffLine = { kind: "add" | "del" | "eq" | "header"; text: string };

// Browser-side LCS unified diff. Mirrors the Rust `unified_diff` so the
// preview matches what the server emits on confirm; the runtime
// recomputes its own diff for the `Tool` message rather than trusting
// this one. Kept inside this file because nothing else needs it —
// promoting to `@/lib/diff` would be premature.
function unifiedDiffLines(oldText: string, newText: string): DiffLine[] {
  const a = splitLines(oldText);
  const b = splitLines(newText);
  const n = a.length;
  const m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] =
        a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out: DiffLine[] = [
    { kind: "header", text: "--- current" },
    { kind: "header", text: "+++ proposed" },
  ];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "eq", text: " " + a[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ kind: "del", text: "-" + a[i] });
      i++;
    } else {
      out.push({ kind: "add", text: "+" + b[j] });
      j++;
    }
  }
  while (i < n) {
    out.push({ kind: "del", text: "-" + a[i++] });
  }
  while (j < m) {
    out.push({ kind: "add", text: "+" + b[j++] });
  }
  return out;
}

function splitLines(s: string): string[] {
  if (s.length === 0) return [];
  const lines = s.split("\n");
  // `split("\n")` leaves a trailing empty string when the source ends
  // with a newline; drop it so the diff doesn't show a phantom blank
  // addition at the end of every file.
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

// `Tool`-role messages carry the structured result of a confirmed
// action. The structured payload sits on `meta_json`; the
// human-readable summary is `content`. We render the summary plus a
// foldable raw-JSON block so an action's outcome is inspectable
// without a separate developer surface.
function ToolResultView({ message }: { message: AssistantMessage }) {
  return (
    <div className="flex justify-start">
      <div className="flex w-full max-w-[85%] flex-col gap-1 rounded-md border border-border/60 bg-card px-3 py-2 text-sm">
        <span className="text-[11px] font-mono uppercase tracking-wide text-muted-foreground">
          tool result
        </span>
        <div className="whitespace-pre-wrap">{message.content}</div>
        {message.meta_json && (
          <details className="text-xs text-muted-foreground">
            <summary className="cursor-pointer select-none">payload</summary>
            <pre className="mt-1 max-h-48 overflow-auto rounded bg-muted/40 p-2 text-[11px]">
              {prettyJson(message.meta_json)}
            </pre>
          </details>
        )}
      </div>
    </div>
  );
}

// PS7 attachment-card decoder. Same shape as `parseActionCard` but
// discriminates on `kind === "attachment_card"`. Returns null for any
// other meta payload (action cards, future variants, malformed JSON)
// so callers can fall through to the next renderer.
function parseAttachmentCard(
  raw: string | null,
): AssistantAttachmentCard | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<AssistantAttachmentCard>;
    if (
      parsed
      && parsed.kind === "attachment_card"
      && Array.isArray(parsed.items)
    ) {
      return parsed as AssistantAttachmentCard;
    }
    return null;
  } catch {
    return null;
  }
}

type AttachmentCardViewProps = {
  message: AssistantMessage;
  card: AssistantAttachmentCard;
};

// Reconciled-attachment card. The server already enforced cross-thread
// + dangling-id checks (`rpc::attachment::build_attachment_card`); the
// renderer trusts the items it sees and shows filename, mime, size,
// plus a placeholder inline preview slot for image/PDF mimes. The
// download link surface is deferred until the runtime exposes an HTTP
// route for `assistant_attachments/<id>` -- until then the card
// documents the file the tool produced so the user can locate it on
// disk via the existing attachments folder convention.
function AttachmentCardView({ message: _message, card }: AttachmentCardViewProps) {
  return (
    <div className="flex justify-start">
      <div className="flex w-full max-w-[85%] flex-col gap-2 rounded-md border border-border/60 bg-card px-3 py-2 text-sm">
        <span className="text-[11px] font-mono uppercase tracking-wide text-muted-foreground">
          {card.items.length === 1 ? "attachment" : `${card.items.length} attachments`}
        </span>
        <ul className="flex flex-col gap-1">
          {card.items.map((item) => (
            <li
              key={item.attachment_id}
              className="flex items-center gap-2 rounded border border-border/40 bg-muted/30 px-2 py-1"
            >
              <span className="truncate font-medium">{item.filename}</span>
              <span className="text-xs text-muted-foreground">
                {item.mime ?? "unknown type"} · {formatBytes(item.size_bytes)}
              </span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kib = bytes / 1024;
  if (kib < 1024) return `${kib.toFixed(1)} KiB`;
  return `${(kib / 1024).toFixed(1)} MiB`;
}

function prettyJson(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}
