// The single transport boundary every UI module imports. Sketched in
// SCOPE.md "The `RpcClient` interface (TS)". Two implementations:
// `HttpSseClient` for browser + mobile shells, `TauriIpcClient` (later)
// for the desktop shell. Components must depend only on this interface
// — never on `fetch`, `EventSource`, or `@tauri-apps/api/*` directly.
// That separation is what makes "one UI, four shells" survive.

import type { EventEnvelope, ServerInfo } from "./wire";
import type {
  EventFilter,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "./methods";

// Liveness of a subscription as observed by the client. `live` means
// the transport is delivering events (or healthy heartbeats);
// `reconnecting` is a transient outage the transport is recovering
// from; `disconnected` is a hard failure the consumer should surface
// to the user. The Tauri/mock implementations have no real network
// failure mode and stay `live` for the lifetime of the subscription.
export type SseConnectionState =
  | "connecting"
  | "live"
  | "reconnecting"
  | "disconnected";

// One state observation. `since_ms` is the wall-clock duration the
// connection has been in this state, useful for "reconnecting for
// 7s" badge copy. `last_cursor` is the most recent cursor the
// transport believes it has delivered, surfaced so the consumer can
// indicate the reconnect resume point.
export interface SseConnectionStatus {
  state: SseConnectionState;
  since_ms: number;
  last_cursor: number | null;
}

export interface RpcClient {
  call<M extends RpcMethod>(method: M, args: RpcArgs<M>): Promise<RpcResultOf<M>>;

  // Async iterable so call sites use `for await`. Implementations are
  // responsible for resume-on-disconnect using the latest cursor seen.
  subscribe(filter: EventFilter, since?: Since): AsyncIterable<EventEnvelope>;

  // Optional liveness-observable variant. When the transport supports
  // explicit connection-state signalling (only `HttpSseClient` today),
  // the consumer can read both the event stream and the state stream
  // in lockstep. Implementations that do not implement it (mock,
  // Tauri) can omit the field — consumers degrade to the iterable
  // form which treats the connection as always-live.
  subscribeWithState?(
    filter: EventFilter,
    since: Since | undefined,
    onEvent: (env: EventEnvelope) => void,
    onState: (s: SseConnectionStatus) => void,
  ): () => void;

  // Unauthenticated bootstrap snapshot: runner list, fs root, worktree
  // root, claude probe. Lives outside the `/rpc/*` gate on the wire
  // (the UI needs it before it can supply a token) but routes through
  // the same transport interface so the Tauri and mock shells don't
  // have to fall back to direct fetch.
  serverInfo(): Promise<ServerInfo>;
}
