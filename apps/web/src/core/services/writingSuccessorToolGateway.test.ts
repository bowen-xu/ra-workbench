import { expect, it, vi } from "vitest";
import { writingSuccessorToolGateway } from "./writingSuccessorToolGateway";

const platform = vi.hoisted(() => ({ listen: vi.fn(), invoke: vi.fn() }));
vi.mock("../runtime/platformEvents", () => ({ listen: platform.listen }));
vi.mock("../runtime/platformCore", () => ({ invoke: platform.invoke }));

it("receives only a valid successor identity and reports the exact result", async () => {
  let deliver: (event: { payload: unknown }) => void = () => undefined;
  platform.listen.mockImplementation(async (_name, handler) => { deliver = handler; return () => undefined; });
  platform.invoke.mockResolvedValue(undefined);
  const receive = vi.fn();
  await writingSuccessorToolGateway.listen(receive);
  const id = "00000000-0000-4000-8000-000000000001";
  const request = { requestId: id, scope: { manuscriptId: id, conversationId: id, requestId: id, contextSnapshotId: id },
    turnId: id, generation: 1, predecessorDeliveryId: id, leaseId: id };
  deliver({ payload: { ...request, generation: 0 } });
  deliver({ payload: { ...request, leaseId: "invalid" } });
  expect(receive).not.toHaveBeenCalled();
  deliver({ payload: request });
  expect(receive).toHaveBeenCalledWith(request);
  await writingSuccessorToolGateway.report({ request, confirmation: null });
  expect(platform.invoke).toHaveBeenCalledWith("agent_report_writing_successor_workspace", { result: { request, confirmation: null } });
});
