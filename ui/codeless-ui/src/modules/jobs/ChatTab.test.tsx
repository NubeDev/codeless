// chat_tab_round_trips_a_post — the load-bearing claim of stage 6 of
// JOB-CHAT.md: the message input → `post_job_message` →
// `chat-message-appended` → render loop closes without touching the
// network. The fixture `RpcClient` records the call, fabricates the
// fan-out envelope the runtime would have published after the
// INSERT, and the rendered list picks up the appended row through
// the same SSE subscription a real browser uses. If a future drive-by
// edit re-adds an optimistic local append (the legacy `CHAT.md`
// shape), this test still passes — but if anything detaches the
// composer from the event-driven render path, it breaks loudly.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { RpcProvider } from "@/lib/rpc/provider";
import type { RpcClient } from "@/lib/rpc/client";
import type {
  ChatMessage,
  EventEnvelope,
  ServerInfo,
} from "@/lib/rpc/wire";
import type {
  EventFilter,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "@/lib/rpc/methods";

import { ChatTab, __resetChatCacheForTests } from "./ChatTab";

const JOB_ID = "01HMOCKJOB000000000000000000";

interface Recorded<M extends RpcMethod = RpcMethod> {
  method: M;
  args: RpcArgs<M>;
}

class FixtureRpcClient implements RpcClient {
  readonly calls: Recorded[] = [];
  history: ChatMessage[] = [];
  private subscribers = new Set<(env: EventEnvelope) => void>();
  private cursor = 1;

  preload(messages: ChatMessage[]): void {
    this.history = [...messages];
  }

  async call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    this.calls.push({ method, args } as Recorded<M>);

    if (method === "list_job_messages") {
      return { messages: [...this.history] } as RpcResultOf<M>;
    }

    if (method === "post_job_message") {
      const a = args as RpcArgs<"post_job_message">;
      const msg: ChatMessage = {
        id: `01HMOCKMSG${String(this.history.length).padStart(18, "0")}`,
        job_id: a.job_id,
        run_id: null,
        transport: a.transport,
        external_id: a.external_id ?? null,
        thread_key: a.thread_key ?? null,
        author: a.author,
        role: a.role ?? "user",
        body: a.body,
        metadata_json: a.metadata_json ?? null,
        created_at: 1_700_000_000_000 + this.history.length,
      };
      this.history.push(msg);
      this.publish({
        cursor: this.cursor++,
        job_id: a.job_id,
        stage_id: null,
        task_id: null,
        created_at: msg.created_at,
        event: { type: "chat-message-appended", job_id: a.job_id, message: msg },
      });
      return msg as RpcResultOf<M>;
    }

    throw new Error(`FixtureRpcClient: unhandled method ${method}`);
  }

  subscribe(
    _filter: EventFilter,
    _since?: Since,
  ): AsyncIterable<EventEnvelope> {
    // The hook used by ChatTab calls `subscribeWithState` rather than
    // this iterable form; provide a no-op implementation only to
    // satisfy the interface.
    return {
      [Symbol.asyncIterator]() {
        return {
          next: () => Promise.resolve({ value: undefined, done: true }),
        };
      },
    } as AsyncIterable<EventEnvelope>;
  }

  subscribeWithState(
    filter: EventFilter,
    _since: Since | undefined,
    onEvent: (env: EventEnvelope) => void,
    onState: (s: { state: "live"; since_ms: number; last_cursor: null }) => void,
  ): () => void {
    onState({ state: "live", since_ms: 0, last_cursor: null });
    const handler = (env: EventEnvelope) => {
      if (filter.scope === "all") {
        onEvent(env);
      } else if (filter.scope === "job" && env.job_id === filter.job_id) {
        onEvent(env);
      }
    };
    this.subscribers.add(handler);
    return () => {
      this.subscribers.delete(handler);
    };
  }

  publish(env: EventEnvelope): void {
    for (const fn of this.subscribers) fn(env);
  }

  async serverInfo(): Promise<ServerInfo> {
    return {
      runners: [],
      claude: { kind: "missing", message: null, version: null, path: null },
      fs_root: "/tmp",
      worktree_root: "/tmp",
      feature_flags: { runtime_supervisor_enabled: false },
      available_cli_runners: [],
    } as unknown as ServerInfo;
  }
}

