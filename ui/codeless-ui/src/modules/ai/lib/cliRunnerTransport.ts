// Transport that routes a single chat turn through the host's
// `agent_chat` RPC instead of running an agent loop in the browser.
//
// The footer panel sends "claude / codex / copilot CLI" turns through
// here. The host spawns the chosen CLI runner, streams upstream events
// over the regular event bus tagged with a per-turn session id, and we
// translate those envelopes into the `UIMessageChunk` shape `@ai-sdk/react`
// already renders. Browser-direct provider runs (OpenAI/Anthropic keys
// from the keychain) continue to use `DirectChatTransport`; that branch
// owns its own agent loop and tools.

import type { UIMessage, UIMessageChunk } from "ai";
import type { RpcClient } from "@/lib/rpc/client";
import type { Event, EventEnvelope, JobId } from "@/lib/rpc/wire";

/** Wire ids the backend's `agent_chat` accepts for `args.runner`. */
export type CliRunnerId = "claude" | "codex" | "copilot";

/** Strip the `cli:` prefix from a model id, returning the runner id
 *  the backend expects (or `null` if the model id is not a CLI entry).
 *  Centralising the prefix mapping keeps both the dropdown filter and
 *  the transport branching in `createContextAwareTransport` agreeing
 *  on the same string set. */
export function cliRunnerIdFromModelId(modelId: string): CliRunnerId | null {
  if (!modelId.startsWith("cli:")) return null;
  const tail = modelId.slice(4);
  if (tail === "claude" || tail === "codex" || tail === "copilot") return tail;
  return null;
}

function lastUserText(messages: readonly UIMessage[]): string {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role !== "user") continue;
    const parts = m.parts ?? [];
    const text = parts
      .map((p) => (p.type === "text" ? p.text : ""))
      .join("")
      .trim();
    if (text.length > 0) return text;
  }
  return "";
}

// Per-turn correlation id. The backend persists events with this id as
// the envelope `job_id`, so a `subscribe(EventFilter::Job)` matches
// every emitted event for this turn — even without a backing jobs row.
function mintSessionId(): JobId {
  // ULID-flavoured 26-char base32. We don't need cryptographic strength;
  // collision resistance + monotonic order is enough for a chat turn id.
  const alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  const bytes = new Uint8Array(16);
  (globalThis.crypto ?? window.crypto).getRandomValues(bytes);
  let out = "";
  for (let i = 0; i < 26; i++) {
    out += alphabet[bytes[i % bytes.length] & 0x1f];
  }
  return out as JobId;
}

/** Build the cli-runner-backed transport. `rpc.subscribe` must be live
 *  before `sendMessages` resolves the underlying `agent_chat` call so
 *  no early tokens are dropped — we open the subscription first, then
 *  fire the RPC. */
export function createCliRunnerTransport(rpc: RpcClient, runner: CliRunnerId) {
  return {
    async sendMessages(options: {
      messages: UIMessage[];
      [k: string]: unknown;
    }): Promise<ReadableStream<UIMessageChunk>> {
      const prompt = lastUserText(options.messages);
      const sessionId = mintSessionId();

      // Subscribe before the RPC fires so the live broadcast tail is
      // already attached when the runtime starts publishing — see
      // `EventBus::subscribe_since` for the gap-free contract.
      const stream = rpc.subscribe({ scope: "job", job_id: sessionId });

      // Detached: fire the RPC; the result is just an ack with the
      // task id under which events are tagged. Failures here surface
      // as the stream emitting an `error` chunk and closing.
      const rpcPromise = rpc
        .call("agent_chat", { runner, prompt, session_id: sessionId })
        .catch((e) => {
          throw e instanceof Error ? e : new Error(String(e));
        });

      return new ReadableStream<UIMessageChunk>({
        async start(controller) {
          // `text-start` opens an assistant text block; one block per
          // turn is enough — Claude/Codex output is treated as a
          // single assistant response. Tool calls get their own
          // `tool-input-available` chunk and do not break the text
          // stream (the renderer interleaves them by order received).
          const messageId = `cli-${sessionId}`;
          controller.enqueue({ type: "start", messageId });
          controller.enqueue({ type: "start-step" });
          controller.enqueue({ type: "text-start", id: messageId });

          let textOpen = true;
          const closeText = () => {
            if (textOpen) {
              controller.enqueue({ type: "text-end", id: messageId });
              textOpen = false;
            }
          };

          try {
            // Surface RPC-level errors (unknown runner, registry not
            // configured) as a single `error` chunk. The await runs in
            // parallel with the subscribe pump below — keep it on the
            // micro-task queue rather than blocking the loop.
            rpcPromise.catch((err: Error) => {
              controller.enqueue({ type: "error", errorText: err.message });
              closeText();
              controller.enqueue({ type: "finish-step" });
              controller.enqueue({ type: "finish" });
              controller.close();
            });

            for await (const env of stream as AsyncIterable<EventEnvelope>) {
              const chunk = mapEnvelope(env, messageId);
              if (!chunk) continue;
              for (const c of chunk) {
                if (c.type === "text-end") closeText();
                else controller.enqueue(c);
              }
              if (env.event.type === "ai-message-complete") {
                closeText();
                controller.enqueue({ type: "finish-step" });
                controller.enqueue({ type: "finish" });
                controller.close();
                return;
              }
            }
            closeText();
            controller.enqueue({ type: "finish-step" });
            controller.enqueue({ type: "finish" });
            controller.close();
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            controller.enqueue({ type: "error", errorText: msg });
            closeText();
            controller.error(e);
          }
        },
      });
    },

    async reconnectToStream(): Promise<null> {
      // Resuming a CLI run mid-flight is intentionally not supported
      // in v1. The browser tab driving the chat owns the subscription;
      // reload = drop the turn. A future revision can store the
      // session id in chat-store metadata and resume the subscription.
      return null;
    },
  };
}

function mapEnvelope(
  env: EventEnvelope,
  messageId: string,
): UIMessageChunk[] | null {
  const ev = env.event;
  return mapEvent(ev, messageId);
}

function mapEvent(ev: Event, messageId: string): UIMessageChunk[] | null {
  switch (ev.type) {
    case "ai-token":
      return [{ type: "text-delta", id: messageId, delta: ev.delta }];
    case "tool-call": {
      // Synthesize a stable id from the tool name + JSON args so the
      // renderer keys chips deterministically. Two identical calls in
      // one turn collide — acceptable for v1; the renderer just
      // collapses them. A future revision can route a tool-call id
      // through the ai_runner_bridge.
      const id = `${ev.tool}:${hash(ev.args_json)}`;
      let input: unknown = ev.args_json;
      try {
        input = JSON.parse(ev.args_json);
      } catch {
        // leave as raw string
      }
      return [
        {
          type: "tool-input-available",
          toolCallId: id,
          toolName: ev.tool,
          input,
          dynamic: true,
        },
      ];
    }
    case "ai-message-complete":
      // Caller closes the text block + emits `finish` to keep the
      // single-source-of-truth invariant on stream termination.
      return [{ type: "text-end", id: messageId }];
    default:
      return null;
  }
}

// Tiny non-cryptographic hash for stable tool-call chip ids.
function hash(s: string): string {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}
