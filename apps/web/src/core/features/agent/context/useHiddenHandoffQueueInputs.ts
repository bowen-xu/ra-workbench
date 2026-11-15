import { useLayoutEffect, useRef } from "react";
import type { AgentQueueState, AgentSteeringHandoffIntent } from "@research-assistant/core/agent";
import type { AgentInputQueueState, AgentSteeringHandoffState } from "@research-assistant/data/agent";
import type { WritingAgentMessage } from "../presentation/WritingAgentMessage";
import { hasConfirmedSteeringMessage, pendingSteeringMessages } from "../AgentPendingSteeringBubbles";

/** Keep the visual ownership until the queue's post-handoff read is visible.
 * Queue entries remain durable; only their duplicate presentation is hidden. */
export function useHiddenHandoffQueueInputs(identity: string, conversationId: string,
  handoff: AgentSteeringHandoffState | null, queueState: AgentInputQueueState<AgentQueueState> | null,
  messages: readonly WritingAgentMessage[]): readonly string[] {
  const current = useRef<{identity:string; intents:AgentSteeringHandoffIntent[]; readSequence:number}>({identity, intents:[], readSequence:0});
  let next = current.current.identity === identity ? current.current : {identity, intents:[], readSequence:0};
  const intents = [...(handoff?.items.map(item => item.intent) ?? []), ...(handoff?.intent ? [handoff.intent] : [])];
  if (intents.length) {
    const pending = pendingSteeringMessages({conversationId, messages, handoffs:intents});
    const eligible = intents.filter(intent => intent.conversationId === conversationId &&
      (pending.some(message => message.id === `${intent.turnId}/${intent.steeringInputId}`)
       || hasConfirmedSteeringMessage(messages, {conversationId, turnId:intent.turnId, inputId:intent.steeringInputId, text:intent.text})));
    next = {identity, intents:[...new Map([...next.intents,...eligible].map(intent=>[intent.queueInputId,intent])).values()], readSequence:queueState?.readSequence ?? 0};
  } else if (handoff?.phase === "ready" && queueState?.queue && !queueState.loading && !queueState.error
    && (queueState.readSequence ?? 0) > next.readSequence) {
    // A confirmed input cannot be displayed as queued, even if a stale read
    // still contains it. A released input returns after the authoritative read.
    next = {identity, intents:next.intents.filter(intent => queueState.queue?.inputs.some(input=>input.inputId===intent.queueInputId)
      && hasConfirmedSteeringMessage(messages, {conversationId,turnId:intent.turnId,inputId:intent.steeringInputId,text:intent.text})),readSequence:queueState.readSequence ?? 0};
  }
  useLayoutEffect(() => {current.current=next;});
  return next.intents.map(intent => intent.queueInputId);
}
