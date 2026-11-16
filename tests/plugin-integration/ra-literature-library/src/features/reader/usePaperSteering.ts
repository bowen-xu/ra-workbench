import type { AgentConversationProjection, AgentPaperSteerRequest } from "@research-assistant/core/agent";
import { agentPaperSubmissionGateway } from "../../../../../../apps/web/src/core/public/agent";
import { useAgentSteeringOwners } from "../../../../../../apps/web/src/core/public/agentSubmission/AgentSteeringProvider";
import { useAgentSteering } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentSteering";

type Scope = Omit<AgentPaperSteerRequest, "inputId" | "text">;
const send = (request: AgentPaperSteerRequest) => agentPaperSubmissionGateway.steer(request);

export function usePaperSteering(
  scope: Scope | null,
  accepted: (text: string) => void,
  projection: AgentConversationProjection | null,
  host?: { readonly key: string; readonly conversationId: string; readonly queueRejected?: import("../../../../../../ra-workbench/apps/web/src/core/public/agentSubmission/useAgentSteering").AgentSteeringSubmitCallbacks["queueRejected"]; readonly captureAcknowledgment: (text: string) => () => void },
  historyGate?: { readonly isReady: () => boolean; readonly state: unknown },
) {
  const owners = useAgentSteeringOwners();
  return useAgentSteering(
    scope,
    scope ? `${scope.conversationId}/${scope.paperId}/${scope.turnId}` : "",
    send,
    accepted,
    projection,
    owners && host ? { registry: owners.papers, hostKey: host.key, conversationId: host.conversationId, captureAcknowledgment: host.captureAcknowledgment, ...(host.queueRejected ? { queueRejected: host.queueRejected } : {}) } : undefined,
    historyGate,
  );
}
