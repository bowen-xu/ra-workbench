import { expect, it, vi } from "vitest";
import { EMPTY_AGENT_CONTEXT } from "@research-assistant/core/agent";
import { createWritingDemoContextGateway } from "./WritingDemoContextGateway";

it("keeps configuration revisions per conversation and rejects stale saves", async () => {
  const gateway = createWritingDemoContextGateway({create: vi.fn(), open: vi.fn(), readHistory: vi.fn(), waitForHistory: vi.fn()});
  await expect(gateway.readConfiguration!("one")).resolves.toBeNull();
  await expect(gateway.saveConfiguration!("one", 0, EMPTY_AGENT_CONTEXT)).resolves.toEqual({revision: 1, state: EMPTY_AGENT_CONTEXT});
  await expect(gateway.saveConfiguration!("one", 0, EMPTY_AGENT_CONTEXT)).rejects.toThrow("版本冲突");
  await expect(gateway.saveConfiguration!("one", 1, EMPTY_AGENT_CONTEXT)).resolves.toMatchObject({revision: 2});
  await expect(gateway.readConfiguration!("two")).resolves.toBeNull();
  await expect(gateway.readConfiguration!("one")).resolves.toEqual({revision: 2, state: EMPTY_AGENT_CONTEXT});
});
