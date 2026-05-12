// Typed query/subscription hooks. Thin wrappers over `useRpc()` that
// centralise the load/error state pattern so call sites stay short.
// Deliberately not a full query library — when the UI grows enough to
// need cache invalidation across components, swap in TanStack Query
// behind these signatures and call sites won't notice.

import { useEffect, useRef, useState } from "react";

import { useRpc } from "./provider";
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

export function useJob(jobId: JobId | null): QueryState<Job> {
  const rpc = useRpc();
  const [state, setState] = useState<QueryState<Job>>({
    data: null,
    error: null,
    loading: jobId != null,
  });
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
  }, [jobId, rpc]);
  return state;
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
    let cancelled = false;
    const filter: EventFilter =
      args.job_id != null ? { scope: "job", job_id: args.job_id } : { scope: "all" };
    const stream = rpc.subscribe(filter);
    const iter = stream[Symbol.asyncIterator]();
    (async () => {
      try {
        while (true) {
          const r = await iter.next();
          if (r.done || cancelled) return;
          const t = r.value.event.type;
          if (
            t === "review-requested" ||
            t === "review-approved" ||
            t === "review-commented" ||
            t === "review-stopped"
          ) {
            setTick((n) => n + 1);
          }
        }
      } catch {
        // Stream errors end the iteration; consumer can re-mount.
      }
    })();
    return () => {
      cancelled = true;
      // Explicit close so the underlying EventSource is released
      // synchronously on unmount. Without this, the browser's per-origin
      // SSE connection cap (6 in Chrome) is reached after a few job
      // navigations and new subscriptions stall.
      iter.return?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [args.job_id, rpc]);

  return state;
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
    let cancelled = false;
    // `since: 0` replays every persisted event for the filter before
    // going live. Completed jobs have no live events left, so without
    // replay the JobTimeline pane sits forever on "waiting for
    // events…". Callers that genuinely want live-only state pass a
    // non-zero cursor (a recent cursor, or one captured from a
    // previous batch).
    const stream = rpc.subscribe(filter, since);
    const iter = stream[Symbol.asyncIterator]();
    (async () => {
      try {
        while (true) {
          const r = await iter.next();
          if (r.done || cancelled) return;
          cbRef.current(r.value);
        }
      } catch {
        // Stream errors end the iteration; consumer can re-mount to retry.
      }
    })();
    return () => {
      cancelled = true;
      // Explicit close so the underlying EventSource is released
      // synchronously on unmount. Without this, the browser's per-origin
      // SSE connection cap (6 in Chrome) is reached after a few job
      // navigations and new subscriptions stall.
      iter.return?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, rpc]);
}
