import type { AgentCompactResult } from "./agentCommandGateway";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import { AgentMessageData, type ReadMessageData } from "./AgentMessageData";
import "./AgentCompactionActivity.css";

/** Command feedback is separate from model messages and tool activity. */
export interface AgentCompactionRecord {
  readonly id: string;
  readonly conversationId: string;
  readonly afterMessageId?: string | undefined;
  readonly requestId?: string | undefined;
  readonly inputId?: string | undefined;
  readonly state: "pending" | "completed" | "failed";
  readonly result?: AgentCompactResult;
}

type Props = { readonly records: readonly AgentCompactionRecord[]; readonly message?: never; readonly read?: never }
  | { readonly message: WritingAgentMessage; readonly read?: ReadMessageData | undefined; readonly records?: never };

function durableLabel(message: WritingAgentMessage): string {
  const installed = message.compaction?.windowId != null;
  if (!message.turnFinished) return installed ? "上下文已压缩，正在完成后续处理" : "正在压缩上下文";
  if (message.turnOutcome === "failed") return installed ? "上下文已压缩，后续处理失败" : "上下文压缩失败";
  if (message.turnOutcome === "canceled" || message.turnOutcome === "interrupted") return installed ? "上下文已压缩，任务已中止" : "上下文压缩已中止";
  return installed ? "上下文已压缩" : "压缩任务已结束，未确认窗口安装";
}

/** Native messages come exclusively from the durable turn/item projection.
 * The records branch remains a compatibility port for synthetic UI fixtures. */
export function AgentCompactionActivity(props: Props): React.JSX.Element {
  if (props.message) {
    const message = props.message;
    return <section className="agent-compaction-activity" aria-label="上下文压缩" tabIndex={-1}
      data-agent-message-id={message.id} data-agent-turn-id={message.turnId}
      data-agent-compaction-operation-id={message.compaction?.operationId}
      data-agent-compaction-window-id={message.compaction?.windowId ?? undefined}
      data-agent-compaction-installed={message.compaction?.windowId != null}
      data-agent-compaction-outcome={message.turnOutcome ?? "running"}>
      <span role="status">{durableLabel(message)}</span>
      <AgentMessageData messages={[message]} read={props.read} />
    </section>;
  }
  return <>{props.records.map(record => <section key={record.id} className="agent-compaction-activity" aria-label="上下文压缩">
    <span role="status">{record.state === "pending" ? "正在压缩上下文" : record.state === "completed" ? "上下文已压缩" : "上下文压缩失败"}</span>
    <AgentMessageData data={{ command: "/compact", ...record }} />
  </section>)}</>;
}