function flushAsync(): Promise<void> {
  return act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("ChatTab", () => {
  afterEach(() => {
    cleanup();
    __resetChatCacheForTests();
  });

  it("rehydrates from list_job_messages on mount", async () => {
    const client = new FixtureRpcClient();
    client.preload([
      {
        id: "01HMOCKMSG000000000000000001",
        job_id: JOB_ID,
        run_id: null,
        transport: "telegram",
        external_id: "tg:1:7",
        thread_key: null,
        author: "alice",
        role: "user",
        body: "hi from telegram",
        metadata_json: null,
        created_at: 1,
      },
    ]);

    render(
      <RpcProvider client={client}>
        <ChatTab jobId={JOB_ID} />
      </RpcProvider>,
    );

    await flushAsync();

    const rendered = await screen.findAllByTestId("chat-tab-message");
    expect(rendered).toHaveLength(1);
    expect(rendered[0]).toHaveTextContent("hi from telegram");
  });

  it("chat_tab_round_trips_a_post: input → post_job_message → ChatMessageAppended → render", async () => {
    const client = new FixtureRpcClient();

    render(
      <RpcProvider client={client}>
        <ChatTab jobId={JOB_ID} author="ap" />
      </RpcProvider>,
    );

    await flushAsync();
    expect(screen.queryAllByTestId("chat-tab-message")).toHaveLength(0);

    const input = screen.getByTestId("chat-tab-input") as HTMLTextAreaElement;
    const send = screen.getByTestId("chat-tab-send");

    fireEvent.change(input, { target: { value: "hello world" } });
    fireEvent.click(send);

    await flushAsync();

    const posted = client.calls.find((c) => c.method === "post_job_message");
    expect(posted, "post_job_message must be called").toBeTruthy();
    const args = posted!.args as RpcArgs<"post_job_message">;
    expect(args).toMatchObject({
      job_id: JOB_ID,
      transport: "web",
      author: "ap",
      role: "user",
      body: "hello world",
    });

    const rendered = screen.getAllByTestId("chat-tab-message");
    expect(rendered).toHaveLength(1);
    expect(rendered[0]).toHaveTextContent("hello world");
    expect(rendered[0].getAttribute("data-role")).toBe("user");
    expect(rendered[0].getAttribute("data-transport")).toBe("web");

    expect((screen.getByTestId("chat-tab-input") as HTMLTextAreaElement).value)
      .toBe("");
  });

  it("appends a non-origin ChatMessageAppended (Telegram fan-out) without re-fetch", async () => {
    const client = new FixtureRpcClient();

    render(
      <RpcProvider client={client}>
        <ChatTab jobId={JOB_ID} />
      </RpcProvider>,
    );

    await flushAsync();

    act(() => {
      client.publish({
        cursor: 99,
        job_id: JOB_ID,
        stage_id: null,
        task_id: null,
        created_at: 1_700_000_500_000,
        event: {
          type: "chat-message-appended",
          job_id: JOB_ID,
          message: {
            id: "01HMOCKMSGEXT00000000000001",
            job_id: JOB_ID,
            run_id: null,
            transport: "telegram",
            external_id: "tg:42",
            thread_key: null,
            author: "bob",
            role: "user",
            body: "ping from telegram",
            metadata_json: null,
            created_at: 1_700_000_500_000,
          },
        },
      });
    });

    const rendered = screen.getAllByTestId("chat-tab-message");
    expect(rendered).toHaveLength(1);
    expect(rendered[0]).toHaveTextContent("ping from telegram");
    expect(rendered[0].getAttribute("data-transport")).toBe("telegram");

    // The ListJobMessages call ran once on mount; the inbound fan-out
    // must not provoke a second fetch.
    const listCount = client.calls.filter(
      (c) => c.method === "list_job_messages",
    ).length;
    expect(listCount).toBe(1);
  });

  it("dedups a redelivered ChatMessageAppended on the same MessageId", async () => {
    const client = new FixtureRpcClient();

    render(
      <RpcProvider client={client}>
        <ChatTab jobId={JOB_ID} />
      </RpcProvider>,
    );

    await flushAsync();

    const env: EventEnvelope = {
      cursor: 5,
      job_id: JOB_ID,
      stage_id: null,
      task_id: null,
      created_at: 1_700_000_600_000,
      event: {
        type: "chat-message-appended",
        job_id: JOB_ID,
        message: {
          id: "01HMOCKMSGDUP000000000000001",
          job_id: JOB_ID,
          run_id: null,
          transport: "web",
          external_id: null,
          thread_key: null,
          author: "ap",
          role: "user",
          body: "only once",
          metadata_json: null,
          created_at: 1_700_000_600_000,
        },
      },
    };
    act(() => {
      client.publish(env);
      client.publish(env);
    });

    const rendered = screen.getAllByTestId("chat-tab-message");
    expect(rendered).toHaveLength(1);
  });
});
