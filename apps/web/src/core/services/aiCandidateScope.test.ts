// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { selectAppRuntime } from "../runtime/appRuntime";
import { invoke } from "../runtime/platformCore";
import { getAiAssistanceGateway } from "./aiAssistanceGateway";

vi.mock("../runtime/platformCore", () => ({ invoke: vi.fn(), isDesktopRuntime: () => true }));
afterEach(() => vi.clearAllMocks());

it("passes the paper scope to native SQL while preserving project-wide history calls", async () => {
  selectAppRuntime({ mode: "desktop", storage: localStorage, gateways: {}, dispose() {} });
  vi.mocked(invoke).mockResolvedValue([]);
  const gateway = getAiAssistanceGateway();
  await gateway.listCandidates(undefined, { paperId: "paper" });
  expect(invoke).toHaveBeenLastCalledWith("list_ai_candidates", { projectId: undefined, paperId: "paper" });
  await gateway.listCandidates("project");
  expect(invoke).toHaveBeenLastCalledWith("list_ai_candidates", { projectId: "project" });
});
