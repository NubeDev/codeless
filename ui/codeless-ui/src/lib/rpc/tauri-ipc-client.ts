// Desktop transport. `invoke()` for request/reply, Tauri 2 `Channel<T>`
// for the event stream — see SCOPE.md "Streaming subscriptions" for why
// channels and not the bus-style `listen()` API: channels give the
// desktop shell a typed, per-subscription stream from Rust to JS with
// no extra global routing.
//
// Wire contract the `codeless-tauri-desktop` Rust crate must implement:
//
//   #[tauri::command] async fn rpc_<method>(args: <ArgsStruct>)
//     -> Result<<Result>, RpcError>
//
//   #[tauri::command] async fn rpc_subscribe(
//     args: SubscribeArgs,
//     channel: tauri::ipc::Channel<EventEnvelope>,
//   ) -> Result<(), RpcError>
//
//   #[tauri::command] async fn rpc_unsubscribe(channel_id: u32)
//     -> Result<(), RpcError>
//
// The args struct is nested under `{ args }` so the Rust command takes
// a single typed parameter; aligns with the `codeless-rpc::RpcServer`
// trait shape and keeps `tauri-specta` output one-to-one with the
// trait methods.

import { Channel, invoke } from "@tauri-apps/api/core";

import type { RpcClient } from "./client";
import type {
  EventFilter,
  RpcArgs,
  RpcMethod,
  RpcResultOf,
  Since,
} from "./methods";
import type { EventEnvelope, ServerInfo } from "./wire";

interface SubscribeArgs {
  filter: EventFilter;
  since: Since;
}

export class TauriIpcClient implements RpcClient {
  call<M extends RpcMethod>(
    method: M,
    args: RpcArgs<M>,
  ): Promise<RpcResultOf<M>> {
    return invoke<RpcResultOf<M>>(`rpc_${method}`, { args });
  }

  serverInfo(): Promise<ServerInfo> {
    return invoke<ServerInfo>("rpc_server_info");
  }

  subscribe(filter: EventFilter, since?: Since): AsyncIterable<EventEnvelope> {
    const args: SubscribeArgs = { filter, since: since ?? null };
    return channelToAsyncIterable(args);
  }
}

function channelToAsyncIterable(
  args: SubscribeArgs,
): AsyncIterable<EventEnvelope> {
  return {
    [Symbol.asyncIterator](): AsyncIterator<EventEnvelope> {
      const queue: EventEnvelope[] = [];
      const waiters: Array<(v: IteratorResult<EventEnvelope>) => void> = [];
      let done = false;
      let error: unknown = null;

      const channel = new Channel<EventEnvelope>();
      channel.onmessage = (env) => {
        if (done) return;
        const w = waiters.shift();
        if (w) w({ value: env, done: false });
        else queue.push(env);
      };

      const subscribeCall = invoke<void>("rpc_subscribe", { args, channel })
        .catch((e: unknown) => {
          error = e;
          drain();
        });

      function drain() {
        done = true;
        while (waiters.length) {
          const w = waiters.shift()!;
          w({ value: undefined, done: true });
        }
      }

      async function cancel(): Promise<void> {
        if (done) return;
        done = true;
        // Best-effort: ignore unsubscribe errors. The channel id may
        // already be torn down if the Rust side ended the stream.
        await subscribeCall;
        try {
          await invoke<void>("rpc_unsubscribe", { channel_id: channel.id });
        } catch {
          // swallow
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
          await cancel();
          return { value: undefined, done: true };
        },
      };
    },
  };
}
