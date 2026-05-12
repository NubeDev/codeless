// Browser + mobile transport. REST POST per call, SSE for the event
// stream. The exact URL shapes here will be matched by `codeless-server`
// (Phase 3) — when that lands, this file is the contract the server
// must conform to.
//
// Auth: bearer token sent as `Authorization: Bearer <token>` on REST
// calls. SSE has no header API in browsers, so the token is passed as
// `?token=` query param; revisit when the hosted auth story is firm
// (cookie-based session is the likely Phase 7 answer).

import type { RpcClient } from "./client";
import { RpcError } from "./error";
import type {
  EventFilter,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "./methods";
import type { EventEnvelope } from "./wire";

export interface HttpSseClientConfig {
  // Origin of the codeless-server, e.g. `"https://core.example.com"`.
  // No trailing slash.
  baseUrl: string;
  // Bearer token; `null` for unauthenticated dev runs against a local core.
  token: string | null;
}

export class HttpSseClient implements RpcClient {
  constructor(private readonly cfg: HttpSseClientConfig) {}

  async call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    const headers: Record<string, string> = {
      "content-type": "application/json",
    };
    if (this.cfg.token) headers["authorization"] = `Bearer ${this.cfg.token}`;

    const res = await fetch(`${this.cfg.baseUrl}/rpc/${method}`, {
      method: "POST",
      headers,
      body: JSON.stringify(args),
    });

    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw RpcError.fromHttpStatus(res.status, text || res.statusText);
    }
    // `null` results come back as `null` JSON; await res.json() handles
    // both cases. The cast is safe because the server is the source of
    // truth for method results and is generated from the same Rust types.
    return (await res.json()) as RpcResultOf<M>;
  }

  subscribe(filter: EventFilter, since?: Since): AsyncIterable<EventEnvelope> {
    const url = this.buildSubscribeUrl(filter, since);
    return sseToAsyncIterable(url);
  }

  private buildSubscribeUrl(filter: EventFilter, since?: Since): string {
    const u = new URL(`${this.cfg.baseUrl}/events`);
    u.searchParams.set("scope", filter.scope);
    if (filter.scope === "job") u.searchParams.set("job_id", filter.job_id);
    if (since != null) u.searchParams.set("since", String(since));
    if (this.cfg.token) u.searchParams.set("token", this.cfg.token);
    return u.toString();
  }
}

function sseToAsyncIterable(url: string): AsyncIterable<EventEnvelope> {
  return {
    [Symbol.asyncIterator](): AsyncIterator<EventEnvelope> {
      const source = new EventSource(url);
      const queue: EventEnvelope[] = [];
      const waiters: Array<(v: IteratorResult<EventEnvelope>) => void> = [];
      let done = false;
      let error: unknown = null;

      source.onmessage = (e) => {
        try {
          const env = JSON.parse(e.data) as EventEnvelope;
          const w = waiters.shift();
          if (w) w({ value: env, done: false });
          else queue.push(env);
        } catch (err) {
          error = err;
          source.close();
          drainWithError();
        }
      };
      source.onerror = (e) => {
        // EventSource auto-reconnects with `Last-Event-ID` on its own.
        // We only surface the error if the connection is permanently
        // closed (readyState === CLOSED), which happens after exhausting
        // its retries or on an explicit `source.close()`.
        if (source.readyState === EventSource.CLOSED) {
          error = e;
          drainWithError();
        }
      };

      function drainWithError() {
        done = true;
        while (waiters.length) {
          const w = waiters.shift()!;
          w({ value: undefined, done: true });
        }
      }

      return {
        async next(): Promise<IteratorResult<EventEnvelope>> {
          if (queue.length) return { value: queue.shift()!, done: false };
          if (done) return { value: undefined, done: true };
          if (error) throw error;
          return new Promise((resolve) => waiters.push(resolve));
        },
        async return(): Promise<IteratorResult<EventEnvelope>> {
          source.close();
          done = true;
          return { value: undefined, done: true };
        },
      };
    },
  };
}
