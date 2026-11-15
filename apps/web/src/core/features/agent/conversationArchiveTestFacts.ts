import { AGENT_ARCHIVE_FAMILY, type AgentArchiveProof, type AgentArchiveRequest } from "@research-assistant/core/agent";

export function archiveProof(request: AgentArchiveRequest): AgentArchiveProof {
  return { schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
    family: AGENT_ARCHIVE_FAMILY, epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "7", replayed: false, result: null };
}
