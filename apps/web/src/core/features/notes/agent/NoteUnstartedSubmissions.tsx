import {useEffect,useState} from "react";
import type {AgentNoteReceipt,AgentNoteRecoveryPage,AgentReadingNoteReceipt,AgentNoteQueueScope,AgentNoteQueueAdmission,AgentNoteQueueRecoveryPage} from "@research-assistant/core/agent";
import {agentNoteRecoveryGateway,agentReadingNoteRecoveryGateway,agentNoteQueueGateway} from "../../../services/agentGateway";
import {useAgentRequestGuard} from "../../agent/useAgentMountedGuard";

type Props = {
  readonly conversationId:string;
  readonly disabled:boolean;
} & ({readonly pageId:string;readonly onReview:(receipt:AgentNoteReceipt)=>void;} | {readonly readingNoteId:string;readonly onReview:(receipt:AgentReadingNoteReceipt)=>void;} | {readonly queueScope:AgentNoteQueueScope;readonly onReview:(admission:AgentNoteQueueAdmission)=>void;});
/** Key by host/session. Opening reads metadata only, never starts or refreshes sources. */
export function NoteUnstartedSubmissions(props:Props):React.JSX.Element {
  const [open,setOpen]=useState(false);
  const [after,setAfter]=useState<string|null>(null);
  const [revision,setRevision]=useState(0);
  const beginRequest=useAgentRequestGuard();
  const [state,setState]=useState<{page:(Pick<AgentNoteRecoveryPage,"items"|"nextCursor">|Pick<AgentNoteQueueRecoveryPage,"items"|"nextCursor">)|null;loading:boolean;failed:boolean}>({page:null,loading:false,failed:false});
  useEffect(()=>{
    if(!open || props.disabled) return;
    const active=beginRequest();
    setState({page:null,loading:true,failed:false});
    const pending="queueScope" in props?agentNoteQueueGateway.list({...props.queueScope,after,limit:20}):"pageId" in props
      ? agentNoteRecoveryGateway.list({conversationId:props.conversationId,pageId:props.pageId,after,limit:20})
      : agentReadingNoteRecoveryGateway.list({conversationId:props.conversationId,readingNoteId:props.readingNoteId,after,limit:20});
    void pending
      .then(page=>{if(active())setState({page,loading:false,failed:false});})
      .catch(()=>{if(active())setState({page:null,loading:false,failed:true});});
  },[beginRequest,open,props.disabled,props.conversationId,"queueScope" in props?props.queueScope.scope.resourceId:"pageId" in props?props.pageId:props.readingNoteId,after,revision]);
  return <section className="note-agent-recovery" aria-label={"queueScope" in props?"笔记队列未启动恢复":"笔记未启动提交恢复"}>
    <button type="button" aria-expanded={open} onClick={()=>setOpen(value=>!value)}>{"queueScope" in props?"检查队列中未启动的提交":"检查未启动的提交"}</button>
    {open && <>
      <p>找回已准备、未发现启动记录的消息。查看原版本后重新确认，不自动发送。</p>
      {props.disabled ? <p role="status">请先处理当前运行或预览，并确认会话可用。</p> : <>
        {state.loading && <p role="status">正在读取提交记录…</p>}
        {state.failed && <p role="alert">读取失败，原记录仍保留，请重试。</p>}
        {!state.loading && !state.failed && state.page?.items.length===0 && <p>本页没有未启动的提交。</p>}
        {state.page?.items.map(item=><div key={item.requestId}>
          <small>{new Date(item.createdAt).toLocaleString()}</small><p>{item.messageExcerpt}</p>
          <button type="button" onClick={()=>{if("queueScope" in props){if("queue" in item)props.onReview(item);}else if("pageId" in props)props.onReview({conversationId:props.conversationId,pageId:props.pageId,requestId:item.requestId});else props.onReview({conversationId:props.conversationId,readingNoteId:props.readingNoteId,requestId:item.requestId});}}>{"queueScope" in props?"查看队列原提交":"查看原提交"}</button>
        </div>)}
        <button type="button" disabled={state.loading} onClick={()=>setRevision(value=>value+1)}>刷新提交记录</button>
        {after && <button type="button" disabled={state.loading} onClick={()=>setAfter(null)}>返回第一页</button>}
        {state.page?.nextCursor && <button type="button" disabled={state.loading} onClick={()=>setAfter(state.page?.nextCursor ?? null)}>下一页</button>}
      </>}
    </>}
  </section>;
}
