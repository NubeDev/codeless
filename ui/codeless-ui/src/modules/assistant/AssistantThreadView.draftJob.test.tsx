// W2 round-trip: planner-seeded `draft_job` action card -> user edits
// a composer field -> Confirm dispatches `submit_job` with the edited
// value. The wire from a planner proposal to a fully-spec'd job is the
// parity guarantee `SCOPE-ASSISTANT-PARITY` §W2 exists to defend; this
// test fails if the seeded card stops accepting user edits or if the
// composer's `composerToSubmitArgs` mapping diverges (e.g. forgets to
// translate the cost-cap USD field into cents on the wire).
//
// The card mounts a real `JobComposer` against a real `MockRpcClient`
// so the test exercises `useJobComposerState({ initial })`,
// `composerToSubmitArgs`, and the assistant view's
// `onConfirmDraftJob` handler exactly as production would.

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
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  ServerInfo,
  SubmitJobArgs,
} from "@/lib/rpc";

import { AssistantThreadView } from "./AssistantThreadView";

// MockRpcClient predates the assistant surface and throws
// `unhandled method` for the four assistant RPCs. Subclass to canned
// responses; everything else (`list_repos`, `submit_job`,
// `subscribe`, …) falls through to the base class unchanged. The base
// `submit_job` push into `this.jobs` is what the test reads back to
// assert on the edited wire shape.
class AssistantStubMock extends MockRpcClient {
  public submittedJobs: SubmitJobArgs[] = [];
  private cards: AssistantMessage[];

  constructor(cards: AssistantMessage[]) {
    super();
    this.cards = cards;
  }

  async call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    if (method === "list_assistant_messages") {
      return { messages: this.cards } as RpcResultOf<M>;
    }
    if (method === "submit_job") {
      this.submittedJobs.push(args as SubmitJobArgs);
    }
    return super.call(method, args);
  }

  // ServerInfo must advertise the planner's `runner: "claude"` so the
  // `JobComposer` finds matching `RUNNER_CAPS` and renders the
  // Cost cap field. The base mock only publishes `mock + claude` with
  // mock as default — we override to make `claude` the default so the
  // initial composer state matches the planner-seeded value.
  async serverInfo(): Promise<ServerInfo> {
    return {
      version: "test",
      runners: [
        { id: "claude", default: true },
        { id: "mock", default: false },
      ],
      fs_root: null,
      worktree_root: null,
      claude: null,
      available_cli_runners: [],
    };
  }
}

const THREAD_ID = "01HMOCKTHREAD0000000000000000" as AssistantThreadId;
const CARD_MESSAGE_ID = "01HMOCKCARD000000000000000000" as AssistantMessageId;

function threadFixture(): AssistantThread {
  return {
    id: THREAD_ID,
    title: "draft-job test",
    persona_id: "builtin:general",
    created_at: 0,
    updated_at: 0,
  };
}

// Build the persisted `Assistant`-role row the planner would have
// written through `append_assistant_message` — `meta_json` carries the
// `action_card` document the renderer discriminates on. Cost cap and
// branch are the load-bearing fields for this test; the rest match
// what the planner ships today (see `AssistantAction { tool: draft_job }`).
function plannerSeededDraftJobCard(repoId: string): AssistantMessage {
  const card: AssistantActionCard = {
    kind: "action_card",
    status: "pending",
    action: {
      tool: "draft_job",
      repo_id: repoId,
      prompt: "implement the W2b round-trip test",
      runner: "claude",
      branch: "codeless/parity-w2b",
      cost_cap_cents: 500,
      wall_clock_cap_ms: 30 * 60_000,
      workspace_mode: "in-repo",
      auto_bypass_policy: null,
    },
  };
  return {
    id: CARD_MESSAGE_ID,
    thread_id: THREAD_ID,
    role: "assistant",
    content: "Drafted a job for review:",
    meta_json: JSON.stringify(card),
    created_at: 0,
  };
}

afterEach(() => cleanup());

