// SCOPE-ASSISTANT-PARITY §W1d render-time guarantee: for the same
// projected prose rows, `JobChat` and `AssistantThreadView` produce
// identical message-list DOM. Both surfaces mount `ChatMessageList`
// for the body — the value the W1 refactor delivers — and this test
// exists to fail loudly if a future change in one wrapper grows a
// sibling bubble renderer, drops a row through a different chrome
// path, or projects the timestamp differently. The streaming
// accumulator, tool-call interleave, and lifecycle dividers are
// exercised in `ChatMessageList.test.tsx`; here we pin the
// no-event, no-streaming baseline so a swap in `ChatBubble`'s output
// cannot silently regress only one of the two surfaces.

import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type {
  AssistantMessage,
  AssistantMessageId,
  AssistantThread,
  AssistantThreadId,
  Job,
  JobId,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
} from "@/lib/rpc";

import { AssistantThreadView } from "../assistant/AssistantThreadView";
import { JobChat } from "../jobs/RunPane";

const JOB_ID = "01HMOCKJOBPARITY00000000ROWS" as JobId;
const THREAD_ID = "01HMOCKTHREADPARITY00000ROWS" as AssistantThreadId;

// `Date(this).toISOString()` round-trips to a stable string both
// surfaces can quote into their row projections. JobChat reads the
// ts back from `CHAT.md` verbatim through `parseChatMarkdown`;
// `AssistantThreadView` projects via `new Date(created_at).toISOString()`.
// Both arrive at the same string, so `ChatBubble`'s `shortTime`
// produces the same rendered text in both DOMs.
const TS_MS = Date.UTC(2026, 4, 18, 7, 30, 0);
const TS_ISO = new Date(TS_MS).toISOString();

// Three prose rows alternating roles. The assistant turn carries
// markdown that exercises Streamdown's headings/code-fence paths so
// a regression that swaps the renderer for a plain text node on one
// side surfaces as differing per-row HTML rather than a subtle
// styling drift.
const ROWS: Array<{ role: "user" | "assistant"; text: string }> = [
  { role: "user", text: "what changed in `lib/foo.ts`?" },
  {
    role: "assistant",
    text:
      "Added a **getter** for `bar`; the diff:\n\n```ts\nexport function bar() { return 1 }\n```",
  },
  { role: "user", text: "ok, ship it" },
];

function jobFixture(): Job {
  return {
    id: JOB_ID,
    repo_id: "01HMOCKREPOPARITY0000000ROWS",
    status: "running",
    stop_reason: null,
    template_yaml: "name: parity\n",
    prompt: "noop",
    runner: "mock",
    branch: "feat/parity",
    workspace_mode: "in-repo",
    // Null worktree skips JobChat's `fs_stat` probe so the test does
    // not have to wire a banner-suppression handler.
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
    created_at: TS_MS,
  };
}

function threadFixture(): AssistantThread {
  return {
    id: THREAD_ID,
    title: "parity",
    persona_id: "builtin:general",
    created_at: TS_MS,
    updated_at: TS_MS,
  };
}

// Mirrors the format `renderChatMarkdown` writes (`## role @ ts`
// headings) so JobChat's parser produces `ChatMessage` rows whose
// `ts` string matches what AssistantThreadView projects from
// `created_at`. Diverge either side of this — a header reformatted in
// the runtime, a projection that strips milliseconds — and the
// parity assertion fires.
function chatMarkdown(): string {
  const out: string[] = ["# Chat for this job", ""];
  for (const m of ROWS) {
    out.push(`## ${m.role} @ ${TS_ISO}`);
    out.push("");
    out.push(m.text);
    out.push("");
  }
  return out.join("\n");
}

function assistantRows(): AssistantMessage[] {
  return ROWS.map((m, i) => ({
    id: `01HMOCKMSGROW${i.toString().padStart(15, "0")}` as AssistantMessageId,
    thread_id: THREAD_ID,
    role: m.role,
    content: m.text,
    // `meta_json: null` flags the row as plain prose; both
    // `parseActionCard` and `parseAttachmentCard` short-circuit on
    // null so `MessageBubble` returns null and `ChatMessageList`
    // falls through to the default `ChatBubble` — the path this test
    // exists to pin.
    meta_json: null,
    created_at: TS_MS,
  }));
}

class JobStub extends MockRpcClient {
  async call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    if (method === "read_job_file") {
      return { content: chatMarkdown() } as RpcResultOf<M>;
    }
    return super.call(method, args);
  }
}

class AssistantStub extends MockRpcClient {
  async call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    if (method === "list_assistant_messages") {
      return { messages: assistantRows() } as RpcResultOf<M>;
    }
    return super.call(method, args);
  }
}

afterEach(() => cleanup());

describe("CommonChat parity: JobChat vs AssistantThreadView", () => {
  it("renders identical message-list DOM for the same prose history", async () => {
    const jobView = render(
      <RpcProvider client={new JobStub()}>
        <JobChat job={jobFixture()} />
      </RpcProvider>,
    );

    const assistantView = render(
      <RpcProvider client={new AssistantStub()}>
        <AssistantThreadView thread={threadFixture()} />
      </RpcProvider>,
    );

    const jobItems = () =>
      Array.from(jobView.container.querySelectorAll("ul > li"));
    const assistantItems = () =>
      Array.from(assistantView.container.querySelectorAll("ul > li"));

    await waitFor(() => {
      expect(jobItems()).toHaveLength(ROWS.length);
      expect(assistantItems()).toHaveLength(ROWS.length);
    });

    // Per-row comparison keeps the failure message surgical when the
    // divergence is in a single bubble; comparing the whole `<ul>`
    // would point at "the message list" without naming the row that
    // drifted. The `<ul>` itself differs by `className` (the
    // assistant wrapper passes a custom container class) — this test
    // is about the rendered message rows, not the scroll container,
    // so we walk `li`s and compare their `outerHTML`.
    for (let i = 0; i < ROWS.length; i++) {
      const j = jobItems()[i];
      const a = assistantItems()[i];
      expect(a.outerHTML).toBe(j.outerHTML);
    }
  });
});
