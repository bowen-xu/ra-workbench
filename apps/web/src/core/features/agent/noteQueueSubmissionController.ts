import {decodeAgentNoteQueueScope,decodeAgentNoteQueuePrepareRequest,decodeAgentQueueConfirmation,decodeAgentNoteQueueAdmission,decodeAgentNoteReceipt,decodeAgentReadingNoteReceipt,
  type AgentNoteQueueScope,type AgentNoteQueuePrepareRequest,type AgentNoteQueuePreview,
  type AgentNoteQueueResponse,type AgentNoteQueueCancelRequest,type AgentQueueConfirmation,
  type AgentFrozenNoteRecord,type AgentNoteQueueAdmission,type AgentExecutionMode,type AgentTurnOutcome,
} from "@research-assistant/core/agent";
import type {AgentNoteQueueGateway,AgentTurnGateway} from "@research-assistant/data/agent";
import {ReviewedSubmissionController,type ReviewedSubmissionSnapshot} from "./reviewedSubmissionController";
import {ReviewedSubmissionOwnerRegistry} from "./reviewedSubmissionOwner";

interface Receipt {
  readonly conversationId:string;
  readonly kind:"pending"|"admitted";
  readonly requestId:string|null;
  readonly inputId:string|null;
  readonly admission:AgentNoteQueueAdmission|null;
}
export interface NoteQueueReview extends AgentFrozenNoteRecord {
  readonly kind:"pending"|"admitted";
  readonly confirmation:AgentQueueConfirmation;
  readonly queue:AgentNoteQueueResponse|null;
}
export interface NoteQueueSubmissionSnapshot extends ReviewedSubmissionSnapshot<AgentNoteQueuePrepareRequest,Receipt,NoteQueueReview> {
  readonly cancelPending:boolean;
  readonly cancelError:"rejected"|"uncertain"|null;
}

