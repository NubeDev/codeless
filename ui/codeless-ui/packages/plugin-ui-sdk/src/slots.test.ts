/**
 * Slot vocabulary parsing. Tests pin the v0.1 contract: which slots
 * exist, which are parameterised, and how the id string is split.
 * Adding a slot must update SLOT_VOCABULARY *and* break a test here
 * so the contract change is visible in the diff.
 */
import { describe, it, expect } from "vitest";
import {
  SLOT_VOCABULARY,
  SLOT_NAMES,
  parseSlotId,
  isKnownSlot,
} from "./slots";

describe("v0.1 slot vocabulary", () => {
  it("locks the five slot names", () => {
    expect(new Set(SLOT_NAMES)).toEqual(
      new Set([
        "assistant-panel",
        "tool-result",
        "persona-picker",
        "settings-page",
        "composer-attachment-action",
      ]),
    );
  });

  it("marks parameterised slots correctly", () => {
    expect(SLOT_VOCABULARY["assistant-panel"].parameterised).toBe(false);
    expect(SLOT_VOCABULARY["tool-result"].parameterised).toBe(true);
    expect(SLOT_VOCABULARY["persona-picker"].parameterised).toBe(true);
    expect(SLOT_VOCABULARY["settings-page"].parameterised).toBe(true);
    expect(SLOT_VOCABULARY["composer-attachment-action"].parameterised).toBe(
      true,
    );
  });
});

describe("parseSlotId", () => {
  it("accepts a bare non-parameterised slot", () => {
    expect(parseSlotId("assistant-panel")).toEqual({
      name: "assistant-panel",
      shape: SLOT_VOCABULARY["assistant-panel"],
      arg: null,
      raw: "assistant-panel",
    });
  });

  it("rejects a parameter on a non-parameterised slot", () => {
    expect(parseSlotId("assistant-panel:something")).toBeNull();
  });

  it("accepts a parameter on a parameterised slot", () => {
    const parsed = parseSlotId("tool-result:notes.append");
    expect(parsed).not.toBeNull();
    expect(parsed!.name).toBe("tool-result");
    expect(parsed!.arg).toBe("notes.append");
  });

  it("rejects a parameterised slot with no argument", () => {
    expect(parseSlotId("tool-result")).toBeNull();
    expect(parseSlotId("tool-result:")).toBeNull();
  });

  it("rejects unknown slot names", () => {
    expect(parseSlotId("never-heard-of-it")).toBeNull();
    expect(parseSlotId("never-heard-of-it:arg")).toBeNull();
  });

  it("rejects empty and non-string ids", () => {
    expect(parseSlotId("")).toBeNull();
    // @ts-expect-error type-checking covers, runtime guard too
    expect(parseSlotId(undefined)).toBeNull();
  });

  it("retains the raw id on parsed results", () => {
    expect(parseSlotId("settings-page:notes")!.raw).toBe("settings-page:notes");
  });

  it("isKnownSlot agrees with parseSlotId", () => {
    expect(isKnownSlot("composer-attachment-action:notes")).toBe(true);
    expect(isKnownSlot("composer-attachment-action")).toBe(false);
  });
});
