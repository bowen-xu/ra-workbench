import {useCallback,useEffect,useMemo,useRef,useState,type MutableRefObject} from "react";
import type {HbsSession} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";
import {textPatchGateway,subscribeTextPatches,notifyTextPatchesChanged,type TextPatchResolution,type TextPatchPageReview} from "../../agent/textPatchGateway";
import {loadNoteRecoveryDraft,saveNoteRecoveryDraft,clearNoteRecoveryDraft} from "../workspace/noteRecovery";
import {acknowledgeNoteReviewRequest,registerNoteReviewStateContributor,notifyNoteReviewSaveState,type NoteReviewSaveState,type NoteReviewMenuAction} from "./noteTextEditorPorts";
import {getAppStorage} from "../../../runtime/appStorage";
import {getAppRuntime} from "../../../runtime/appRuntime";
import {flushStorage} from "../../../runtime/storagePersistence";
import {noteReviewErrorMessage} from "./noteReviewRecovery";
import type {NoteEditorInstance} from "./noteEditorSchema";
import type {NoteEditorInstallPort} from "./noteEditorInstallPort";
import {appendSessionHistory} from "../../../components/hbs/noteSessionHistory";
import {notify} from "../../../services/notifications";

export interface StaleNoteReview {readonly conversationId:string;readonly proposalId:string;readonly generation:number;readonly page:TextPatchPageReview}
/** Inspection and recovery of the existing proposal mirror only. H/B/S choices,
 * persistence, queue and undo remain owned by the same live HBS session. */
