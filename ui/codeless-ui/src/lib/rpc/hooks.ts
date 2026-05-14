// Typed query/subscription hooks. Thin wrappers over `useRpc()` that
// centralise the load/error state pattern so call sites stay short.
// Deliberately not a full query library — when the UI grows enough to
// need cache invalidation across components, swap in TanStack Query
// behind these signatures and call sites won't notice.

import { useCallback, useEffect, useRef, useState } from "react";

import { useRpc } from "./provider";
import type { RpcClient } from "./client";
import type { EventFilter, ListJobsArgs, ListReviewsArgs } from "./methods";
import type { EventEnvelope, Job, JobId, Repo, Review } from "./wire";

export interface QueryState<T> {
  data: T | null;
  error: Error | null;
  loading: boolean;
}

function useAsyncOnce<T>(run: (signal: AbortSignal) => Promise<T>): QueryState<T> {
  const [state, setState] = useState<QueryState<T>>({
    data: null,
    error: null,
    loading: true,
  });

  // Re-run only when the caller passes a new function reference; callers
  // are expected to wrap in useCallback if their args change.
  const runRef = useRef(run);
  runRef.current = run;

  useEffect(() => {
    const ac = new AbortController();
    setState((s) => ({ ...s, loading: true }));
    runRef
      .current(ac.signal)
      .then((data) => {
        if (ac.signal.aborted) return;
        setState({ data, error: null, loading: false });
      })
      .catch((err: unknown) => {
        if (ac.signal.aborted) return;
        setState({
          data: null,
          error: err instanceof Error ? err : new Error(String(err)),
          loading: false,
        });
      });
    return () => ac.abort();
  }, []);

  return state;
}

export function useRepos(): QueryState<Repo[]> {
  const rpc = useRpc();
  const q = useAsyncOnce(() => rpc.call("list_repos", {}));
  return { ...q, data: q.data?.repos ?? null };
}

