// W3c paused-job rule: the `set_policy` card surfaces a "Pause &
// confirm" affordance when the planner proposes a policy change
// against a Running / AwaitingReview row (`AUTO-BYPASS-DECISIONS.md`
// Q5). Clicking the affordance calls `pause_job` first so the row is
// in a status `set_job_policy` accepts before
// `confirm_assistant_action` dispatches the policy mutation. A
// Draft / Stopped / Paused row falls through to the standard
// Confirm path the runtime already accepts.
//
// The dispatcher's `set_policy` arm lives in
// `crates/codeless-runtime/src/rpc/assistant.rs` and calls
// `set_job_policy` directly; the UI's job is to honour the rule the
// RPC enforces so the user does not have to bounce between the
// assistant pane and the job page to apply a recommendation.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type {
  AssistantActionCard,
  AssistantMessage,
  AssistantMessageId,
  AssistantThread,
  AssistantThreadId,
  AutoBypassPolicy,
  Job,
  JobId,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
} from "@/lib/rpc";

import { AssistantThreadView } from "./AssistantThreadView";

// Tracks the order of mutating RPCs so the pause-then-confirm test
// asserts on sequence, not just presence. A future regression where
// `confirm_assistant_action` ran first would silently fail against an
// existence-only assertion.
class SetPolicyStubMock extends MockRpcClient {
  public calls: string[] = [];
  public pauseArgs: RpcArgs<"pause_job">[] = [];
  public confirmArgs: RpcArgs<"confirm_assistant_action">[] = [];
  private cards: AssistantMessage[];
  private jobs: Map<JobId, Job>;

  constructor(cards: AssistantMessage[], jobs: Job[]) {
    super();
    this.cards = cards;
    this.jobs = new Map(jobs.map((j) => [j.id, j]));
  }

  async call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    this.calls.push(method);
    if (method === "list_assistant_messages") {
      return { messages: this.cards } as RpcResultOf<M>;
    }
    if (method === "get_job") {
      const a = args as RpcArgs<"get_job">;
      const job = this.jobs.get(a.job_id);
      if (!job) {
        return super.call(method, args);
      }
      return job as RpcResultOf<M>;
    }
    if (method === "pause_job") {
      const a = args as RpcArgs<"pause_job">;
      this.pauseArgs.push(a);
      const job = this.jobs.get(a.job_id);
      if (job) {
        job.status = "paused";
        job.stop_reason = "user";
        job.ended_at = Date.now();
      }
      return null as RpcResultOf<M>;
    }
    if (method === "confirm_assistant_action") {
      const a = args as RpcArgs<"confirm_assistant_action">;
      this.confirmArgs.push(a);
      const cardMsg = this.cards.find((c) => c.id === a.message_id);
      if (!cardMsg) {
        throw new Error(`card ${a.message_id} not in fixture`);
      }
      const card = JSON.parse(cardMsg.meta_json ?? "{}") as AssistantActionCard;
      const confirmedCard: AssistantActionCard = { ...card, status: "confirmed" };
      const updatedMsg: AssistantMessage = {
        ...cardMsg,
        meta_json: JSON.stringify(confirmedCard),
      };
      const toolMsg: AssistantMessage = {
        id: `${cardMsg.id}-tool` as AssistantMessageId,
        thread_id: cardMsg.thread_id,
        role: "tool",
        content: "Set auto-bypass policy.",
        meta_json: JSON.stringify({ tool: "set_policy" }),
        created_at: Date.now(),
      };
      return {
        card: updatedMsg,
        tool_message: toolMsg,
      } as RpcResultOf<M>;
    }
    return super.call(method, args);
  }
}

const THREAD_ID = "01HSETPOLTHREAD0000000000000000" as AssistantThreadId;
const CARD_MESSAGE_ID = "01HSETPOLCARD000000000000000000" as AssistantMessageId;

function threadFixture(): AssistantThread {
  return {
    id: THREAD_ID,
    title: "set-policy test",
    persona_id: "builtin:general",
    created_at: 0,
    updated_at: 0,
  };
}

function setPolicyCard(
  jobId: JobId,
  policy: AutoBypassPolicy | null,
): AssistantMessage {
  const card: AssistantActionCard = {
    kind: "action_card",
    status: "pending",
    action: { tool: "set_policy", job_id: jobId, policy: policy ?? undefined },
  };
  return {
    id: CARD_MESSAGE_ID,
    thread_id: THREAD_ID,
    role: "assistant",
    content: "Switch to long-term so the next failure auto-bypasses.",
    meta_json: JSON.stringify(card),
    created_at: 0,
  };
}

function jobFixture(id: JobId, status: Job["status"]): Job {
  return {
    id,
    repo_id: "repo-1" as Job["repo_id"],
    status,
    stop_reason: null,
    template_yaml: null,
    prompt: null,
    runner: "claude",
    branch: "codeless/test",
    workspace_mode: "in-repo",
    worktree_path: null,
    cost_cap_cents: 500,
    wall_clock_cap_ms: 30 * 60_000,
    cost_cents: 0,
    model: null,
    permission_mode: null,
    effort: null,
    system_prompt: null,
    persona_id: null,
    auto_bypass_policy: null,
    pending_operator_comment: null,
    started_at: null,
    ended_at: null,
    created_at: 0,
  };
}

