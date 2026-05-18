// Stage 8 coverage for the chat-side projection of a scoped pause.
// The runtime's `pause_job` path now publishes `JobPaused` with the
// new `StopReason::ScopedPausePoint { point_id }` reason; the chat
// surface must render a dedicated "paused at scoped point" divider
// for it, distinct from a runtime-driven `user` / `cost-cap` pause.
//
// Wire-shape pinned here: serde-JSON renames the enum variant tag to
// `scoped-pause-point` (kebab-case) but leaves the inner struct field
// as `point_id` (the specta TS module spells the inner field with a
// hyphen — that's the known specta/serde divergence the stage-6
// handover documents). The runtime emits the underscore form on the
// bus, so the helper must read it from there.

import { describe, expect, it } from "vitest";

import type { EventEnvelope } from "@/lib/rpc";

import { liveItemFromEvent, scopedPausePointId, stopReasonLabel } from "./feed";

const POINT_ID = "01HPP000000000000000000ABC";

function pausedEnv(reason: unknown): EventEnvelope {
  return {
    cursor: 1,
    job_id: "01HJOB",
    stage_id: null,
    task_id: null,
    created_at: 1_700_000_000_000,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    event: { type: "job-paused", job_id: "01HJOB", reason: reason as any },
  };
}

describe("scopedPausePointId", () => {
  it("extracts point_id from the serde-JSON wire shape", () => {
    expect(
      scopedPausePointId({
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        "scoped-pause-point": { point_id: POINT_ID },
      } as any),
    ).toBe(POINT_ID);
  });

  it("accepts the specta-TS hyphen variant for forward compatibility", () => {
    // Wrappers that hand-build StopReason from the generated TS shape
    // would spell the inner field `point-id`. Reading both keeps the
    // chip lookup uniform across producers without forcing a wire
    // migration when specta's hyphenation rule changes.
    expect(
      scopedPausePointId({
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        "scoped-pause-point": { "point-id": POINT_ID },
      } as any),
    ).toBe(POINT_ID);
  });

  it("returns null for the string variants", () => {
    expect(scopedPausePointId("user")).toBeNull();
    expect(scopedPausePointId("cost-cap")).toBeNull();
    expect(scopedPausePointId(null)).toBeNull();
    expect(scopedPausePointId(undefined)).toBeNull();
  });
});

describe("liveItemFromEvent — job-paused with ScopedPausePoint reason", () => {
  it("renders a distinct planned-pause divider label", () => {
    const item = liveItemFromEvent(
      pausedEnv({ "scoped-pause-point": { point_id: POINT_ID } }),
    );
    expect(item).not.toBeNull();
    if (item == null || item.kind !== "lifecycle") {
      throw new Error("expected lifecycle item");
    }
    expect(item.label).toContain("paused at scoped point");
    expect(item.label).toContain(POINT_ID);
    expect(item.tone).toBe("warn");
  });

  it("still renders the legacy `user` / `cost-cap` strings as before", () => {
    const u = liveItemFromEvent(pausedEnv("user"));
    if (u == null || u.kind !== "lifecycle") throw new Error("expected lifecycle");
    expect(u.label).toBe("paused");
    const c = liveItemFromEvent(pausedEnv("cost-cap"));
    if (c == null || c.kind !== "lifecycle") throw new Error("expected lifecycle");
    expect(c.label).toBe("paused: cost-cap");
  });
});

describe("stopReasonLabel", () => {
  it("returns the string variant unchanged", () => {
    expect(stopReasonLabel("user")).toBe("user");
    expect(stopReasonLabel("cost-cap")).toBe("cost-cap");
  });

  it("formats the scoped-pause-point object so it never lands as JSX", () => {
    expect(
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      stopReasonLabel({ "scoped-pause-point": { point_id: POINT_ID } } as any),
    ).toBe(`scoped-pause-point:${POINT_ID}`);
  });

  it("returns empty for null / undefined", () => {
    expect(stopReasonLabel(null)).toBe("");
    expect(stopReasonLabel(undefined)).toBe("");
  });
});
