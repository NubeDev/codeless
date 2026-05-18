/**
 * `<PluginSlot id="..." />` — the single mount point for plugin UI
 * inside the host shell.
 *
 * The host renders `<PluginSlot id="assistant-panel" />`,
 * `<PluginSlot id="tool-result:notes.append" />`, etc. at the sites
 * named in DOCS/plugins/PLUGIN-UI-FEDERATION.md § Slot vocabulary.
 * PluginSlot resolves contributors out of the registration table,
 * asks the installed `MfRuntime` to load each contributor's
 * exposed module, and renders it inside an error boundary. Any
 * extra props are forwarded to the plugin component verbatim.
 *
 * The error boundary's job is to make a misbehaving plugin a local
 * problem: a thrown render, a failed import, or an MF version
 * mismatch shows a small structured-error card in the slot instead
 * of taking the host page down.
 */

import {
  Component,
  Suspense,
  createElement,
  lazy,
  useEffect,
  useMemo,
  useState,
  type ComponentType,
  type LazyExoticComponent,
  type ReactElement,
  type ReactNode,
} from "react";

/**
 * Wrap `createElement(Comp, { ...rest, slotArg })` in a helper so the
 * spread doesn't fight TypeScript's `IntrinsicAttributes` check; the
 * lazy component is intentionally typed as `ComponentType<unknown>`
 * so the SDK is not coupled to per-slot prop shapes.
 */
function createSlotElement(
  Comp: ComponentType<unknown>,
  rest: Record<string, unknown>,
  slotArg: string | null,
): ReactElement {
  return createElement(Comp as ComponentType<Record<string, unknown>>, {
    ...rest,
    slotArg,
  });
}

import { getMfRuntime, parseSlotId } from "../mf";
import {
  getSlotContributors,
  subscribeToRegistry,
  type SlotContributor,
} from "../registration";

export interface PluginSlotProps {
  /** Full slot id, e.g. `"tool-result:notes.append"`. */
  id: string;
  /** Rendered when no plugin contributes to this slot. Optional. */
  fallback?: ReactNode;
  /** Rendered while plugin modules are being lazy-loaded. */
  loading?: ReactNode;
  /**
   * All other props pass through to every plugin component mounted at
   * this slot. The SDK does not type-check these — the slot's
   * contract is documented in PLUGIN-UI-FEDERATION.md alongside the
   * slot id.
   */
  [propName: string]: unknown;
}

interface ErrorCardProps {
  pluginId: string;
  slotId: string;
  reason: string;
}

/**
 * Default rendering of a failed plugin mount. Plain HTML so it works
 * before any plugin styles load and on every shell. The host may swap
 * this by wrapping `<PluginSlot/>` in its own boundary if it wants
 * branded chrome — that's a host concern, not the SDK's.
 */
function DefaultErrorCard({ pluginId, slotId, reason }: ErrorCardProps) {
  return (
    <div
      role="alert"
      data-codeless-plugin-error="true"
      data-plugin-id={pluginId}
      data-slot-id={slotId}
      style={{
        border: "1px solid #c33",
        borderRadius: 6,
        padding: "8px 12px",
        fontSize: 12,
        color: "#c33",
        background: "rgba(204, 51, 51, 0.06)",
      }}
    >
      <strong>plugin failed: </strong>
      <code>{pluginId}</code> at <code>{slotId}</code>
      <div>{reason}</div>
    </div>
  );
}

interface BoundaryProps {
  pluginId: string;
  slotId: string;
  children: ReactNode;
}
interface BoundaryState {
  reason: string | null;
}

/**
 * Per-contributor error boundary. Each plugin module mounts inside
 * its own boundary so a crash in one plugin can never blank the slot
 * for another contributor at the same slot (matters for the
 * `composer-attachment-action:<plugin_id>` slot, which is unbounded).
 */
class PluginErrorBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { reason: null };

  static getDerivedStateFromError(err: unknown): BoundaryState {
    const reason =
      err instanceof Error
        ? `${err.name}: ${err.message}`
        : `non-Error throw: ${String(err)}`;
    return { reason };
  }

  componentDidCatch(err: unknown): void {
    // Console-only for now; structured plugin telemetry lands when
    // codeless-server gains a `plugins.report_failure` RPC.
    // eslint-disable-next-line no-console
    console.error(
      `[plugin-ui-sdk] plugin "${this.props.pluginId}" crashed at slot "${this.props.slotId}"`,
      err,
    );
  }

  render() {
    if (this.state.reason !== null) {
      return (
        <DefaultErrorCard
          pluginId={this.props.pluginId}
          slotId={this.props.slotId}
          reason={this.state.reason}
        />
      );
    }
    return this.props.children;
  }
}

/**
 * Memoise the `lazy(...)` wrapper per contributor so unrelated
 * `<PluginSlot/>` re-renders don't reset the loader and re-fetch the
 * chunk. The cache key is `(remoteName, exposeName)`; identity within
 * a single `MfRuntime` install.
 */
const lazyCache = new Map<string, LazyExoticComponent<ComponentType<unknown>>>();
function getLazyComponent(
  c: SlotContributor,
): LazyExoticComponent<ComponentType<unknown>> {
  const key = `${c.remoteName}//${c.exposeName}`;
  const cached = lazyCache.get(key);
  if (cached) return cached;
  const lazyComp = lazy(async () => {
    const rt = getMfRuntime();
    if (!rt) {
      throw new Error(
        "no MfRuntime installed; the host shell must call setMfRuntime() before any <PluginSlot/> mounts",
      );
    }
    const mod = await rt.loadRemote<unknown>(c.remoteName, c.exposeName);
    return { default: pickDefaultExport(mod) };
  });
  lazyCache.set(key, lazyComp);
  return lazyComp;
}

/**
 * Accept either `{ default: Component }` or `Component` as the
 * loadRemote result. MF runtimes vary on whether the unwrap has
 * happened by the time the promise resolves; both shapes are valid.
 */
function pickDefaultExport(mod: unknown): ComponentType<unknown> {
  if (
    mod &&
    typeof mod === "object" &&
    "default" in mod &&
    typeof (mod as { default: unknown }).default === "function"
  ) {
    return (mod as { default: ComponentType<unknown> }).default;
  }
  if (typeof mod === "function") {
    return mod as ComponentType<unknown>;
  }
  throw new Error(
    "plugin module did not export a React component as its default export",
  );
}

/** Test-only. Drops the lazy-component cache. */
export function resetPluginSlotCacheForTesting(): void {
  lazyCache.clear();
}

export function PluginSlot(props: PluginSlotProps) {
  const { id, fallback = null, loading = null, ...rest } = props;

  const parsed = parseSlotId(id);
  const [, force] = useState(0);
  useEffect(() => subscribeToRegistry(() => force((n) => n + 1)), []);

  const contributors = useMemo(
    () => (parsed ? getSlotContributors(id) : []),
    // Re-read on registry tick (state bump) and on slot id change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [id, parsed],
  );

  if (!parsed) {
    return (
      <DefaultErrorCard
        pluginId="(host)"
        slotId={id}
        reason="unknown slot id — see PLUGIN-UI-FEDERATION.md § Slot vocabulary"
      />
    );
  }

  if (contributors.length === 0) return <>{fallback}</>;

  return (
    <>
      {contributors.map((c) => {
        const Comp = getLazyComponent(c);
        const arg = c.slot.arg;
        return (
          <PluginErrorBoundary
            key={`${c.pluginId}//${c.exposeName}`}
            pluginId={c.pluginId}
            slotId={id}
          >
            <Suspense fallback={loading}>
              {createSlotElement(Comp, rest, arg)}
            </Suspense>
          </PluginErrorBoundary>
        );
      })}
    </>
  );
}
