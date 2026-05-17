import { describe, expect, it } from "vitest";

import {
  NO_POLICY,
  POLICY_CUSTOM,
  pickerFromPolicy,
  policyFromPicker,
} from "./policyPresets";

describe("policyFromPicker", () => {
  it("returns null for the sentinel", () => {
    expect(policyFromPicker(NO_POLICY, "")).toBeNull();
  });

  it("returns null for Custom with no comment", () => {
    expect(policyFromPicker(POLICY_CUSTOM, "   ")).toBeNull();
  });

  it("trims the custom comment", () => {
    expect(policyFromPicker(POLICY_CUSTOM, "  do it  ")).toEqual({
      type: "custom",
      comment: "do it",
    });
  });

  it("maps a preset kind through unchanged", () => {
    expect(policyFromPicker("long-term", "")).toEqual({ type: "long-term" });
    expect(policyFromPicker("relentless", "")).toEqual({ type: "relentless" });
  });
});

describe("pickerFromPolicy", () => {
  it("round-trips null", () => {
    expect(pickerFromPolicy(null)).toEqual({
      kind: NO_POLICY,
      customComment: "",
    });
  });

  it("round-trips a preset", () => {
    expect(pickerFromPolicy({ type: "cheap" })).toEqual({
      kind: "cheap",
      customComment: "",
    });
  });

  it("preserves the custom comment", () => {
    expect(pickerFromPolicy({ type: "custom", comment: "halt on shell" }))
      .toEqual({ kind: POLICY_CUSTOM, customComment: "halt on shell" });
  });
});
