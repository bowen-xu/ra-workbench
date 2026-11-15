import type { AgentRenameProof, AgentRenameRequest, AgentConversationSummary } from "@research-assistant/core/agent";

export const renameConversationId = "11111111-1111-4111-8111-111111111111";
export const renameEpoch = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
export const renameSummary: AgentConversationSummary = { id: renameConversationId, title: "更新名称", archived: false,
  durableCount: 3, createdAt: "2026-10-11T00:00:00Z", updatedAt: "2026-10-11T00:01:00Z" };
export function renameProof(request: AgentRenameRequest): AgentRenameProof {
  return { schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
    family: "agent.conversation.rename", epoch: renameEpoch, sqlFence: "9007199254740993", replayed: false, result: null };
}
