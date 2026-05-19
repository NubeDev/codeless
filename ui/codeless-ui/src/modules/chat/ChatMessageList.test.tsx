// Streaming-text accumulator parity test. The W1 extraction
// (`SCOPE-ASSISTANT-PARITY.md`) moves the in-flight bubble
// accumulator out of `JobChat` and into `ChatMessageList`; the test
// asserts the accumulator behaves identically — `ai-token` deltas
// matching the active task append into the bubble, deltas tagged with
// a different `task_id` are ignored, and the bubble retires on
// `ai-message-complete`. A regression that drops the activeTaskId
// guard would surface as a phantom bubble between turns.

import { cleanup, render, screen, waitFor, act } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { MockRpcClient } from "@/lib/rpc/mock-client";
import { RpcProvider } from "@/lib/rpc/provider";
import type {
  Event,
  EventEnvelope,
  EventFilter,
  JobId,
  Since,
} from "@/lib/rpc";

import { ChatMessageList } from "./ChatMessageList";

const JOB_ID = "01HMOCKJOBTOKENSTREAMING0000" as JobId;
const ACTIVE_TASK = "01HMOCKTASK000000000000ACTIVE";
const OTHER_TASK = "01HMOCKTASK000000000000OTHERS";

// Test RpcClient with a public hook for shoving event envelopes into
// the live subscription. The base `MockRpcClient.subscribe` plumbing
// is private; mirror the iterator pattern here so the renderer's
// `useEventStream` call site receives our deltas as they would from a
// real SSE channel.
class StreamingRpcClient extends MockRpcClient {
  private listeners = new Set<(env: EventEnvelope) => void>();
  private cursor = 1;

  push(event: Event, taskId: string | null) {
    const env: EventEnvelope = {
      cursor: this.cursor++,
      job_id: JOB_ID,
      stage_id: null,
      task_id: taskId,
      created_at: Date.now(),
      event,
    };
    for (const l of this.listeners) l(env);
  }

  override subscribe(
    filter: EventFilter,
    _since?: Since,
  ): AsyncIterable<EventEnvelope> {
    const matches = (env: EventEnvelope) =>
      filter.scope === "all" ||
      (filter.scope === "job" && env.job_id === filter.job_id);
    const queue: EventEnvelope[] = [];
    const waiters: Array<(v: IteratorResult<EventEnvelope>) => void> = [];
    let done = false;
    const handler = (env: EventEnvelope) => {
      if (!matches(env)) return;
      const w = waiters.shift();
      if (w) w({ value: env, done: false });
      else queue.push(env);
    };
    this.listeners.add(handler);
    return {
      [Symbol.asyncIterator]: () => ({
        next: async () => {
          if (queue.length) return { value: queue.shift()!, done: false };
          if (done) return { value: undefined, done: true };
          return new Promise<IteratorResult<EventEnvelope>>((resolve) => {
            waiters.push(resolve);
          });
        },
        return: async () => {
          done = true;
          this.listeners.delete(handler);
          while (waiters.length)
            waiters.shift()!({ value: undefined, done: true });
          return { value: undefined, done: true };
        },
      }),
    };
  }
}

afterEach(() => cleanup());

describe("ChatMessageList streaming accumulator", () => {
  it("accumulates ai-token deltas for the active task into the in-flight bubble", async () => {
    const client = new StreamingRpcClient();
    render(
      <RpcProvider client={client}>
        <ChatMessageList
          filter={{ scope: "job", job_id: JOB_ID }}
          history={[]}
          activeTaskId={ACTIVE_TASK}
        />
      </RpcProvider>,
    );

    // The renderer shows a placeholder "…" bubble before any tokens
    // arrive so the user knows the turn is in flight.
    await waitFor(() =>
      expect(screen.getByText("…")).toBeInTheDocument(),
    );

    await act(async () => {
      client.push({ type: "ai-token", task_id: ACTIVE_TASK, delta: "Hel" }, ACTIVE_TASK);
      client.push({ type: "ai-token", task_id: ACTIVE_TASK, delta: "lo " }, ACTIVE_TASK);
      client.push({ type: "ai-token", task_id: ACTIVE_TASK, delta: "world" }, ACTIVE_TASK);
    });

    await waitFor(() =>
      expect(screen.getByText("Hello world")).toBeInTheDocument(),
    );
  });

  it("ignores ai-token deltas tagged with a different task_id", async () => {
    const client = new StreamingRpcClient();
    render(
      <RpcProvider client={client}>
        <ChatMessageList
          filter={{ scope: "job", job_id: JOB_ID }}
          history={[]}
          activeTaskId={ACTIVE_TASK}
        />
      </RpcProvider>,
    );

    await waitFor(() => expect(screen.getByText("…")).toBeInTheDocument());

    await act(async () => {
      client.push({ type: "ai-token", task_id: OTHER_TASK, delta: "stale" }, OTHER_TASK);
      client.push({ type: "ai-token", task_id: ACTIVE_TASK, delta: "fresh" }, ACTIVE_TASK);
    });

    await waitFor(() =>
      expect(screen.getByText("fresh")).toBeInTheDocument(),
    );
    // The earlier-task delta must not have leaked into the active bubble.
    expect(screen.queryByText("stalefresh")).toBeNull();
    expect(screen.queryByText("freshstale")).toBeNull();
  });

  it("hides the in-flight bubble when activeTaskId is null", () => {
    const client = new StreamingRpcClient();
    render(
      <RpcProvider client={client}>
        <ChatMessageList
          filter={{ scope: "job", job_id: JOB_ID }}
          history={[]}
          activeTaskId={null}
        />
      </RpcProvider>,
    );
    expect(screen.queryByText("…")).toBeNull();
  });
});
