import { Shimmer } from "../../../public/ui";
import { AgentMessageData, type ReadMessageData } from "../../../public/agentPresentation/AgentMessageData";
import { WritingAgentMarkdown } from "./WritingAgentMarkdown";
import type { WritingAgentMessage } from "./WritingAgentMessage";
import "./AgentReasoningSummary.css";

/** Public model summary, owned by the turn's process disclosure. No native JSON decoding. */
export function AgentReasoningSummary({ message, read, matched }: {
  readonly message: WritingAgentMessage;
  readonly read?: ReadMessageData | undefined;
  readonly matched: boolean;
}): React.JSX.Element | null {
  const parts = message.reasoningSummaryParts?.filter(part => part.text.trim());
  if (!parts?.length) return null;
  return <section className={`agent-reasoning-summary${matched ? " search-match" : ""}`} aria-label={message.interrupted ? "未完成的公开思考摘要" : "公开思考摘要"}
    data-agent-message-id={message.id} tabIndex={-1}>
    <div className="agent-reasoning-summary-body">
      <Shimmer as="div" active={message.streaming === true} data-selectable-text>{parts.map(part => <div key={part.index} data-summary-part-index={part.index}>
        <WritingAgentMarkdown value={part.text} streaming={message.streaming === true} />
      </div>)}</Shimmer>
    </div>
    <AgentMessageData messages={[message]} read={read} />
  </section>;
}
