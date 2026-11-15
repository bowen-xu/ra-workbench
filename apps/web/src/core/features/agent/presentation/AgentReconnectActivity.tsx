import { ChevronRight } from "lucide-react";
import { AGENT_MODEL_MAX_RECONNECTS } from "@research-assistant/core/agent";
import { ShimmerText } from "../../../public/ui";

/** Consecutive attempts share a disclosure at their position in the process. */
export function AgentReconnectActivity({retries, reconnecting, attempt}: {
  readonly retries: readonly {readonly attemptId: string; readonly reason: string}[]; readonly reconnecting: boolean; readonly attempt: number | undefined;
}): React.JSX.Element {
  return <>
    {reconnecting && <div className="writing-agent-turn-header writing-agent-live-activity" data-agent-activity="reconnecting" role="status">
      <ShimmerText>{`正在重新连接… ${attempt ?? 1}/${AGENT_MODEL_MAX_RECONNECTS}`}</ShimmerText>
    </div>}
    <details className="writing-agent-reconnect-history" data-agent-activity="reconnect-history">
      <summary className="writing-agent-turn-header"><ChevronRight aria-hidden="true" className="writing-agent-turn-chevron" size={14} />重连记录（{retries.length} 条）</summary>
      <ol>{retries.map(retry => <li key={retry.attemptId}>{retry.reason}</li>)}</ol>
    </details>
  </>;
}
