import { useAgentInputQueue } from "../../../../../../apps/web/src/core/public/agentSubmission/useAgentInputQueue";
import { useAgentQueueOwners } from "../../../../../../apps/web/src/core/public/agentSubmission/AgentQueueProvider";
import { agentPaperQueueGateway } from "../../../../../../apps/web/src/core/public/agent";
import type { AgentPaperQueueResponse, AgentPaperQueueScope } from "@research-assistant/core/agent";

/** Shared durable queue lifecycle for the literature host. Paper-specific UI
 * keeps its draft/auto-drain policy, while reads and edits use the same
 * versioned controller as writing and notes. */
export function usePaperInputQueue(scope: AgentPaperQueueScope | null, ready: boolean, refresh = 0) {
  const owners = useAgentQueueOwners();
  return useAgentInputQueue(agentPaperQueueGateway, scope, ready, refresh, owners?.papers);
}

export type PaperInputQueueState = AgentPaperQueueResponse;