export function noteQueueSubmissionOwnerKey(scope: AgentNoteQueueScope): string {
  return JSON.stringify(["noteQueue",scope.conversationId.toLowerCase(),scope.scope.kind,scope.scope.resourceId.toLowerCase()]);
}
function pending(value:AgentNoteQueuePreview):NoteQueueReview {
  return {...value.record,kind:"pending",confirmation:{inputId:value.inputId,expectedRevision:value.queue.revision},queue:value.queue};
}
/** Reuse the reviewed-submission state machine; only queue receipt and cancellation differ. */
export class NoteQueueSubmissionController {
  private readonly review:ReviewedSubmissionController<AgentNoteQueuePrepareRequest,Receipt,NoteQueueReview>;
  private readonly scope:AgentNoteQueueScope;
  private readonly listeners=new Set<()=>void>();
  private readonly unsubscribe:()=>void;
  private cancelRequest:AgentNoteQueueCancelRequest|null=null;
  private cancelPending=false;
  private cancelError:NoteQueueSubmissionSnapshot["cancelError"]=null;
  private disposed=false;
  private snapshot:NoteQueueSubmissionSnapshot;
  constructor(scope:AgentNoteQueueScope,private readonly gateway:AgentNoteQueueGateway,turns:AgentTurnGateway) {
    this.scope=decodeAgentNoteQueueScope(scope);
    this.review=new ReviewedSubmissionController({
      conversationId:this.scope.conversationId,
      decodeRequest:value=>{
        const request=decodeAgentNoteQueuePrepareRequest(value),submission=request.submission;
        const resource=submission.kind==="note"?submission.request.context.pageId:submission.request.context.readingNoteId;
        if(submission.request.conversationId!==this.scope.conversationId||submission.kind!==this.scope.scope.kind||resource!==this.scope.scope.resourceId)throw new Error("Queue scope mismatch");
        return request;
      },
      decodeReceipt:value=>{
        if(!value||typeof value!=="object"||Array.isArray(value))throw new Error("Queue receipt mismatch");
        const r=value as Record<string,unknown>;
        if(Object.keys(r).length!==5||r.conversationId!==this.scope.conversationId||!["pending","admitted"].includes(String(r.kind)))throw new Error("Queue receipt mismatch");
        const requestId=r.requestId===null?null:this.scope.scope.kind==="note"
          ?decodeAgentNoteReceipt({conversationId:r.conversationId,pageId:this.scope.scope.resourceId,requestId:r.requestId}).requestId
          :decodeAgentReadingNoteReceipt({conversationId:r.conversationId,readingNoteId:this.scope.scope.resourceId,requestId:r.requestId}).requestId;
        const inputId=r.inputId===null?null:decodeAgentQueueConfirmation({inputId:r.inputId,expectedRevision:0}).inputId;
        if(r.kind==="pending"&&r.admission===null&&(requestId===null)===(inputId===null))return {conversationId:this.scope.conversationId,kind:"pending",requestId,inputId,admission:null};
        const admission=decodeAgentNoteQueueAdmission(r.admission);
        if(r.kind!=="admitted"||admission.requestId!==requestId||admission.queue.inputId!==inputId)throw new Error("Queue admission mismatch");
        return {conversationId:this.scope.conversationId,kind:"admitted",requestId,inputId,admission};
      },
      receipt:request=>({conversationId:this.scope.conversationId,kind:"pending",requestId:request.submission.request.requestId,inputId:request.inputId,admission:null}),
    },{
      prepare:async request=>pending(await gateway.prepare(request)),
      restore:async receipt=>{
        if(receipt.kind==="admitted"&&receipt.admission&&receipt.requestId){
          const record=await gateway.restore(this.scope,receipt.requestId);
          if(record.turnId!==receipt.admission.turnId)throw new Error("Queue admission turn mismatch");
          return {...record,kind:"admitted",confirmation:decodeAgentQueueConfirmation(receipt.admission.queue),queue:null};
        }
        const result=await gateway.recover(this.scope);
        if(!result||(receipt.inputId&&result.inputId!==receipt.inputId)||(receipt.requestId&&result.record.requestId!==receipt.requestId))throw new Error("Pending queue preview mismatch");
        return pending(result);
      },
      start:(review,mode)=>gateway.start(this.scope,{conversationId:review.conversationId,requestId:review.requestId,turnId:review.turnId,contextSnapshotId:review.contextSnapshotId,input:review.input},review.confirmation,mode),
    },turns);
    this.snapshot={...this.review.getSnapshot(),cancelPending:false,cancelError:null};
    this.unsubscribe=this.review.subscribe(()=>this.publish());
  }
  getSnapshot=():NoteQueueSubmissionSnapshot=>this.snapshot;
  subscribe=(listener:()=>void):(()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
  private publish():void {
    if(this.disposed)return;
    this.snapshot={...this.review.getSnapshot(),cancelPending:this.cancelPending,cancelError:this.cancelError};
    for(const listener of this.listeners)listener();
  }
  prepare(request:AgentNoteQueuePrepareRequest, acknowledgment?:()=>void):Promise<void> {return this.review.prepare(request, acknowledgment);}
  recoverPending():Promise<void> {
    return this.review.recover({conversationId:this.scope.conversationId,kind:"pending",requestId:null,inputId:null,admission:null});
  }
  recoverAdmission(admission:AgentNoteQueueAdmission):Promise<void> {
    return this.review.recover({conversationId:this.scope.conversationId,kind:"admitted",requestId:admission.requestId,inputId:admission.queue.inputId,admission:structuredClone(admission)});
  }
  retryPrepare(restore=false):Promise<void> {return this.review.retryPrepare(restore);}
  async confirm(mode:AgentExecutionMode):Promise<void> {
    if(this.cancelPending||this.cancelError)return;
    await this.review.confirm(mode);
  }
  observeOutcome(turnId:string,outcome:AgentTurnOutcome):void {this.review.observeOutcome(turnId,outcome);}
  stop():Promise<void> {return this.review.stop();}
  async cancel(requestId:string):Promise<boolean> {
    const state=this.review.getSnapshot(),preview=state.preview;
    if(this.disposed||this.cancelPending||!preview||preview.kind!=="pending"||!(state.phase==="preview"||state.error==="startRejected"))return false;
    this.cancelRequest??={...this.scope,requestId,previewRequestId:preview.requestId,...preview.confirmation};
    const request=structuredClone(this.cancelRequest),wasUncertain=this.cancelError==="uncertain";
    this.cancelPending=true;this.cancelError=null;this.publish();
    try {
      await this.gateway.cancel(request);
      if(this.disposed)return false;
      this.cancelRequest=null;this.cancelPending=false;this.cancelError=null;this.review.dismiss();this.publish();return true;
    } catch(error) {
      if(this.disposed)return false;
      const reason=error&&typeof error==="object"&&"reason" in error?error.reason:null;
      const code=reason&&typeof reason==="object"&&"code" in reason?reason.code:null;
      const rejected=!wasUncertain&&["invalid","conflict","busy","paused","capacity","closed"].includes(String(code));
      if(rejected)this.cancelRequest=null;
      this.cancelPending=false;this.cancelError=rejected?"rejected":"uncertain";this.publish();return false;
    }
  }
  async refreshPending():Promise<void> {
    const state=this.review.getSnapshot(),preview=state.preview;
    if(this.cancelPending||this.cancelError==="uncertain"||preview?.kind!=="pending"||!(state.phase==="preview"||state.error==="startRejected"))return;
    const receipt:Receipt={conversationId:this.scope.conversationId,kind:"pending",inputId:preview.confirmation.inputId,requestId:preview.requestId,admission:null};
    if(!this.review.dismiss())return;
    this.cancelRequest=null;this.cancelError=null;
    await this.review.recover(receipt);
  }
  dismiss():boolean {
    const state=this.review.getSnapshot();
    if(this.cancelPending||this.cancelError||(!state.outcome&&(state.request||state.preview?.kind==="pending")))return false;
    return this.review.dismiss();
  }
  dispose():void {this.disposed=true;this.unsubscribe();this.review.dispose();this.listeners.clear();}
}

/** Queue review is also pre-admission state: changing the selected session
 * must not dispose a pending/uncertain head before its owner can be resumed. */
export const noteQueueSubmissionOwners = new ReviewedSubmissionOwnerRegistry<NoteQueueSubmissionController>(
  controller => {
    const phase = controller.getSnapshot().phase;
    return phase !== "idle" && phase !== "started";
  },
);
export function resetNoteQueueSubmissionOwnersForTests(): void { noteQueueSubmissionOwners.resetForTests(); }
