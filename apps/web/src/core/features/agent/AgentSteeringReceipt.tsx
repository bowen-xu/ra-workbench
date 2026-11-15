import "./AgentSteeringReceipt.css";
import { AgentRecoveryMessage } from "./AgentRecoveryMessage";

export function AgentSteeringReceipt({uncertain,pending,text,status,requestId,disabled,onRetry,awaitingHistory=[],transferFailures=[],onRetryTransfer,hidePendingMessages=false}:{
  readonly uncertain:boolean;readonly pending:boolean;readonly text:string|undefined;
  readonly status:string;readonly requestId?:string;readonly disabled:boolean;readonly onRetry:()=>void;
  readonly awaitingHistory?:readonly {readonly inputId:string;readonly text:string}[]|undefined;
  readonly transferFailures?:readonly string[]|undefined;readonly onRetryTransfer?:(inputId:string)=>void;
  readonly hidePendingMessages?:boolean;
}):React.JSX.Element|null {
  const shownHistory=hidePendingMessages?awaitingHistory.filter(input=>transferFailures.includes(input.inputId)):awaitingHistory;
  if(!uncertain&&!pending&&!shownHistory.length)return null;
  return <>
  {shownHistory.some(input=>!transferFailures.includes(input.inputId)) && <section aria-label="待确认补充说明" className="agent-steering-pending" role="status">
    {shownHistory.filter(input=>!transferFailures.includes(input.inputId)).map(input=><p key={input.inputId}>{!hidePendingMessages&&<span>{input.text}</span>}<small>等待确认</small></p>)}
  </section>}
  {shownHistory.filter(input=>transferFailures.includes(input.inputId)).map(input=><AgentRecoveryMessage key={input.inputId} id={`steering-transfer-${input.inputId}`} label="队列消息恢复" content="消息已进入对话，但暂时无法确认是否已从队列移除。" diagnosticText={`I=${input.inputId}`}>
    {onRetryTransfer && <div className="agent-failure-actions"><button type="button" disabled={disabled} onClick={()=>onRetryTransfer(input.inputId)}>核对队列移除</button></div>}
  </AgentRecoveryMessage>)}
  {uncertain && <AgentRecoveryMessage id="steering-recovery" label="补充说明接收确认" content={status}
    {...(requestId ? {diagnosticText: `R=${requestId}`} : {})}>
    <details><summary>查看原补充说明</summary><pre>{text}</pre></details>
    <div className="agent-failure-actions"><button type="button" disabled={disabled||pending} onClick={onRetry}>重试原补充说明</button></div>
  </AgentRecoveryMessage>}
  </>;
}
