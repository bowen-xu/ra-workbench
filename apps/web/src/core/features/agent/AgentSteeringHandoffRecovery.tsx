import type { AgentSteeringHandoffState } from "@research-assistant/data/agent";
import { AgentRecoveryMessage } from "./AgentRecoveryMessage";

/** Recovery only inspects the original operation; retry keeps its original identity. */
export function AgentSteeringHandoffRecovery({available, state, disabled, canRetry, queueContainsInput, onCheck, onRetry}: {
  readonly available: boolean;
  readonly state: AgentSteeringHandoffState | null;
  readonly disabled: boolean;
  readonly canRetry: boolean;
  readonly queueContainsInput: boolean;
  readonly onCheck: () => void;
  readonly onRetry: () => void;
}): React.JSX.Element {
  const unknown = state?.phase === "uncertain";
  const diagnostic = [state?.message, state?.intent ? `请求 ${state.intent.operationId}` : undefined].filter(Boolean).join("\n");
  return <AgentRecoveryMessage id="steering-handoff-recovery" label="队列引导核对"
    content={`${!available ? "暂时无法检查引导状态。" : "无法确认引导是否已接收。"} ${queueContainsInput ? "消息仍显示在队列中。" : "请检查对话和队列中的消息。"} 队列暂不继续，请检查状态。`}
    diagnosticText={diagnostic}>
      <div className="agent-failure-actions">
        {available && <button type="button" disabled={disabled} onClick={onCheck}>核对队列引导</button>}
        {unknown && <button type="button" disabled={disabled || !canRetry} onClick={onRetry}>重试原队列引导</button>}
      </div>
  </AgentRecoveryMessage>;
}