export function useNoteStaleReview(session:HbsSession<NoteDocumentV1>,enabled:boolean,readOnly=false) {
  const pageId=session.getSnapshot().content.pageId;
  const [issues,setIssues]=useState<readonly StaleNoteReview[]>([]),[error,setError]=useState<string>(),[busy,setBusy]=useState(false);
  const [request,setRequest]=useState<TextPatchResolution>();
  const pending=useRef<TextPatchResolution|undefined>(undefined),running=useRef(false),mounted=useRef(true),inspectionFailed=useRef(false);
  const editorBinding=useRef<{editor:NoteEditorInstance;port:MutableRefObject<NoteEditorInstallPort|undefined>}|undefined>(undefined);
  const bindEditor=useCallback((editor:NoteEditorInstance,port:MutableRefObject<NoteEditorInstallPort|undefined>)=>{
    const binding={editor,port};editorBinding.current=binding;
    return()=>{if(editorBinding.current===binding)editorBinding.current=undefined;};
  },[]);
  const read=async()=>{
    const found:StaleNoteReview[]=[];
    // Native proposal mirrors do not exist in the ordinary browser runtime.
    // Match the adjacent metadata inspector; browser HBS saves keep their owner.
    if(getAppRuntime().mode === "browser")return found;
    for(const conversationId of await textPatchGateway.forPage(pageId)) {
      const review=await textPatchGateway.review(conversationId,undefined,undefined,pageId);
      if(review?.state!=="pending")continue;
      // The status menu belongs to this page, not the whole Agent conversation.
      for(const page of review.pages)if(page.pageId===pageId&&page.recovery)found.push({conversationId,proposalId:review.proposalId,generation:review.generation,page});
    }
    return found;
  };
  useEffect(()=>{
    mounted.current=true;if(!enabled)return;
    let generation=0;
    try {const recovered=loadNoteRecoveryDraft(pageId)?.review?.request;if(recovered?.action==="retain-current"){pending.current=recovered;setRequest(recovered);}}
    catch(reason){setError(noteReviewErrorMessage(reason));}
    const refresh=()=>{const current=++generation;void read().then(value=>{if(mounted.current&&current===generation){setIssues(value);if(inspectionFailed.current){inspectionFailed.current=false;setError(undefined);}}}).catch(reason=>{if(mounted.current&&current===generation){inspectionFailed.current=true;setError(noteReviewErrorMessage(reason));}});};
    refresh();const stop=subscribeTextPatches(refresh);
    return()=>{mounted.current=false;generation++;stop();};
  },[enabled,pageId]);
  async function retain(editor?:NoteEditorInstance,port?:MutableRefObject<NoteEditorInstallPort|undefined>,readOnly=false,selected?:StaleNoteReview):Promise<void>{
    if(!enabled||running.current)return;running.current=true;setBusy(true);setError(undefined);
    try {
      if(!pending.current){
        const state=session.getSnapshot();
        const existing=loadNoteRecoveryDraft(pageId)?.review?.request;
        if(existing)throw new Error(`MODEL_REQUEST_UNSETTLED：请先核对原请求 ${existing.requestId}`);
        if(state.pending||state.preservationError)throw new Error("MODEL_REQUEST_UNSETTLED：请先核对正文原保存请求，当前内容已保留");
        const choice=structuredClone(state.content);
        if(state.changes.length||state.conflicts.length){
          if(readOnly)throw new Error("MODEL_RESOURCE_INACTIVE：请先恢复笔记，再处理正文待审内容");
          if(!editor||!port?.current)throw new Error("MODEL_EDITOR_NOT_READY：编辑器尚未就绪");
          // Keep is the shared HBS choice of current for both review and
          // conflicts. Capture it in the original editor's native history.
          const entry=session.perform(()=>{
            // Resolve each live range before selecting the next: resolving a
            // text range rebases the remaining IDs and comparison ancestors.
            const visited=new Set<string>();
            while(session.getSnapshot().conflicts.length){
              const conflict=session.getSnapshot().conflicts[0]!;
              const key=JSON.stringify(conflict);
              if(visited.has(key))throw new Error("MODEL_REVIEW_FAILED：冲突范围未能推进，请在正文内核对");
              visited.add(key);session.resolveMany([conflict.id],"keep");
            }
            const changes=session.getSnapshot().changes;
            if(changes.length)session.resolveMany(changes.map(change=>change.id),"keep",{content:choice,baseline:choice});
          });
          if(!session.adapter.equal(session.getSnapshot().content,choice))throw new Error("MODEL_REVIEW_FAILED：当前内容发生变化，请重新核对");
          port.current.prepareStructure(port.current.read(),session.getSnapshot().content,undefined,{onTransaction:transaction=>appendSessionHistory(transaction,entry)})();
        }
        // Inactive pages settle only already-confirmed S=B; no body write or restore.
        if(!readOnly)await session.save();
        // Keep may already own an in-flight save. Like the normal save host,
        // observe its real acknowledgement; save() is not a receipt barrier.
        if(session.getSnapshot().pending&&!session.getSnapshot().message.startsWith("MODEL_RESULT_UNKNOWN"))await new Promise<void>(resolve=>{
          const check=()=>{const state=session.getSnapshot();if(!state.pending||state.message.startsWith("MODEL_RESULT_UNKNOWN")){stop();resolve();}};
          const stop=session.subscribe(check);check();
        });
        await session.flushPreservation();
        const acknowledged=session.getSnapshot();
        if(acknowledged.pending||acknowledged.queued||acknowledged.preservationError||acknowledged.conflicts.length
          ||!session.adapter.equal(acknowledged.content,choice)||!session.adapter.equal(choice,acknowledged.observed.saved)
          ||!session.adapter.equal(choice,acknowledged.observed.baseline))throw new Error(`MODEL_NOTE_RECOVERY_SAVE_REQUIRED：当前内容尚未确认保存，请先核对正文保存结果；${acknowledged.preservationError??acknowledged.message}`);
        const fresh=await read();if(mounted.current)setIssues(fresh);
        const issue=fresh.find(value=>value.page.pageId===pageId&&(!selected||value.conversationId===selected.conversationId&&value.proposalId===selected.proposalId));
        if(!issue){notify.success("当前内容已保留，审阅状态已核对。");return;}
        pending.current={conversationId:issue.conversationId,proposalId:issue.proposalId,generation:issue.generation,
          requestId:crypto.randomUUID(),pageId,action:"retain-current",document:acknowledged.observed.saved,expectedRevision:acknowledged.observed.version};
        setRequest(pending.current);
      }
      const original=pending.current,state=session.getSnapshot();
      saveNoteRecoveryDraft(pageId,state.observed.version,state.content,{document:state.content,baseline:state.baselineProjection,request:original});
      await flushStorage(getAppStorage());
      await textPatchGateway.resolve(original);
      acknowledgeNoteReviewRequest(pageId,original.requestId);
      if(loadNoteRecoveryDraft(pageId)?.review?.request?.requestId===original.requestId)clearNoteRecoveryDraft(pageId);
      pending.current=undefined;if(mounted.current)setRequest(undefined);
      notifyTextPatchesChanged();
      if(mounted.current)setIssues(await read());
      notify.success("已保留当前页面内容，过期提案已核对。");
    }catch(reason){
      if(reason&&typeof reason==="object"&&"status"in reason&&reason.status==="failed"){
        const id=pending.current?.requestId;
        if(id&&loadNoteRecoveryDraft(pageId)?.review?.request?.requestId===id)clearNoteRecoveryDraft(pageId);
        pending.current=undefined;if(mounted.current)setRequest(undefined);
      }
      const message=noteReviewErrorMessage(reason);if(mounted.current)setError(message);notify.error(message);throw reason;
    }finally{running.current=false;if(mounted.current)setBusy(false);}
  }
  const currentIssue=issues.some(issue=>issue.page.pageId===pageId);
  const contributes=enabled&&(issues.length>0||!!request||!!error);
  const message=request?`MODEL_NOTE_RECOVERY_REQUEST：请求 ${request.requestId} 尚未核对；请重试原操作。${error??""}`:error??(currentIssue?"MODEL_NOTE_PROPOSAL_STALE：当前页面有过期提案；请打开右侧恢复与诊断菜单核对并保留当前内容。":undefined);
  const saveState=useMemo<NoteReviewSaveState>(()=>({status:busy?"saving":request||error?"failed":currentIssue?"conflict":"saved",...(message?{message}:{})}),[busy,request,error,currentIssue,message]);
  const live=useRef({saveState,retain,actions:[] as readonly NoteReviewMenuAction[]});
  const actions=useMemo<readonly NoteReviewMenuAction[]>(()=>{
    const retainCurrent=(issue?:StaleNoteReview)=>{const binding=editorBinding.current;void live.current.retain(binding?.editor,binding?.port,readOnly,issue).catch(()=>{});};
    const result:NoteReviewMenuAction[]=issues.map(issue=>({id:`${issue.conversationId}:${issue.proposalId}:${issue.page.pageId}`,
      group:`${issue.page.title}：提案版本 ${issue.page.expectedRevision??"未知"}，当前版本 ${issue.page.recovery?.currentRevision}`,
      label:"保留当前内容并解决冲突",icon:"keep",disabled:busy||!!request,
      run:()=>retainCurrent(issue)}));
    if(request)result.push({id:"retry-recovery",group:"原请求待核对",label:"核对并重试原恢复请求",icon:"retry",disabled:busy,run:()=>retainCurrent()});
    if(error&&!issues.length&&!request)result.push({id:"refresh-recovery",group:error,label:"重新核对当前页面",icon:"retry",disabled:busy,run:()=>{void read().then(value=>{if(mounted.current){setIssues(value);setError(undefined);inspectionFailed.current=false;}}).catch(reason=>notify.error(noteReviewErrorMessage(reason)));}});
    return result;
  },[issues,pageId,busy,request,error,readOnly]);
  live.current={saveState,retain,actions};
  useEffect(()=>{if(!contributes)return;return registerNoteReviewStateContributor(pageId,{source:"metadata",getState:()=>live.current.saveState,
    menuActions:()=>live.current.actions,
    retry:async()=>{if(pending.current)await live.current.retain();else{const fresh=await read();if(mounted.current){setIssues(fresh);setError(undefined);}}},recovery:()=>{if(!pending.current)return;const state=session.getSnapshot();return{document:state.content,baseline:state.baselineProjection,request:pending.current};}});},[contributes,pageId,session]);
  useEffect(()=>{if(contributes)notifyNoteReviewSaveState();},[contributes,saveState,actions]);
  return{busy,bindEditor};
}
