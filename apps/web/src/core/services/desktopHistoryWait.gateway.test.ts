// @vitest-environment happy-dom
import {expect, it, vi} from "vitest";

const native = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../runtime/platformCore", () => ({invoke: native.invoke, isDesktopRuntime: () => true}));

import {agentSessionGateway} from "./agentGateway";
import {getAppRuntime, selectAppRuntime} from "../runtime/appRuntime";

it("passes history cancellation from the production gateway to the desktop IPC adapter", async () => {
  const previous = getAppRuntime();
  selectAppRuntime({mode: "desktop", storage: {} as Storage, gateways: {}, dispose() {}});
  let rejectWait!: (error: unknown) => void;
  let finishCreate!: (response: unknown) => void;
  native.invoke.mockImplementation((command: string) => {
    if (command === "agent_create_history_wait") return Promise.resolve(12);
    if (command === "agent_wait_for_history_cancellable") return new Promise((_resolve, reject) => {rejectWait = reject;});
    if (command === "agent_cancel_history_wait") {rejectWait(new Error("cancelled by native owner")); return Promise.resolve();}
    if (command === "agent_create_conversation") return new Promise(resolve => {finishCreate = resolve;});
    throw new Error(`unexpected native command ${command}`);
  });
  try {
    const owner = new AbortController();
    const request = {conversationId: "00000000-0000-4000-8000-000000000001", afterSeq: 0, waitMs: 30_000};
    const wait = agentSessionGateway.waitForHistory(request, owner.signal);
    const creation = agentSessionGateway.create(request.conversationId);
    const rejected = expect(wait).rejects.toThrow("cancelled by native owner");
    await vi.waitFor(() => expect(native.invoke).toHaveBeenCalledWith("agent_wait_for_history_cancellable", {request, resourceId: 12}));
    owner.abort();
    await rejected;
    expect(native.invoke).toHaveBeenCalledWith("agent_cancel_history_wait", {resourceId: 12});
    const receipt = {schemaVersion: 1, conversationId: request.conversationId, watermark: 0};
    finishCreate(receipt);
    await expect(creation).resolves.toEqual(receipt);
  } finally { selectAppRuntime(previous); native.invoke.mockReset(); }
});
