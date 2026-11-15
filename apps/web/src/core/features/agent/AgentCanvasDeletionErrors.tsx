import { useRef, useState } from "react";
import type { CanvasDeletionFaultPort, CanvasQueueFailure } from "./agentCanvasFaultPorts";
import { agentCanvasPreviewError } from "./agentCanvasPreviewError";
import { useAgentMountedGuard } from "./useAgentMountedGuard";
import { notify } from "../../services/notifications";

const failures: ReadonlyArray<readonly [CanvasQueueFailure, string]> = [
  ["afterCommit", "成功删除后丢失回执"], ["beforeCommit", "执行前失败"],
  ["rejected", "明确拒绝"], ["unavailable", "持续无法连接"],
];
/** Configures external I/O only. Users still use the actual production delete
 * and recovery controls; no synthetic deletion or receipt is generated here. */
export function AgentCanvasDeletionErrors({faults, ready, available}: {
  readonly faults: CanvasDeletionFaultPort; readonly ready: boolean; readonly available: boolean;
}): React.JSX.Element {
  const [failure, setFailure] = useState<CanvasQueueFailure>("afterCommit"), [pending, setPending] = useState(false);
  const [configured, setConfigured] = useState(false), arming = useRef(false), mounted = useAgentMountedGuard();
  async function configure(value: CanvasQueueFailure | null): Promise<void> {
    if (!ready || !available || arming.current) return;
    arming.current = true; setPending(true);
    try {await faults.setNextDeletionFailure(value); if (mounted.current) setConfigured(value !== null);}
    catch (cause: unknown) {notify.error(agentCanvasPreviewError(cause));}
    finally {arming.current = false; if (mounted.current) setPending(false);}
  }
  return <div className="agent-canvas-control-row">
    <label className="agent-canvas-demo-duration">删除错误<select aria-label="模拟删除错误" value={failure} disabled={pending || !ready || !available}
      onChange={event => {const selected = failures.find(([value]) => value === event.currentTarget.value); if (selected) setFailure(selected[0]);}}>
      {failures.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
    </select></label>
    <button className="agent-canvas-demo-reset" type="button" disabled={pending || !ready || !available} onClick={() => void configure(failure)}>设置删除故障</button>
    <button className="agent-canvas-demo-reset" type="button" disabled={pending || !ready || !available} onClick={() => void configure(null)}>清除删除故障</button>
    <span role="status">{!available ? "请重启原生预览以启用删除故障。" : configured
      ? "已发送故障设置；请使用原删除或清空控件。成功后丢回执仅一次，保留校验不消耗它。"
      : "选择已归档对话并设置故障，再使用原删除或清空控件。"}</span>
  </div>;
}
