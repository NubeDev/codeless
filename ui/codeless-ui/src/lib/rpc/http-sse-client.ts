// Browser + mobile transport. REST POST per call, SSE for the event
// stream. The exact URL shapes here will be matched by `codeless-server`
// (Phase 3) — when that lands, this file is the contract the server
// must conform to.
//
// Auth: bearer token sent as `Authorization: Bearer <token>` on REST
// calls. SSE has no header API in browsers, so the token is passed as
// `?token=` query param; revisit when the hosted auth story is firm
// (cookie-based session is the likely Phase 7 answer).

import type {
  RpcClient,
  SseConnectionState,
  SseConnectionStatus,
} from "./client";
import { RpcError } from "./error";
import type {
  EventFilter,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "./methods";
import type { EventEnvelope, ServerInfo } from "./wire";

// EventSource is silent on stale streams: a TCP connection that is
// idle but not closed (NAT timeout, proxy half-close, suspended
// laptop) keeps `readyState === OPEN` while delivering nothing. The
// server-side SSE handler sends a `: heartbeat` comment every 20 s
// (`KeepAlive`), so if 30 s pass with neither an event nor a
// heartbeat we treat the stream as stale, tear it down explicitly,
// and let the auto-reconnect logic recreate it. 30 s is comfortably
// past the 20 s heartbeat with one missed-frame budget.
const STALE_AFTER_MS = 30_000;

// Time the next reconnect attempt — capped exponential, jittered.
// EventSource's built-in reconnect uses the server's `retry:` field
// or a UA default (3 s in most browsers); when *we* tear it down
// after a stale detection we apply our own backoff so a wedged
// proxy doesn't get hammered.
function reconnectDelay(attempt: number): number {
  const base = Math.min(1000 * 2 ** attempt, 30_000);
  return base + Math.random() * base * 0.25;
}

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

  async serverInfo(): Promise<ServerInfo> {
    const res = await fetch(`${this.cfg.baseUrl}/server/info`);
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw RpcError.fromHttpStatus(res.status, text || res.statusText);
    }
    return (await res.json()) as ServerInfo;
  }

  subscribe(filter: EventFilter, since?: Since): AsyncIterable<EventEnvelope> {
    return sseToAsyncIterable((onEvent, onState) =>
      this.subscribeWithState(filter, since, onEvent, onState),
    );
  }

  subscribeWithState(
    filter: EventFilter,
    since: Since | undefined,
    onEvent: (env: EventEnvelope) => void,
    onState: (s: SseConnectionStatus) => void,
  ): () => void {
    return openManagedSse(
      (cursor) => this.buildSubscribeUrl(filter, cursor ?? since),
      onEvent,
      onState,
    );
  }

  private buildSubscribeUrl(filter: EventFilter, since?: Since): string {
    const u = new URL(`${this.cfg.baseUrl}/events`);
    u.searchParams.set("scope", filter.scope);
    if (filter.scope === "job") u.searchParams.set("job_id", filter.job_id);
    if (filter.scope === "repo") u.searchParams.set("repo_id", filter.repo_id);
    if (since != null) u.searchParams.set("since", String(since));
    if (this.cfg.token) u.searchParams.set("token", this.cfg.token);
    return u.toString();
  }
}