describe("AssistantThreadView draft_job round-trip", () => {
  it("submits the user-edited cost cap, not the planner's seeded value", async () => {
    // The base mock seeds two repos; pick the first and re-use its id
    // both as the card's `repo_id` and as the value we expect to see
    // verbatim on the submit_job wire shape — without this match the
    // composer's `list_repos` lookup would mark the action's repo as
    // "no longer registered" and the form would never mount.
    const seedClient = new MockRpcClient();
    const { repos } = await seedClient.call("list_repos", {});
    const repoId = repos[0].id;

    const card = plannerSeededDraftJobCard(repoId);
    const client = new AssistantStubMock([card]);

    render(
      <RpcProvider client={client}>
        <AssistantThreadView thread={threadFixture()} />
      </RpcProvider>,
    );

    // The card resolves its repo via list_repos and then loads
    // serverInfo() before the composer body paints. Wait for the
    // cost-cap field — that's the leaf React commit we care about.
    const costInput = (await waitFor(
      () => screen.getByLabelText(/cost cap/i),
    )) as HTMLInputElement;

    // Planner seeded 500¢ -> composer renders "5" in USD. The branch
    // is the strongest evidence the seeded values reached the form;
    // assert on both so a regression that drops the `initial` prop
    // surfaces here instead of as a silent default-cap submit.
    expect(costInput.value).toBe("5");
    expect((screen.getByLabelText(/branch/i) as HTMLInputElement).value).toBe(
      "codeless/parity-w2b",
    );

    // The user disagrees with the planner's cap and bumps it to $10
    // before confirming. `composerToSubmitArgs` is what converts the
    // string back into cents on the wire — the assertion below is
    // that conversion plus the editable-card wiring working in concert.
    fireEvent.change(costInput, { target: { value: "10" } });
    expect(costInput.value).toBe("10");

    fireEvent.click(screen.getByRole("button", { name: /confirm action/i }));

    await waitFor(() => {
      expect(client.submittedJobs).toHaveLength(1);
    });

    const submitted = client.submittedJobs[0];
    expect(submitted.repo_id).toBe(repoId);
    expect(submitted.runner).toBe("claude");
    expect(submitted.branch).toBe("codeless/parity-w2b");
    // The headline assertion: the user's edit (1000¢) reached
    // submit_job, not the planner's seed (500¢).
    expect(submitted.cost_cap_cents).toBe(1000);
    expect(submitted.wall_clock_cap_ms).toBe(30 * 60_000);
    // `start_immediately` stays false because the assistant card
    // hides the "Run immediately" checkbox via `hideRunImmediately`;
    // a regression that re-shows the toggle would let a stray click
    // queue the job without review.
    expect(submitted.start_immediately).toBe(false);

    // The card flips to `confirmed` in-place so a re-render of the
    // transcript does not surface the buttons that fired the RPC.
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: /confirm action/i }),
      ).toBeNull();
    });
    // The synthetic tool row that the assistant view appends after a
    // successful submit references the newly-drafted job id so the
    // transcript reflects what happened without a refetch.
    await waitFor(() => {
      expect(screen.getByText(/Drafted job/i)).toBeInTheDocument();
    });
  });

  // W3c: the planner's `auto_bypass_policy` reaches the composer's
  // policy picker. The composer reads the seed via `pickerFromPolicy`
  // in `useJobComposerState`; this test asserts the seeded value
  // surfaces on the rendered picker, so a regression that loses the
  // mapping at the `JobComposerInitial` boundary fails here instead
  // of silently submitting `auto_bypass_policy: null`.
  it("seeds the policy picker from the planner's auto_bypass_policy", async () => {
    const seedClient = new MockRpcClient();
    const { repos } = await seedClient.call("list_repos", {});
    const repoId = repos[0].id;

    const card = plannerSeededDraftJobCard(repoId);
    const action = JSON.parse(card.meta_json!);
    action.action.auto_bypass_policy = { type: "long-term" };
    card.meta_json = JSON.stringify(action);

    const client = new AssistantStubMock([card]);

    render(
      <RpcProvider client={client}>
        <AssistantThreadView thread={threadFixture()} />
      </RpcProvider>,
    );

    // The picker is a shadcn Select whose trigger reflects the chosen
    // preset's label. Waiting on /Long-term/ asserts both that the
    // composer mounted (so the trigger exists) and that the planner's
    // seed propagated through `useJobComposerState` to the visible
    // label, which is what users see and act on.
    await waitFor(() => {
      expect(screen.getByText(/Long-term/i)).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /confirm action/i }));

    await waitFor(() => {
      expect(client.submittedJobs).toHaveLength(1);
    });
    // The picker round-trips the seed back onto the wire via
    // `composerToSubmitArgs` — confirming without editing must hand
    // the planner's policy to `submit_job` verbatim. A regression
    // that drops the value at the composer-state boundary surfaces
    // as `null` here.
    expect(client.submittedJobs[0].auto_bypass_policy).toEqual({
      type: "long-term",
    });
  });
});
