import {useMemo} from "react";
import type {AgentQueueState} from "@research-assistant/core/agent";
import type {AgentQueueGateway} from "@research-assistant/data/agent";
import {agentContextGateway, type AgentContextGateway} from "../../../services/agentContextGateway";
import {useAgentInputQueue} from "../useAgentInputQueue";
import {useAgentQueueOwners} from "../AgentQueueProvider";

const unavailable: AgentQueueGateway<{conversationId:string}, AgentQueueState> = {
  read: async () => {throw new Error("会话队列不可用");},
  edit: async () => {throw new Error("会话队列不可用");},
};
/** The independent host adapts identity only; queue ownership and retries stay shared. */
export function useConversationInputQueue(gateway: AgentContextGateway, id: string, ready: boolean, refresh: number) {
  const owners = useAgentQueueOwners();
  const scope = useMemo(() => gateway.queue ? {conversationId:id} : null, [gateway,id]);
  return useAgentInputQueue(gateway.queue ?? unavailable, scope, ready, refresh,
    gateway.queue === agentContextGateway.queue ? owners?.conversations : undefined);
}
