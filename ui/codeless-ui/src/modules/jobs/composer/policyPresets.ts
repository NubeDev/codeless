import type { AutoBypassPolicy } from "@/lib/rpc";

// Sentinel for "no auto-bypass policy" in the picker. Per
// DOCS/AUTO-BYPASS-DECISIONS.md the safe default is None — a stage
// failure halts the job and waits for the operator. The other
// options pre-authorise the runtime to advance past a non-cap
// failure with the matching guidance threaded into the next stage's
// prompt. Cap-breach failures always halt regardless (Q2). Lifted
// out of `SubmitJobDialog` so the assistant `draft_job` card and
// the planner's prompt builder consume the same list — keeping the
// UI / model / dialog aligned is the whole reason this file is
// load-bearing for SCOPE-ASSISTANT-PARITY W3.
export const NO_POLICY = "__no_policy__";
export const POLICY_CUSTOM = "custom";

export type PresetPolicyKind = Exclude<AutoBypassPolicy["type"], "custom">;

export interface PolicyPreset {
  id: PresetPolicyKind;
  label: string;
  hint: string;
}

export const POLICY_PRESETS: PolicyPreset[] = [
  {
    id: "quick",
    label: "Quick",
    hint: "Smallest change that works; skip nice-to-haves and refactors.",
  },
  {
    id: "long-term",
    label: "Long-term",
    hint: "Prefer the durable fix; tests stay in sync with behaviour.",
  },
  {
    id: "cheap",
    label: "Cheap",
    hint: "Minimise tokens and tool calls; one-line fixes ship.",
  },
  {
    id: "best-judgement",
    label: "Best judgement",
    hint: "Let the runner decide quality-vs-speed with no operator present.",
  },
  {
    id: "just-code",
    label: "Just code",
    hint: "Pick a reasonable approach and ship it; do not block on questions.",
  },
  {
    id: "relentless",
    label: "Relentless",
    hint: "Never stops on stage failure; only the cost cap and wall-clock cap halt the job.",
  },
];

// Picker state → wire shape. `customComment` is held even while a
// preset is selected so toggling back to Custom restores the
// operator's text; the conversion drops it for non-Custom kinds.
export function policyFromPicker(
  kind: string,
  customComment: string,
): AutoBypassPolicy | null {
  if (kind === NO_POLICY) return null;
  if (kind === POLICY_CUSTOM) {
    const trimmed = customComment.trim();
    return trimmed.length > 0 ? { type: "custom", comment: trimmed } : null;
  }
  return { type: kind as PresetPolicyKind };
}

// Wire shape → picker state. Used by `draft_job` (planner-proposed
// policy) and `update`-card seeding to round-trip a server value
// back into the form without losing the Custom comment.
export function pickerFromPolicy(
  policy: AutoBypassPolicy | null,
): { kind: string; customComment: string } {
  if (!policy) return { kind: NO_POLICY, customComment: "" };
  if (policy.type === "custom") {
    return { kind: POLICY_CUSTOM, customComment: policy.comment };
  }
  return { kind: policy.type, customComment: "" };
}
