import type { ReactNode } from "react";
import type { AgentConversationBinding, AgentConversationProjection, AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentForkGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { agentForkGateway, agentMetadataGateway } from "../../services/agentGateway";
import { ConversationForkAction } from "./ConversationForkAction";
import { ConversationForkRecoveries } from "./ConversationForkRecoveries";
import { forkScope } from "./conversationForkPending";
import type { ConversationForkLock } from "./conversationForkLock";
import "./ConversationForkControl.css";

interface Props {
  readonly parentId: string | null;
  readonly binding: AgentConversationBinding | null;
  readonly projection: AgentConversationProjection | null;
  readonly disabled: boolean;
  readonly onOpen?: ((conversation: AgentConversationSummary) => void) | undefined;
  readonly additionalActions?: ReactNode;
  readonly gateway?: AgentForkGateway;
  readonly metadata?: Pick<AgentMetadataGateway, "read">;
  readonly lock?: ConversationForkLock;
}
export function useConversationForkActions({ parentId, binding, projection, disabled, onOpen, additionalActions,
  gateway = agentForkGateway, metadata = agentMetadataGateway, lock }: Props): {
  readonly messageActions: (messageId: string) => ReactNode;
  readonly historyActions: ReactNode;
} {
  const lastTerminalTurnId = [...(projection?.turnOrder ?? [])].reverse().find(id => projection?.turns.get(id)?.outcome);
  function control(turnId: string, index: number): ReactNode {
    if (!parentId || !binding || !onOpen) return null;
    return <div key={turnId} data-agent-turn-id={turnId} tabIndex={-1} className="conversation-fork-control" role="group" aria-label={`第 ${index + 1} 轮分支`}>
      <ConversationForkAction key={`${binding.kind}:${binding.resourceId}:${parentId}:${turnId}`} scope={{ parentId, throughTurnId: turnId, binding }} turnLabel={`第 ${index + 1} 轮`} disabled={disabled}
        ports={{ fork: gateway, metadata, ...(lock ? {lock} : {}) }} onOpen={onOpen} />
      {turnId === lastTerminalTurnId ? additionalActions : null}
    </div>;
  }
  const actions = new Map<string, ReactNode>();
  const visibleScopes = new Set<string>();
  const emptyTurns: ReactNode[] = [];
  const standaloneAdditionalActions = additionalActions && (!onOpen || !parentId || !binding || projection?.conversationId !== parentId);
  if (onOpen && parentId && binding && projection?.conversationId === parentId) {
    projection.turnOrder.forEach((turnId, index) => {
      const turn = projection.turns.get(turnId);
      if (!turn?.outcome) return;
      if (!disabled) visibleScopes.add(forkScope({parentId, throughTurnId: turnId, binding}));
      const lastItem = turn.itemOrder.at(-1);
      if (lastItem) actions.set(`${turnId}/${lastItem}`, control(turnId, index));
      else emptyTurns.push(<div key={turnId}><span>第 {index + 1} 轮已结束，未生成消息。</span>{control(turnId, index)}</div>);
    });
  }
  if (standaloneAdditionalActions) {
    emptyTurns.push(<div key="conversation-more-actions" className="conversation-fork-control conversation-fork-control-fallback">{additionalActions}</div>);
  }
  return { messageActions: (messageId) => actions.get(messageId), historyActions: <>{emptyTurns}
    {binding && onOpen && <ConversationForkRecoveries key={`${binding.kind}:${binding.resourceId}`} binding={binding} ports={{fork: gateway, metadata, ...(lock ? {lock} : {})}} visibleScopes={visibleScopes} onOpen={onOpen} />}
    {additionalActions && !lastTerminalTurnId && !standaloneAdditionalActions ? <div className="conversation-fork-control conversation-fork-control-fallback">{additionalActions}</div> : null}</> };
}
