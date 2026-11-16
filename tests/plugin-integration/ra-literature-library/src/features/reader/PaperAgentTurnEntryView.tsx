import { AgentMessageData, type ReadMessageData } from "../../../../../../apps/web/src/core/public/agentPresentation/AgentMessageData";
import type { PaperAgentActivityGroup, PaperAgentTurnEntry } from "../../../../../../../plugins/ra-literature-library/src/features/reader/paperAgentProjection";
import { AgentMessageCopyButton } from "../../../../../../apps/web/src/core/public/agentPresentation/AgentMessageCopyButton";
import { PaperAiReadingMarkdown } from "../../../../../../../plugins/ra-literature-library/src/features/reader/PaperAiReadingMarkdown";

function formatPaperActivityContent(content: string): string {
  try { return JSON.stringify(JSON.parse(content), null, 2); } catch { return content; }
}

function PaperAgentActivity({ group }: { readonly group: PaperAgentActivityGroup }): React.JSX.Element {
  const error = group.items.some((item) => item.error);
  const name = group.items.find((item) => item.name)?.name ?? "工具";
  const steps = group.items.filter((item) => item.kind === "tool-call").length || group.items.length;
  return <details className={`paper-ai-reading-activity${error ? " error" : ""}`}>
    <summary>{error ? "工具失败" : group.items.some((item) => !item.completed) ? "工作中" : "已完成"} · {name}{steps > 1 ? ` · ${steps} 步` : ""}<span>查看步骤</span></summary>
    <div className="paper-ai-reading-activity-body">{group.items.map((item) => <details key={item.id} className="paper-ai-reading-activity-item">
      <summary>{item.kind === "tool-call" ? "调用" : item.error ? "工具失败" : "工具结果"}{item.name ? ` · ${item.name}` : ""}</summary>
      {item.content.trim() && <pre>{formatPaperActivityContent(item.content)}</pre>}
    </details>)}</div>
  </details>;
}

export function PaperAgentTurnEntryView({ entry, onJump, readMessageData }: { readonly readMessageData?: ReadMessageData; readonly entry: PaperAgentTurnEntry; readonly onJump: (sourceId: string) => void }): React.JSX.Element {
  if (entry.kind === "activity") return <><PaperAgentActivity group={entry.group} /><AgentMessageData messages={entry.group.items.map(item => ({ id: item.id, role: "assistant", content: item.content, rawData: item }))} read={readMessageData} /></>;
  const bubble = entry.message.role === "user"
    ? <div className="paper-ai-reading-message-row paper-ai-reading-message-row-user" key={entry.message.id}><p className="paper-ai-reading-user">{entry.message.content}</p><AgentMessageData messages={[entry.message]} read={readMessageData} className="paper-ai-reading-message-copy" /><AgentMessageCopyButton className="paper-ai-reading-message-copy" value={entry.message.content} /></div>
    : <div className="paper-ai-reading-message-row paper-ai-reading-message-row-assistant" key={entry.message.id}><PaperAiReadingMarkdown onJump={onJump} value={entry.message.content} /><AgentMessageData messages={[entry.message]} read={readMessageData} className="paper-ai-reading-message-copy" /><AgentMessageCopyButton className="paper-ai-reading-message-copy" value={entry.message.content} /></div>;
  return bubble;
}
