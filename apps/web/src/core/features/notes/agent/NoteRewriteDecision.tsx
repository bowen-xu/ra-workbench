import { ControlHint } from "../../../components/ControlHint";
import {NoteRewriteUndo} from "./NoteRewriteUndo";
import {useEffect,useRef,useState} from "react";
import type {AgentNoteReviewScope,AgentNoteRejection,AgentNoteApplication} from "@research-assistant/core/agent";
import {agentNoteRejectionGateway,agentNoteApplicationGateway} from "../../../services/agentGateway";
import {notify} from "../../../services/notifications";
import {useAgentRequestGuard} from "../../agent/useAgentMountedGuard";

interface Props {
  readonly onUndo?:(application:AgentNoteApplication)=>Promise<void>; readonly currentRevision?:number; readonly isSaved?:boolean;
  readonly scope:AgentNoteReviewScope; readonly proposalId:string;
  readonly onAccept?:()=>Promise<void>; readonly acceptDisabled?:boolean; readonly operationBlocked?:boolean;
}
/** Mount by proposal identity. Hiding discards UI replies, never a native decision. */
export function NoteRewriteDecision({scope,proposalId,onAccept,onUndo,currentRevision,isSaved=false,acceptDisabled=false,operationBlocked=false}:Props):React.JSX.Element {
  const [phase,setPhase]=useState<"loading"|"ready"|"accepting"|"rejecting"|"readFailed"|"writeFailed"|"rejected"|"applied">("loading");
  const [receipt,setReceipt]=useState<AgentNoteRejection|null>(null);
  const [application,setApplication]=useState<AgentNoteApplication|null>(null);
  const [refresh,setRefresh]=useState(0);
  const busy=useRef(false);
  const requestId=useRef<string|null>(null);
  const beginRequest=useAgentRequestGuard();
  const {conversationId,pageId,turnId,itemId}=scope;
  useEffect(()=>{
    const active=beginRequest();
    setPhase("loading");setReceipt(null);setApplication(null);requestId.current=null;busy.current=false;
    const selected={conversationId,pageId,turnId,itemId};
    void Promise.all([
      agentNoteRejectionGateway.status(selected,proposalId),
      agentNoteApplicationGateway.status(selected,proposalId),
    ]).then(([saved,applied])=>{
      if(!active())return;
      if(saved && applied)throw new Error("Conflicting note decisions");
      setReceipt(saved);setApplication(applied);setPhase(applied?"applied":saved?"rejected":"ready");
    }).catch(()=>{
      if(!active())return;
      setPhase("readFailed");notify.error("无法读取提案决定，请重试。");
    });
  },[beginRequest,conversationId,pageId,turnId,itemId,proposalId,refresh]);
  async function reject():Promise<void> {
    if(busy.current || operationBlocked || (phase!=="ready" && phase!=="writeFailed"))return;
    busy.current=true;setPhase("rejecting");
    const active=beginRequest();
    try {
      requestId.current??=crypto.randomUUID();
      const saved=await agentNoteRejectionGateway.reject({...scope},requestId.current,proposalId);
      if(!active())return;
      setReceipt(saved);setPhase("rejected");notify.success("已拒绝这条建议，笔记正文未改变。");
    } catch (reason:unknown) {
      if(!active())return;
      // The native decision changed after our read. Refresh without issuing a
      // second mutation; a historical application is never undone by rejection.
      if(reason==="decided"){setRefresh(value=>value+1);return;}
      setPhase("writeFailed");notify.error("拒绝结果尚未确认，请重试原操作。");
    } finally {
      if(active())busy.current=false;
    }
  }
  async function accept():Promise<void> {
    if(busy.current || phase!=="ready" || acceptDisabled || operationBlocked || !onAccept)return;
    busy.current=true;setPhase("accepting");
    const active=beginRequest();
    try { await onAccept(); }
    catch { /* The workspace retains recovery, even if this review is hidden. */ }
    finally {
      if(active()){busy.current=false;setRefresh(value=>value+1);}
    }
  }
  if(phase==="loading")return <p role="status">正在读取提案决定…</p>;
  if(phase==="readFailed")return <button type="button" onClick={()=>setRefresh(v=>v+1)}>重试读取决定</button>;
  if(phase==="rejected" && receipt)return <p role="status">已拒绝 · <time dateTime={receipt.createdAt}>{new Date(receipt.createdAt).toLocaleString()}</time>。原建议保留供查阅。</p>;
  if(phase==="applied" && application)return <><p role="status">已应用至版本 {application.appliedRevision} · <time dateTime={application.createdAt}>{new Date(application.createdAt).toLocaleString()}</time>。这是原应用记录，当前正文可能已有后续修改。</p>{onUndo && <NoteRewriteUndo application={application} currentRevision={currentRevision} isSaved={isSaved} blocked={operationBlocked} onUndo={onUndo}/>}</>;
  return <><p role="status">{phase==="accepting"?"正在应用并核对笔记…":operationBlocked?"当前有待核对的修改操作。":"未应用"}</p>
  {onAccept && <ControlHint content={acceptDisabled || operationBlocked?"请先保存并重新核验版本，或核对上一条修改操作。":"应用这条建议并保存笔记"}><button type="button" disabled={phase!=="ready" || acceptDisabled || operationBlocked}
     onClick={()=>void accept()}>
    {phase==="accepting"?"正在接受…":"接受这条建议"}</button></ControlHint>}
  <button type="button" disabled={phase==="rejecting" || phase==="accepting" || operationBlocked} onClick={()=>void reject()}>
    {phase==="rejecting"?"正在记录拒绝…":phase==="writeFailed"?"重试拒绝":"拒绝这条建议"}
  </button></>;
}
