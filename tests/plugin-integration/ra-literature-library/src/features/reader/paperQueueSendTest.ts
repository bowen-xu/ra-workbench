import {act} from "react";
import {expect, it, vi, type Mock} from "vitest";
import {agentQueueEditIntentFromRequest, type AgentPaperQueueResponse} from "@research-assistant/core/agent";
import {agentPaperQueueGateway} from "../../../../../../apps/web/src/core/public/agent";
import {getAppStorage} from "@research-assistant/workbench-sdk/storage";

/** Exercise the production paper panel and its shared queue/preparation owners. */
export function registerPaperQueueSendTest(ports: {gateway: {list: Mock; prepareSubmission: Mock; prepareStatus: Mock; start: Mock};
  render(): Promise<void>; flush(): Promise<void>; getHost(): HTMLElement}): void {
  it("freezes a paper-origin queue item before starting and removes the proven original", async () => {
    const conversationId = "a1111111-1111-4111-8111-111111111111", inputId = "e1111111-1111-4111-8111-111111111111";
    const epoch = "f1111111-1111-4111-8111-111111111111";
    const original: AgentPaperQueueResponse = {schemaVersion: 1, conversationId, paperId: "paper-1", revision: 2,
      paused: false, preparingInputId: null, inputs: [{inputId, text: "Queued question", origin: {kind: "paper", resourceId: "paper-1"}}]};
    let queue = original;
    ports.gateway.list.mockResolvedValue({conversations: [{id: conversationId, title: "Paper", archived: false, updatedAt: "now"}]});
    window.localStorage.setItem("research-assistant.agent-session.paper.paper-1", conversationId);
    vi.mocked(agentPaperQueueGateway.read).mockImplementation(async () => structuredClone(queue));
    vi.mocked(agentPaperQueueGateway.edit).mockImplementation(async request => {
      expect(request.edit).toEqual({type: "remove", inputId}); expect(request.expectedRevision).toBe(2);
      queue = {...queue, revision: 3, inputs: []}; return structuredClone(queue);
    });
    vi.mocked(agentPaperQueueGateway.inspectEdit!).mockImplementation(async request => ({schemaVersion: 1,
      intent: agentQueueEditIntentFromRequest(request), status: {type: "committed", proof: {schemaVersion: 1,
        operationId: request.requestId, conversationId, family: "agent.input_queue.edit", epoch, sqlFence: "1", replayed: true, result: null},
      queue: {schemaVersion: 1, conversationId, revision: 3, paused: false, preparingInputId: null, inputs: []}}}));
    ports.gateway.prepareStatus.mockImplementation(async intent => ({schemaVersion: 1, intent, status: !ports.gateway.prepareSubmission.mock.calls.length ? {type: "unconfirmed", detail: {operationId: intent.requestId, epoch, sqlFence: "0", phase: "paper-prepare-status", sqliteCode: null, sqliteExtendedCode: null, safeCause: "paper_prepare_worker_unconfirmed"}} : {type: "committed",
      proof: {schemaVersion: 1, operationId: intent.requestId, conversationId, family: "agent.paper.submission.prepare", epoch, sqlFence: "1", replayed: true, result: null},
      result: await ports.gateway.prepareSubmission.mock.results[0]!.value}}));
    await ports.render();
    for (let i = 0; i < 10; i += 1) await ports.flush();
    expect(ports.gateway.prepareSubmission).toHaveBeenCalledOnce();
    expect(ports.gateway.start).toHaveBeenCalledOnce();
    await act(async () => {for (let i = 0; i < 8; i += 1) await Promise.resolve();});
    expect(agentPaperQueueGateway.edit).toHaveBeenCalledOnce();
    expect(queue.inputs).toEqual([]);
    expect([...Array(getAppStorage().length).keys()].map(index => getAppStorage().key(index)).some(key => key?.startsWith("research-assistant:paper-prepare:v1:"))).toBe(false);
    expect(ports.getHost().querySelector('[aria-label="论文提交待核对"]')).toBeNull();
  });
}