afterEach(() => cleanup());

describe("AssistantThreadView set_policy paused-job rule", () => {
  it("pauses the running job before confirming the policy change", async () => {
    const jobId = "job-running" as JobId;
    const card = setPolicyCard(jobId, { type: "long-term" });
    const job = jobFixture(jobId, "running");
    const client = new SetPolicyStubMock([card], [job]);

    render(
      <RpcProvider client={client}>
        <AssistantThreadView thread={threadFixture()} />
      </RpcProvider>,
    );

    // The panel renders after `get_job` resolves so the status-aware
    // button label reflects the actual row. The "Pause first" copy
    // is the load-bearing assertion that the rule is honoured —
    // surfacing the standard Confirm button against a running job
    // would dispatch into a guaranteed server-side Conflict.
    await waitFor(() => {
      expect(
        screen.getByRole("button", {
          name: /pause job then confirm policy change/i,
        }),
      ).toBeInTheDocument();
    });
    expect(screen.getByText(/runtime refuses a policy change/i)).toBeInTheDocument();
    // The standard Confirm button must not appear — the action card
    // chrome's default button row is suppressed when the panel owns
    // its own buttons.
    expect(
      screen.queryByRole("button", { name: /^confirm action$/i }),
    ).toBeNull();

    fireEvent.click(
      screen.getByRole("button", {
        name: /pause job then confirm policy change/i,
      }),
    );

    await waitFor(() => {
      expect(client.confirmArgs).toHaveLength(1);
    });
    // Sequence is load-bearing: confirming before the pause completes
    // would land on the server's Q5 guard and surface a Conflict
    // tool-message tagged Failed instead of a Confirmed card.
    const pauseIdx = client.calls.indexOf("pause_job");
    const confirmIdx = client.calls.indexOf("confirm_assistant_action");
    expect(pauseIdx).toBeGreaterThan(-1);
    expect(confirmIdx).toBeGreaterThan(pauseIdx);
    expect(client.pauseArgs[0].job_id).toBe(jobId);
    expect(client.confirmArgs[0].message_id).toBe(CARD_MESSAGE_ID);
  });

  it("shows current and proposed policy labels", async () => {
    const jobId = "job-paused" as JobId;
    const card = setPolicyCard(jobId, { type: "quick" });
    const job = jobFixture(jobId, "paused");
    job.auto_bypass_policy = { type: "best-judgement" };
    const client = new SetPolicyStubMock([card], [job]);

    render(
      <RpcProvider client={client}>
        <AssistantThreadView thread={threadFixture()} />
      </RpcProvider>,
    );

    // Preset labels come from `POLICY_PRESETS` so a label-copy change
    // in the picker module reaches the chat preview without a paired
    // edit here.
    await waitFor(() => {
      expect(screen.getByText(/Best judgement/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/Quick/i)).toBeInTheDocument();
  });

  it("offers a standard Confirm for a Paused job and dispatches without pausing", async () => {
    const jobId = "job-paused" as JobId;
    const card = setPolicyCard(jobId, { type: "cheap" });
    const job = jobFixture(jobId, "paused");
    const client = new SetPolicyStubMock([card], [job]);

    render(
      <RpcProvider client={client}>
        <AssistantThreadView thread={threadFixture()} />
      </RpcProvider>,
    );

    const confirmBtn = await waitFor(() =>
      screen.getByRole("button", { name: /^confirm action$/i }),
    );
    // No pause-affordance copy on a Paused row — the warning belongs
    // only to statuses the runtime would actually refuse.
    expect(
      screen.queryByText(/runtime refuses a policy change/i),
    ).toBeNull();

    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(client.confirmArgs).toHaveLength(1);
    });
    expect(client.pauseArgs).toHaveLength(0);
    expect(client.confirmArgs[0].message_id).toBe(CARD_MESSAGE_ID);
  });

  it("disables Confirm with a typed reason for a Queued job", async () => {
    const jobId = "job-queued" as JobId;
    const card = setPolicyCard(jobId, { type: "long-term" });
    const job = jobFixture(jobId, "queued");
    const client = new SetPolicyStubMock([card], [job]);

    render(
      <RpcProvider client={client}>
        <AssistantThreadView thread={threadFixture()} />
      </RpcProvider>,
    );

    // Queued rows cannot be paused (pause_job rejects) and cannot have
    // their policy set (set_job_policy rejects). The panel surfaces a
    // typed reason and disables Confirm rather than dispatching into a
    // guaranteed Conflict.
    const confirmBtn = await waitFor(() =>
      screen.getByRole("button", { name: /^confirm action$/i }),
    );
    expect(confirmBtn).toBeDisabled();
    expect(
      screen.getByText(/cannot be changed on a queued job/i),
    ).toBeInTheDocument();
  });
});
