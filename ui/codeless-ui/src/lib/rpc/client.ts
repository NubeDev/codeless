// The single transport boundary every UI module imports. Sketched in
// SCOPE.md "The `RpcClient` interface (TS)". Two implementations:
// `HttpSseClient` for browser + mobile shells, `TauriIpcClient` (later)
// for the desktop shell. Components must depend only on this interface
// — never on `fetch`, `EventSource`, or `@tauri-apps/api/*` directly.
// That separation is what makes "one UI, four shells" survive.

import type { EventEnvelope } from "./wire";
import type {
  EventFilter,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "./methods";

export interface RpcClient {
  call<M extends RpcMethod>(method: M, args: RpcArgs<M>): Promise<RpcResultOf<M>>;

  // Async iterable so call sites use `for await`. Implementations are
  // responsible for resume-on-disconnect using the latest cursor seen.
  subscribe(filter: EventFilter, since?: Since): AsyncIterable<EventEnvelope>;
}
