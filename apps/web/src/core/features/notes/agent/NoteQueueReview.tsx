import { executionModeForModel } from "@research-assistant/core/agent";
import type {NoteQueueSubmissionController,NoteQueueSubmissionSnapshot} from "../../agent/noteQueueSubmissionController";
import {NoteFrozenSubmission} from "./NoteFrozenSubmission";

export function NoteQueueReview({state,controller,disabled,onChanged}:{
  readonly state:NoteQueueSubmissionSnapshot;
  readonly controller:NoteQueueSubmissionController;
  readonly disabled:boolean;
  readonly onChanged:()=>void;
}):React.JSX.Element|null {
  if(state.phase==="idle")return null;
  const model=state.preview?.input.model;
  const mode=model === undefined ? null : executionModeForModel(model, state.preview?.input.reasoningEffort);
  const mayCancel=state.preview?.kind==="pending"&&!state.outcome&&(state.phase==="preview"||state.error==="startRejected");
  return <section className="note-agent-review" aria-label="队列提交预览">
    <strong>{state.preview?.kind==="admitted"?"恢复队列原提交":"队列消息"}</strong>
    {state.phase==="preparing"&&<p role="status">正在读取或准备队列来源…</p>}
    {state.preview&&<NoteFrozenSubmission input={state.preview.input}/>}
    {state.preview&&!mode&&<p>当前模型尚未通过 Agent 工具调用验证，请取消预览后重新选择。</p>}
    {(state.phase==="preview"||state.error==="start")&&!state.outcome&&((state.preview?.kind !== "pending")||state.error === "start")&&<button type="button" disabled={disabled||!mode||state.cancelPending||!!state.cancelError} onClick={()=>{if(mode)void controller.confirm(mode).then(onChanged);}}>{state.error==="start"?"查询或继续队列原轮次":"继续发送原队列请求"}</button>}
    {state.preview?.kind === "pending" && state.phase === "preview" && !state.error && <p role="status">上下文已冻结，正在自动发送…</p>}
    {mayCancel&&<button type="button" disabled={disabled||state.cancelPending||state.cancelError==="rejected"} onClick={()=>void controller.cancel(crypto.randomUUID()).then(onChanged)}>{state.cancelError==="uncertain"?"重试原取消请求":"取消队列预览"}</button>}
    {state.cancelPending&&<p role="status">正在取消预览，消息仍留在队列。</p>}
    {state.cancelError&&<p role="alert">{state.cancelError==="uncertain"?"取消结果未确认，请按原请求重试，不要重新发送。":"队列已变化，取消未应用；请刷新原预览后检查。"}</p>}
    {(state.preview?.kind==="pending"&&(state.error==="startRejected"||state.cancelError==="rejected"))&&<button type="button" disabled={disabled||state.cancelPending} onClick={()=>void controller.refreshPending().then(onChanged)}>刷新原队列预览</button>}
    {state.error==="prepare"&&<>
      <p role="alert">队列准备或恢复未确认，消息仍由服务保留。</p>
      <button type="button" disabled={disabled} onClick={()=>void controller.retryPrepare(true).then(onChanged)}>恢复队列原回执</button>
      {state.request&&<button type="button" disabled={disabled} onClick={()=>void controller.retryPrepare().then(onChanged)}>重试队列原请求</button>}
    </>}
    {state.error==="start"&&<p role="alert">启动结果未确认，请查询原轮次；不会另建消息重发。</p>}
    {state.error==="startRejected"&&<p role="alert">队列发送未应用，请检查模型、来源或最新队列版本。</p>}
    {(state.phase==="starting"||state.phase==="started"||state.error==="start")&&!state.outcome&&<button type="button" disabled={state.stopPending||state.stopRequested} onClick={()=>void controller.stop().then(onChanged)}>停止队列任务</button>}
    {(state.stopPending||state.stopRequested)&&!state.outcome&&<p role="status">正在等待任务停止，最终状态以历史为准。</p>}
    {state.error==="stop"&&<p role="alert">无法确认停止结果，请查看对话历史后重试。</p>}
    {state.outcome&&<p role="status">队列本轮{state.outcome==="completed"?"已完成":state.outcome==="canceled"?"已停止":"未完成"}。</p>}
    {(state.outcome||(state.preview?.kind==="admitted"&&(state.phase==="preview"||state.error==="startRejected"))||(!state.request&&state.error==="prepare"))&&<button type="button" onClick={()=>{if(controller.dismiss())onChanged();}}>返回队列</button>}
  </section>;
}
