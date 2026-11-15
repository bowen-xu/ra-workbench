import { useEffect, useState, useSyncExternalStore } from "react";
import type { AgentWritingQueueResponse, AgentWritingQueueScope } from "@research-assistant/core/agent";
import type { AgentInputQueueController, AgentInputQueueRegistry } from "@research-assistant/data/agent";
import type { CanvasQueueFaultPort, CanvasQueueFailure } from "./agentCanvasFaultPorts";
import { agentCanvasPreviewError } from "./agentCanvasPreviewError";
import { notify } from "../../services/notifications";

const failures: ReadonlyArray<readonly [CanvasQueueFailure, string]> = [
  ["rejected", "明确拒绝"],
  ["beforeCommit", "执行前失败"],
  ["afterCommit", "执行后丢失回执"],
  ["unavailable", "持续无法连接"],
];
const emptySnapshot = () => null;
const emptySubscribe = () => () => undefined;

/** Only injects mock I/O. The existing host owns all error and recovery UI. */
export function AgentCanvasQueueErrors({ faults, registry, scope, ready }: {
  readonly faults: CanvasQueueFaultPort;
  readonly registry: AgentInputQueueRegistry<AgentWritingQueueScope, AgentWritingQueueResponse>;
  readonly scope: AgentWritingQueueScope;
  readonly ready: boolean;
}): React.JSX.Element {
  const [failure, setFailure] = useState<CanvasQueueFailure>("rejected");
  const [arming,setArming]=useState(false);
  const [controller, setController] = useState<AgentInputQueueController<AgentWritingQueueScope, AgentWritingQueueResponse> | null>(null);
  const { manuscriptId, conversationId } = scope;
  useEffect(() => {
    const lease = registry.acquire({ manuscriptId, conversationId });
    setController(lease?.controller ?? null);
    return () => lease?.release();
  }, [registry, manuscriptId, conversationId]);
  const state = useSyncExternalStore(controller?.subscribe ?? emptySubscribe, controller?.getSnapshot ?? emptySnapshot);
  const disabled = arming || !ready || !state?.queue || state.loading || state.editing || state.uncertain || !!state.queue.preparingInputId;
  async function trigger(): Promise<void> {
    if (disabled || !controller || !state?.queue) return;
    setArming(true);
    try {
      await faults.setNextEditFailure(failure);
      // Re-submit the current order so the fixture never adds or removes messages.
      await controller.edit({ type: "reorder", inputIds: state.queue.inputs.map(input => input.inputId) });
    } catch(cause){notify.error(agentCanvasPreviewError(cause));}
    finally {try{await faults.setNextEditFailure(null);}catch(cause){notify.error(agentCanvasPreviewError(cause));}setArming(false);}
  }
  return <div className="agent-canvas-control-row">
    <label className="agent-canvas-demo-duration">队列错误<select aria-label="模拟队列错误" value={failure}
      onChange={event => {
        const selected = failures.find(([value]) => value === event.currentTarget.value);
        if (selected) setFailure(selected[0]);
      }}>
      {failures.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
    </select></label>
    <button className="agent-canvas-demo-reset" type="button" disabled={disabled} onClick={() => void trigger()}>触发队列错误</button>
  </div>;
}
