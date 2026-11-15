import type { AgentUserInput } from "@research-assistant/core/agent";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import { AgentUserMessage, AgentUserMessageEvidence } from "./AgentUserMessage";
import { AgentUserMessageScope } from "./AgentUserMessageScope";
import type { AgentResourceGateways } from "./agentResourceGateways";

/** A local presentation only; the host retains the original submission and draft. */
export interface PendingAgentSubmission {
  readonly id: string;
  readonly conversationId: string;
  readonly turnId?: string | undefined;
  readonly text: string;
  readonly userInput?: readonly AgentUserInput[] | undefined;
}

function inputKey(input: readonly AgentUserInput[]): string {
  return JSON.stringify(input, (_key, value: unknown) => value && typeof value === "object" && !Array.isArray(value)
    ? Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right))) : value);
}

/** The initial user item is written with TurnStarted, before any later steer. */
export function hasConfirmedSubmissionMessage(pending: PendingAgentSubmission, messages: readonly WritingAgentMessage[]): boolean {
  if (pending.turnId === undefined) return false;
  const initial = messages.find(message => message.conversationId === pending.conversationId
    && message.turnId === pending.turnId && message.role === "user");
  return !!initial && initial.streaming === false && initial.interrupted !== true && initial.content === pending.text
    && (initial.userInput === undefined || pending.userInput === undefined || inputKey(initial.userInput) === inputKey(pending.userInput));
}

export function AgentPendingSubmissionBubble({ pending, messages, resourceGateways }: {
  readonly pending: PendingAgentSubmission | null;
  readonly messages: readonly WritingAgentMessage[];
  readonly resourceGateways?: AgentResourceGateways | undefined;
}): React.JSX.Element | null {
  if (!pending || hasConfirmedSubmissionMessage(pending, messages)) return null;
  const message: WritingAgentMessage = {
    id: pending.id, conversationId: pending.conversationId, role: "user", content: pending.text,
    ...(pending.turnId === undefined ? {} : { turnId: pending.turnId }),
    ...(pending.userInput === undefined ? {} : { userInput: pending.userInput }),
  };
  return <AgentUserMessageScope message={message} read={undefined}>
    <article className="writing-agent-message user" data-agent-message-id={message.id} data-pending-submission="true">
      <AgentUserMessageEvidence message={message} />
      <div className="writing-agent-message-bubble" data-selectable-text>
        <AgentUserMessage message={message} gateways={resourceGateways} />
      </div>
    </article>
  </AgentUserMessageScope>;
}
