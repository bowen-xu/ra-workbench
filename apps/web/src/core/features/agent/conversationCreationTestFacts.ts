import type { AgentConversationSummary, AgentCreateBoundProof, AgentCreateBoundRequest } from "@research-assistant/core/agent";
export const creationConversationId = "15d0a879-c170-4309-8f46-18d916f2a3a7";
export const creationHostId = "37b9dab9-0a0f-4aa5-b71a-29cc10b672b4";
export const creationEpoch = "27ca4d65-8a0a-4daa-ab26-8bcb850a8d55";
export const creationSummary: AgentConversationSummary = { id: creationConversationId, title: "当前会话", archived: false,
  durableCount: 0, createdAt: "2026-10-26T00:00:00Z", updatedAt: "2026-10-26T00:00:00Z" };
export function creationProof(request: AgentCreateBoundRequest): AgentCreateBoundProof {
  return { schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
    family: "agent.conversation.create_bound", epoch: creationEpoch, sqlFence: "9007199254740993", replayed: false, result: null };
}