export function useJobs(args: ListJobsArgs = { repo_id: null }): QueryState<Job[]> {
  const rpc = useRpc();
  // Stable JSON key so the effect re-runs when the filter changes by
  // value, not by identity. Cheap because args is shallow.
  const key = JSON.stringify(args);
  const [state, setState] = useState<QueryState<Job[]>>({
    data: null,
    error: null,
    loading: true,
  });
  useEffect(() => {
    let cancelled = false;
    setState((s) => ({ ...s, loading: true }));
    rpc
      .call("list_jobs", args)
      .then((r) => {
        if (cancelled) return;
        setState({ data: r.jobs, error: null, loading: false });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setState({
          data: null,
          error: err instanceof Error ? err : new Error(String(err)),
          loading: false,
        });
      });
    return () => {
      cancelled = true;
    };
    // key encodes args; rpc identity is stable across renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, rpc]);
  return state;
}

export interface JobQueryState extends QueryState<Job> {
  /**
   * Force a fresh `get_job` call. Use after any RPC that mutates the
   * job server-side (start_job, update_job_template, write_job_file,
   * write_handover, …) so consumers re-render with current state.
   *
   * Without this the hook would stay pinned to the row it loaded on
   * mount: clicking "run" would flip the server's status but the UI's
   * `job.status` would stay `draft`, the header's `[run]` button
   * would stay clickable, and a second click would 409.
   */
  refetch: () => void;
}

export function useJob(jobId: JobId | null): JobQueryState {
  const rpc = useRpc();
  const [state, setState] = useState<QueryState<Job>>({
    data: null,
    error: null,
    loading: jobId != null,
  });
  // `tick` is the refetch trigger. Bumping it re-runs the effect
  // without changing `jobId` or `rpc`. Stable reference for the
  // returned `refetch` so callers can pass it to `useEffect`/
  // `useCallback` deps without triggering recreate loops.
  const [tick, setTick] = useState(0);
  const refetch = useCallback(() => setTick((t) => t + 1), []);
  useEffect(() => {
    if (jobId == null) {
      setState({ data: null, error: null, loading: false });
      return;
    }
    let cancelled = false;
    setState((s) => ({ ...s, loading: true }));
    rpc
      .call("get_job", { job_id: jobId })
      .then((job) => {
        if (cancelled) return;
        setState({ data: job, error: null, loading: false });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setState({
          data: null,
          error: err instanceof Error ? err : new Error(String(err)),
          loading: false,
        });
      });
    return () => {
      cancelled = true;
    };
    // tick bumps force a re-fetch; eslint can't see through it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId, rpc, tick]);
  return { ...state, refetch };
}

// Live reviews for a scope. Refetches `list_reviews` on any `review-*`
// event so an approve from another client (or the Phase 2c CLI)
// reflects without polling. Stage gating in `codeless-runtime` emits
// `review-requested` when a stage enters AwaitingReview, so a cold
// mount can rely on the initial fetch alone — the stream is for
// keeping live.
export function useReviews(args: ListReviewsArgs): QueryState<Review[]> {
  const rpc = useRpc();
  const key = JSON.stringify(args);
  const [state, setState] = useState<QueryState<Review[]>>({
    data: null,
    error: null,
    loading: true,
  });
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setState((s) => ({ ...s, loading: true }));
    rpc
      .call("list_reviews", args)
      .then((r) => {
        if (cancelled) return;
        setState({ data: r.reviews, error: null, loading: false });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setState({
          data: null,
          error: err instanceof Error ? err : new Error(String(err)),
          loading: false,
        });
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, rpc, tick]);

  useEffect(() => {
    const filter: EventFilter =
      args.job_id != null ? { scope: "job", job_id: args.job_id } : { scope: "all" };
    const subKey = JSON.stringify({ filter, since: 0 });
    const leave = joinSubscription(rpc, subKey, filter, 0, (env) => {
      const t = env.event.type;
      if (
        t === "review-requested" ||
        t === "review-approved" ||
        t === "review-commented" ||
        t === "review-stopped"
      ) {
        setTick((n) => n + 1);
      }
    });
    return leave;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [args.job_id, rpc]);

  return state;
}

// Per-(rpc, filter, since) shared subscription. The browser's
// HTTP/1.1 per-origin connection cap is 6; one EventSource per
// useEventStream call site burns through that quickly when multiple
// components on the same page subscribe to the same job feed (the
// JobPage, the JobTimeline, the RunPane's live stage cards, the
// dashboard's "all" stream). Sharing the underlying iterator means
// N components subscribed to the same filter use 1 EventSource. The
// fs explorer was the canary: with the cap consumed, fs_read_dir
// POSTs queued indefinitely and the tree sat on "Loading…".
interface SharedSubscription {
  listeners: Set<(env: EventEnvelope) => void>;
  /** Replay buffer: every event received so far. Late-joining
   *  listeners (e.g. the right-panel Timeline mounting after the
   *  center RunPane has already consumed the replay) get the full
   *  history before going live. Without this, completed-job pages
   *  show "waiting for events…" in any panel that mounts after the
   *  initial EventSource replay finishes. */
  buffer: EventEnvelope[];
  cancel: () => void;
}
const SHARED_SUBSCRIPTIONS = new WeakMap<
  RpcClient,
  Map<string, SharedSubscription>
>();

function joinSubscription(
  rpc: RpcClient,
  key: string,
  filter: EventFilter,
  since: number,
  listener: (env: EventEnvelope) => void,
): () => void {
  let perRpc = SHARED_SUBSCRIPTIONS.get(rpc);
  if (!perRpc) {
    perRpc = new Map();
    SHARED_SUBSCRIPTIONS.set(rpc, perRpc);
  }
  let shared = perRpc.get(key);
  if (!shared) {
    const listeners = new Set<(env: EventEnvelope) => void>();
    const buffer: EventEnvelope[] = [];
    const stream = rpc.subscribe(filter, since);
    const iter = stream[Symbol.asyncIterator]();
    let cancelled = false;
    shared = {
      listeners,
      buffer,
      cancel: () => {
        cancelled = true;
        iter.return?.();
      },
    };
    perRpc.set(key, shared);
    (async () => {
      try {
        while (true) {
          const r = await iter.next();
          if (r.done || cancelled) return;
          buffer.push(r.value);
          for (const cb of listeners) {
            try {
              cb(r.value);
            } catch {
              // Listener errors must not break the shared pump.
            }
          }
        }
      } catch {
        // Stream errors end iteration; consumers that re-mount will
        // recreate the shared subscription on next listener join.
      } finally {
        if (perRpc?.get(key) === shared) {
          perRpc.delete(key);
        }
      }
    })();
  }
  // Replay buffered events so late joiners see the full history.
  for (const env of shared.buffer) {
    try {
      listener(env);
    } catch {
      // Same policy as the live pump: swallow listener errors.
    }
  }
  shared.listeners.add(listener);
  return () => {
    if (!shared) return;
    shared.listeners.delete(listener);
    if (shared.listeners.size === 0) {
      shared.cancel();
      perRpc?.delete(key);
    }
  };
}

export function useEventStream(
  filter: EventFilter,
  onEvent: (env: EventEnvelope) => void,
  since: number = 0,
): void {
  const rpc = useRpc();
  // Stable callback ref so the subscription doesn't tear down on every
  // render of the consumer.
  const cbRef = useRef(onEvent);
  cbRef.current = onEvent;

  const key = JSON.stringify({ filter, since });
  useEffect(() => {
    // `since: 0` replays every persisted event for the filter before
    // going live. Completed jobs have no live events left, so without
    // replay the JobTimeline pane sits forever on "waiting for
    // events…". Callers that genuinely want live-only state pass a
    // non-zero cursor (a recent cursor, or one captured from a
    // previous batch).
    const leave = joinSubscription(rpc, key, filter, since, (env) => {
      cbRef.current(env);
    });
    return leave;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, rpc]);
}
