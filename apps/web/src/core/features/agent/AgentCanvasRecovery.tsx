import { useState } from "react";
import type { CanvasRecoveryFaultPort, CanvasSteerFailure } from "./agentCanvasFaultPorts";
import { agentCanvasPreviewError } from "./agentCanvasPreviewError";
import { notify } from "../../services/notifications";

/** Fault injection changes only the mock gateway. Production owners handle
 * pending admission, queue fallback and reconnection presentation. */
export function AgentCanvasRecovery({faults,active,onSimulate}:{readonly faults:CanvasRecoveryFaultPort;readonly active:boolean;readonly onSimulate:()=>void}):React.JSX.Element{
  const [exhaust, setExhaust] = useState(false);
  const [pending,setPending]=useState(false);
  async function apply(action:()=>void|Promise<void>):Promise<void>{setPending(true);try{await action();}catch(cause){notify.error(agentCanvasPreviewError(cause));}finally{setPending(false);}}
  return <div className="agent-canvas-control-row">
    <label className="agent-canvas-demo-duration">补充说明错误<select aria-label="模拟补充说明错误" disabled={!active||pending} defaultValue="normal"
      onChange={event=>{
        const value=event.currentTarget.value;
        if(["normal","rejected","beforeCommit","afterCommit"].includes(value))void apply(()=>faults.setSteerFailure(value as CanvasSteerFailure));
      }}>
      <option value="normal">正常接收</option><option value="rejected">拒绝插入本轮</option>
      <option value="beforeCommit">发送前断连</option><option value="afterCommit">接收后丢失回执</option>
    </select></label>
    <label className="agent-canvas-demo-duration">中断恢复<select aria-label="模拟中断恢复" disabled={!active||pending} value={exhaust ? "exhaust" : "recover"} onChange={event => setExhaust(event.target.value === "exhaust")}>
      <option value="recover">重连后完成</option><option value="exhaust">重试耗尽</option>
    </select></label>
    <button className="agent-canvas-demo-reset" type="button" disabled={!active||pending} onClick={()=>{void apply(async()=>{await faults.disconnect(exhaust?"exhaust":"recover");onSimulate();});}}>模拟网络中断</button>
  </div>;
}
