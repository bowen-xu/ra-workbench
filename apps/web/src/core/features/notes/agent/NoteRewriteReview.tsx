import {NoteRewriteDecision} from "./NoteRewriteDecision";
import { useEffect, useState } from "react";
import type { AgentNoteApplication, AgentNoteReviewScope, AgentNoteRewriteReview } from "@research-assistant/core/agent";
import { noteBlockId, notePageId, noteDocumentPlainText, type NoteInlineBlockContentV1 } from "@research-assistant/core/notes";
import { agentNoteReviewGateway } from "../../../services/agentGateway";
import { notify } from "../../../services/notifications";

interface Props {
  readonly scope: AgentNoteReviewScope;
  readonly disabled: boolean;
  readonly currentRevision: number;
  readonly isSaved: boolean;
  readonly onAccept?: (review: AgentNoteRewriteReview) => Promise<void>;
  readonly onUndo?: (review: AgentNoteRewriteReview, application: AgentNoteApplication) => Promise<void>;
  readonly acceptanceBlocked?: boolean;
}
function plain(content: NoteInlineBlockContentV1, scope: AgentNoteReviewScope, blockId: string): string {
  return noteDocumentPlainText({schemaVersion:1,pageId:notePageId(scope.pageId),blocks:[{
    id:noteBlockId(blockId),type:"paragraph",props:{},content,children:[],
  }]});
}
/** This is an explicit native review, not interpretation of assistant prose.
 * Changing scope or hiding the review invalidates its pending response. */
export function NoteRewriteReview({scope,disabled,currentRevision,isSaved,onAccept,onUndo,acceptanceBlocked=false}: Props): React.JSX.Element {
  const [open,setOpen] = useState(false);
  const [refresh,setRefresh] = useState(0);
  const [review,setReview] = useState<AgentNoteRewriteReview | null>(null);
  const [phase,setPhase] = useState<"idle"|"loading"|"ready"|"failed">("idle");
  const {conversationId,pageId,turnId,itemId} = scope;
  useEffect(()=>{
    if (!open || disabled) return;
    let alive=true;
    setPhase("loading");setReview(null);
    void agentNoteReviewGateway.review({conversationId,pageId,turnId,itemId}).then(value=>{
      if(alive){setReview(value);setPhase("ready");}
    },()=>{
      if(alive){setPhase("failed");notify.error("无法核验这条改写提案，请刷新后重试。");}
    });
    return ()=>{alive=false;};
  },[open,disabled,conversationId,pageId,turnId,itemId,refresh]);
  const shown=review && Object.entries(scope).every(([key,value])=>review.scope[key as keyof AgentNoteReviewScope]===value) ? review : null;
  const stale=shown && (!shown.baselineMatches || currentRevision!==shown.currentRevision || !isSaved);
  return <section className="note-rewrite-review" aria-label="笔记改写提案">
    <button type="button" aria-expanded={open} disabled={disabled} onClick={()=>setOpen(value=>!value)}>{open?"收起改写提案":"审阅改写提案"}</button>
    {open && !disabled && <div>
      {phase==="loading" && <p role="status">正在核验原提案与笔记版本…</p>}
      {phase==="failed" && <button type="button" onClick={()=>setRefresh(value=>value+1)}>重试核验</button>}
      {phase==="ready" && shown && <>
        <p>提案基于版本 {shown.proposal.revision.split(":")[1]}</p>
        <p role="status">{stale?"笔记已有变化或尚未保存；当前提案不能直接应用。":"检查时与已保存的笔记版本一致。"}</p>
        {shown.proposal.edits.map(edit=><div className="note-rewrite-block" key={edit.blockId}>
          <small>块：{edit.blockId}</small>
          <div className="note-rewrite-columns">
            <div><strong>原文</strong><pre>{plain(edit.before,scope,edit.blockId) || "（空）"}</pre></div>
            <div><strong>建议</strong><pre>{plain(edit.after,scope,edit.blockId) || "（空）"}</pre></div>
          </div>
        </div>)}
        <details><summary>查看格式、引用和来源详情</summary><pre>{JSON.stringify(shown.proposal,null,2)}</pre></details>
        <NoteRewriteDecision key={`${conversationId}/${pageId}/${turnId}/${itemId}/${shown.proposal.proposalId}`} scope={scope} proposalId={shown.proposal.proposalId}
          {...(onAccept?{onAccept:()=>onAccept(shown)}:{})} currentRevision={currentRevision} isSaved={isSaved} {...(onUndo?{onUndo:(application: AgentNoteApplication)=>onUndo(shown,application)}:{})} acceptDisabled={!!stale} operationBlocked={acceptanceBlocked}/>
        <p>查看提案不会改变正文。接受后可通过编辑器撤销。</p>
        <button type="button" onClick={()=>setRefresh(value=>value+1)}>重新核验版本</button>
      </>}
    </div>}
  </section>;
}
