import type { AgentSteeringHandoffIntent, AgentUserInput } from "@research-assistant/core/agent";
import { useLayoutEffect, useRef } from "react";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import { AgentUserMessage, AgentUserMessageEvidence } from "./AgentUserMessage";
import { AgentUserMessageScope } from "./AgentUserMessageScope";
import type { AgentResourceGateways } from "./agentResourceGateways";
import {useAgentSteeringOwners} from "./AgentSteeringProvider";
import type {PendingSteeringOrderStore} from "./PendingSteeringOrderStore";

interface PendingSteering {
  readonly conversationId: string;
  readonly turnId: string;
  readonly inputId: string;
  readonly text: string;
  readonly userInput?: readonly AgentUserInput[];
}

interface Props {
  readonly messages: readonly WritingAgentMessage[];
  readonly conversationId: string;
  readonly orderKey?: string | undefined;
  readonly orderStore?: PendingSteeringOrderStore | undefined;
  readonly direct?: readonly PendingSteering[] | undefined;
  readonly handoffs?: readonly AgentSteeringHandoffIntent[] | undefined;
  readonly resourceGateways?: AgentResourceGateways | undefined;
}

/** Only the exact completed user item can replace its provisional text. */
export function hasConfirmedSteeringMessage(messages: readonly WritingAgentMessage[], input: {
  readonly conversationId: string; readonly turnId: string; readonly inputId: string; readonly text: string;
}): boolean {
  const id = `${input.turnId}/${input.inputId}`;
  return messages.some(message => message.id === id && message.conversationId === input.conversationId
    && message.turnId === input.turnId && message.role === "user" && message.content === input.text
    && message.streaming === false && message.interrupted !== true);
}

/** Keep unconfirmed inputs at the tail as output grows. Only their exact
 * durable user item fixes their position and removes the provisional bubble. */
export function pendingSteeringMessages({ messages, conversationId, direct = [], handoffs = [] }: Props): readonly WritingAgentMessage[] {
  const seen = new Set<string>();
  const pending: PendingSteering[] = [...handoffs.map(intent => ({
    conversationId: intent.conversationId, turnId: intent.turnId, inputId: intent.steeringInputId,
    text: intent.text, userInput: intent.userInput,
  })), ...direct];
  return pending.flatMap(input => {
    const id = `${input.turnId}/${input.inputId}`;
    if (input.conversationId !== conversationId || seen.has(id)) return [];
    seen.add(id);
    if (hasConfirmedSteeringMessage(messages, input)) return [];
    return [{ id, conversationId, turnId: input.turnId, role: "user" as const, content: input.text,
      ...(input.userInput === undefined ? {} : { userInput: input.userInput }) }];
  });
}

export function AgentPendingSteeringBubbles(props: Props): React.JSX.Element | null {
  const owners=useAgentSteeringOwners();
  const pending = pendingSteeringMessages(props);
  const key=props.orderKey??props.conversationId;
  const orderStore=props.orderStore??owners?.pendingOrder;
  const committed = useRef<{key:string;conversationId:string;ids:readonly string[]}>({key,conversationId:props.conversationId,ids:[]});
  const previous = orderStore ? orderStore.read(key,props.conversationId) : committed.current.key === key && committed.current.conversationId === props.conversationId ? committed.current.ids : [];
  const byId = new Map(pending.map(message => [message.id,message]));
  const ids = [...previous.filter(id => byId.has(id)),...pending.map(message=>message.id).filter(id=>!previous.includes(id))];
  // Commit only what was rendered; an interrupted concurrent render must not
  // reorder inputs that the user has already seen.
  useLayoutEffect(()=>{committed.current={key,conversationId:props.conversationId,ids};orderStore?.commit(key,props.conversationId,ids);});
  const messages = ids.flatMap(id=>{const message=byId.get(id);return message?[message]:[];});
  if (!messages.length) return null;
  return <>{messages.map(message => <AgentUserMessageScope key={message.id} message={message} read={undefined}>
    <article className="writing-agent-message user" data-agent-message-id={message.id} data-pending-steering="true">
      <AgentUserMessageEvidence message={message} />
      <div className="writing-agent-message-bubble" data-selectable-text>
        <AgentUserMessage message={message} gateways={props.resourceGateways} />
      </div>
      <div className="writing-agent-message-meta"><small role="status">等待处理</small></div>
    </article>
  </AgentUserMessageScope>)}</>;
}
