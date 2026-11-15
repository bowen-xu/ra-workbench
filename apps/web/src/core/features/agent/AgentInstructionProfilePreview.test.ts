import { expect, it } from "vitest";
import { effectiveInstructionBudget } from "./AgentInstructionProfilePreview";

it("shows the final instruction budget after session and turn overrides", () => {
  const input = { instructionConfigs: [{ environment_id: "note:id", layers: [
    { origin: { tier: "user", source_id: "profile", revision: "r" }, instructions: { max_bytes: 10, fallback_filenames: null } },
    { origin: { tier: "session", source_id: "session", revision: "r" }, instructions: { max_bytes: 6, fallback_filenames: null } },
    { origin: { tier: "turn", source_id: "turn", revision: "r" }, instructions: { max_bytes: null, fallback_filenames: null } },
  ] }] } as never;
  expect(effectiveInstructionBudget(input)).toBe(6);
  expect(effectiveInstructionBudget({} as never)).toBe(null);
});
