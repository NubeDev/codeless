import { describe, expect, it } from "vitest";
import { runSubagent } from "./runSubagent";

// The persona-level whitelist gate runs before any model/tooling work, so
// these tests exercise it with stub args and never reach buildLanguageModel.
const stubArgs = {
  prompt: "noop",
  keys: {} as never,
  modelId: "" as never,
  toolContext: {} as never,
};

describe("runSubagent allowedSubagents gate", () => {
  it("rejects when the type is not in the persona whitelist", async () => {
    await expect(
      runSubagent({
        ...stubArgs,
        type: "explore",
        allowedSubagents: ["code-review"],
      }),
    ).rejects.toThrow(/not permitted/);
  });

  it("rejects when the persona whitelist is empty", async () => {
    await expect(
      runSubagent({
        ...stubArgs,
        type: "explore",
        allowedSubagents: [],
      }),
    ).rejects.toThrow(/not permitted/);
  });
});