// Open a managed SSE connection. `urlFor(cursor)` returns the URL to
// connect to; on a fresh open `cursor` is `null` and the caller's
// own `since` is the floor. On a reconnect after a stale detection
// the cursor is the last successfully delivered event id, so the
// server resumes from where we left off without flooding us with
// already-seen events. The returned function cancels the
// subscription (closes the EventSource, clears timers, transitions
// to `disconnected`).
function openManagedSse(
  urlFor: (cursor: number | null) => string,
  onEvent: (env: EventEnvelope) => void,
  onState: (s: SseConnectionStatus) => void,
): () => void {
  let source: EventSource | null = null;
  let lastCursor: number | null = null;
  let stateEnteredAt = Date.now();
  let currentState: SseConnectionState = "connecting";
  let staleTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let attempt = 0;
  let cancelled = false;

  function setState(next: SseConnectionState) {
    if (currentState === next) return;
    currentState = next;
    stateEnteredAt = Date.now();
    publishState();
  }

  function publishState() {
    onState({
      state: currentState,
      since_ms: Date.now() - stateEnteredAt,
      last_cursor: lastCursor,
    });
  }

  function armStaleTimer() {
    if (staleTimer) clearTimeout(staleTimer);
    staleTimer = setTimeout(() => {
      // The server is *supposed* to be sending heartbeats every 20 s.
      // If nothing — neither event nor heartbeat — arrived in
      // STALE_AFTER_MS, the stream is wedged even though
      // readyState may still claim OPEN. Force a reconnect.
      if (cancelled) return;
      teardownSource();
      setState("reconnecting");
      scheduleReconnect();
    }, STALE_AFTER_MS);
  }

  function teardownSource() {
    if (staleTimer) {
      clearTimeout(staleTimer);
      staleTimer = null;
    }
    if (source) {
      source.close();
      source = null;
    }
  }

  function scheduleReconnect() {
    if (cancelled) return;
    if (reconnectTimer) clearTimeout(reconnectTimer);
    const delay = reconnectDelay(attempt);
    attempt += 1;
    reconnectTimer = setTimeout(() => {
      if (cancelled) return;
      open();
    }, delay);
  }

  function open() {
    teardownSource();
    setState(attempt === 0 ? "connecting" : "reconnecting");
    const url = urlFor(lastCursor);
    const s = new EventSource(url);
    source = s;
    armStaleTimer();

    s.onopen = () => {
      attempt = 0;
      setState("live");
      armStaleTimer();
    };

    s.onmessage = (e) => {
      // Any frame (event *or* heartbeat — though heartbeat comments
      // don't fire onmessage) resets the stale clock.
      armStaleTimer();
      if (currentState !== "live") setState("live");

      try {
        const env = JSON.parse(e.data) as EventEnvelope;
        // The server sets `id: <cursor>` on every event; EventSource
        // exposes it as `e.lastEventId`. Track it so our own forced
        // reconnect can resume from the right cursor — and so the
        // consumer can render "reconnecting at cursor 437".
        const idNum = e.lastEventId ? Number(e.lastEventId) : NaN;
        if (Number.isFinite(idNum)) lastCursor = idNum;
        onEvent(env);
      } catch {
        // A malformed frame should not nuke the whole stream;
        // dropping it and waiting for the next is the right
        // recovery. The stale timer will pick it up if the server
        // is genuinely broken.
      }
    };

    s.onerror = () => {
      if (cancelled) return;
      // EventSource sets readyState to CONNECTING on transient
      // errors (it's about to retry) and CLOSED on terminal ones
      // (rare — usually 401 / CORS / DNS). Either way the user
      // experience is "events stopped"; flip to reconnecting and
      // let our own scheduler drive recovery so the badge shows
      // the right thing during long outages.
      if (s.readyState === EventSource.CLOSED) {
        teardownSource();
        setState("reconnecting");
        scheduleReconnect();
      } else {
        // CONNECTING — EventSource is auto-retrying. Surface that
        // state so the badge reflects reality, but let EventSource
        // do the work.
        setState("reconnecting");
      }
    };
  }

  open();

  return () => {
    cancelled = true;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    teardownSource();
    setState("disconnected");
  };
}

// Bridge the managed-state subscription back to the AsyncIterable
// shape the existing CLI / iterable callers expect. State
// transitions are dropped here; iterable consumers that need
// liveness should switch to `subscribeWithState` directly.
function sseToAsyncIterable(
  open: (
    onEvent: (env: EventEnvelope) => void,
    onState: (s: SseConnectionStatus) => void,
  ) => () => void,
): AsyncIterable<EventEnvelope> {
  return {
    [Symbol.asyncIterator](): AsyncIterator<EventEnvelope> {
      const queue: EventEnvelope[] = [];
      const waiters: Array<(v: IteratorResult<EventEnvelope>) => void> = [];
      let done = false;

      const cancel = open(
        (env) => {
          const w = waiters.shift();
          if (w) w({ value: env, done: false });
          else queue.push(env);
        },
        (status) => {
          if (status.state === "disconnected") {
            done = true;
            while (waiters.length) {
              const w = waiters.shift()!;
              w({ value: undefined, done: true });
            }
          }
        },
      );

      return {
        async next(): Promise<IteratorResult<EventEnvelope>> {
          if (queue.length) return { value: queue.shift()!, done: false };
          if (done) return { value: undefined, done: true };
          return new Promise((resolve) => waiters.push(resolve));
        },
        async return(): Promise<IteratorResult<EventEnvelope>> {
          cancel();
          done = true;
          return { value: undefined, done: true };
        },
      };
    },
  };
}
