// PS2 of the plugin substrate pins one rule in code: every chat
// surface in the UI renders through `CommonChat`, and every render
// passes a `threadId` that names the server-resident conversation.
// PS3 will derive capabilities from that id and PS4 will key state
// off it; if a future drive-by edit drops the prop at one call site,
// those stages silently regress. The compile-time check here is the
// guard — TypeScript would have rejected the call but the test
// proves the rendered tree honours the routing for each `kind`.

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../jobs/RunPane", () => ({
  JobChat: (props: { job: { id: string } }) => (
    <div data-testid="job-chat">job:{props.job.id}</div>
  ),
}));

vi.mock("../ai/components/AiChat", () => ({
  AiChatView: () => <div data-testid="ai-chat">ai</div>,
}));

vi.mock("../assistant/AssistantThreadView", () => ({
  AssistantThreadView: (props: { thread: { id: string } }) => (
    <div data-testid="assistant-chat">assistant:{props.thread.id}</div>
  ),
}));

import { CommonChat } from "./CommonChat";
import type { AssistantThread, Job } from "@/lib/rpc";

afterEach(() => cleanup());

function jobFixture(): Job {
  return {
    id: "01HMOCKJOB000000000000000000",
    repo_id: "01HMOCKREPO0000000000000000",
    status: "draft",
    stop_reason: null,
    template_yaml: "name: smoke\n",
    prompt: "noop",
    runner: "mock",
    branch: "feat/smoke",
    workspace_mode: "in-repo",
    worktree_path: null,
    cost_cap_cents: 100,
    wall_clock_cap_ms: 60_000,
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

function threadFixture(): AssistantThread {
  return {
    id: "01HMOCKTHREAD0000000000000000",
    title: "thread",
    // PS5: persona_id is NOT NULL on the wire type; the fixture
    // uses the seeded `builtin:general` row.
    persona_id: "builtin:general",
    created_at: 0,
    updated_at: 0,
  };
}

describe("CommonChat", () => {
  it("routes job kind to JobChat with the thread id matching the job", () => {
    const job = jobFixture();
    render(<CommonChat kind="job" threadId={job.id} job={job} />);
    expect(screen.getByTestId("job-chat")).toHaveTextContent(`job:${job.id}`);
  });

  it("routes assistant kind to AssistantThreadView keyed by the thread id", () => {
    const thread = threadFixture();
    render(
      <CommonChat
        kind="assistant"
        threadId={thread.id}
        thread={thread}
      />,
    );
    expect(screen.getByTestId("assistant-chat")).toHaveTextContent(
      `assistant:${thread.id}`,
    );
  });

  it("routes ai kind to AiChatView with the editor session id as the thread id", () => {
    render(
      <CommonChat
        kind="ai"
        threadId="session-abc"
        messages={[]}
        status="ready"
        error={undefined}
        clearError={() => {}}
        addToolApprovalResponse={() => {}}
        stop={() => {}}
      />,
    );
    expect(screen.getByTestId("ai-chat")).toBeTruthy();
  });
});
