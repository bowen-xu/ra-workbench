import { decodeAgentWritingQueueEditRequest, decodeAgentWritingQueueResponse, decodeAgentWritingQueuePreview, executionModeForModel,
  type AgentWritingQueueScope, type AgentWritingQueueResponse, type AgentWritingQueuePreviewResponse,
  type AgentWritingQueuedCompactionPrepareRequest, type AgentWritingQueuedCompactionSettleRequest,
  type AgentQueuedCompactionOutcome } from "@research-assistant/core/agent";
import {decodeAgentWritingQueueCancelRequest, decodeAgentWritingQueuePreviewRequest, sameAgentWritingQueuePreviewRequest,
  decodeAgentWritingQueuedStartRequest, sameAgentWritingQueuedStartRequest, decodeAgentWritingQueuedStartStatusResponse,
  type AgentWritingQueuedStartRequest, type AgentWritingStartRequest, type AgentWritingStartResponse, type AgentWritingQueuedStartStatusResponse,
  type AgentWritingQueuePrepareStatusResponse, type AgentWritingQueuePreviewRequest, type AgentWritingPrepareRequest,
  type AgentSubmissionPreview, type AgentWritingReceipt, type AgentWritingQueueCancelStatusResponse, type AgentWritingQueueCancelRequest} from "@research-assistant/core/agent";
import type { AgentInputQueueGateway, AgentInputQueuePreviewGateway, AgentSubmissionGateway } from "@research-assistant/data/agent";
import {decodeAgentSteeringHandoffIntent, type AgentSteeringHandoffIntent} from "@research-assistant/core/agent";

const key = (scope: AgentWritingQueueScope) => JSON.stringify([scope.manuscriptId, scope.conversationId]);
function cancelConflict(intent: AgentWritingQueueCancelRequest): never {
  throw {code: "storage", reason: {conversationId: intent.conversationId, manuscriptId: intent.manuscriptId, inputId: intent.inputId,
    previewRequestId: intent.previewRequestId, operationId: intent.requestId, code: "DATA_SQL_DOMAIN", reason: "conversation changed since it was read"}};
}
export type WritingDemoQueueFailure = "rejected" | "beforeCommit" | "afterCommit" | "unavailable";
export type WritingDemoQueueMaterializer = (request: AgentWritingPrepareRequest) => Promise<{preview: AgentSubmissionPreview; commit: (accept: () => void) => void}>;
export type WritingDemoStartFailure = "beforeAdmission" | "afterAdmission" | "afterStarted" | "unavailable";
interface WritingDemoQueuedStartIo {
  readonly validate: (request: AgentWritingStartRequest) => void;
  readonly inspect: (request: AgentWritingStartRequest, preview?: AgentSubmissionPreview) => Promise<AgentWritingStartResponse | null>;
}
interface WritingDemoQueuedAdmission {
  /** A synthetic client envelope and business admission, not a SQL/mode proof. */
  readonly intent: AgentWritingQueuedStartRequest;
  readonly preview: AgentSubmissionPreview;
}
type HandoffResolution={readonly intent:AgentSteeringHandoffIntent;readonly action:"finish"|"release"|"cancel";readonly operationId:string};
function prepareConflict(intent: AgentWritingQueuePreviewRequest): never {
  return cancelConflict({conversationId: intent.submission.conversationId, manuscriptId: intent.submission.context.manuscriptId,
    inputId: intent.inputId, previewRequestId: intent.submission.requestId, requestId: intent.submission.requestId, expectedRevision: intent.expectedRevision});
}
/** Mock durable queue I/O. Ordering, optimistic UI, automatic preparation and
 * uncertain-request recovery remain in the shared production controllers. */
export class WritingDiffDemoQueue {
  snapshot() { return structuredClone({ records: this.records, edits: this.edits, previews: this.previews, cancellations: this.cancellations, preparations: this.preparations, compactions: this.compactions, starts: this.starts, handoffs:this.handoffs, handoffResolutions:this.handoffResolutions, handoffEpoch:this.handoffEpoch }); }
  restore(snapshot: ReturnType<WritingDiffDemoQueue["snapshot"]>): void {
    this.receipts.clear();
    this.records.clear(); for (const entry of snapshot.records) this.records.set(entry[0], {
      ...entry[1], paused: true, revision: entry[1].revision + (entry[1].paused ? 0 : 1),
    });
    this.edits.clear(); for (const entry of snapshot.edits) this.edits.set(entry[0], entry[1]);
    this.previews.clear(); for (const entry of snapshot.previews ?? []) this.previews.set(entry[0], entry[1]);
    this.cancellations.clear(); for (const entry of snapshot.cancellations ?? []) this.cancellations.set(entry[0], entry[1]);
    this.preparations.clear(); for (const entry of snapshot.preparations ?? []) this.preparations.set(entry[0], entry[1]);
    this.compactions.clear(); for (const entry of snapshot.compactions ?? []) this.compactions.set(entry[0], entry[1]);
    this.starts.clear(); for (const [id, entry] of snapshot.starts ?? []) this.starts.set(id, {...entry, intent: decodeAgentWritingQueuedStartRequest(entry.intent)});
    this.handoffs.clear(); for(const [id,intent] of snapshot.handoffs ?? [])this.handoffs.set(id,decodeAgentSteeringHandoffIntent(intent));
    this.handoffResolutions.clear();for(const [id,resolution] of snapshot.handoffResolutions ?? [])this.handoffResolutions.set(id,resolution);
    this.handoffEpoch=snapshot.handoffEpoch ?? crypto.randomUUID();
  }

  private readonly records = new Map<string, AgentWritingQueueResponse>();
  private readonly handoffs = new Map<string, AgentSteeringHandoffIntent>();
  private readonly handoffResolutions = new Map<string,HandoffResolution>();
  private handoffEpoch:string=crypto.randomUUID();
  private handoffEnvironment:(()=>string)|undefined;
  private readonly edits = new Map<string, { readonly request: string; readonly result: AgentWritingQueueResponse }>();
  private readonly previews = new Map<string, AgentWritingQueuePreviewResponse>();
  private readonly receipts = new Map<string, { readonly request: string; readonly result: Promise<unknown>; readonly conversationId: string }>();
  private nextEditFailure: WritingDemoQueueFailure | null = null;
  private nextCancelFailure: WritingDemoQueueFailure | null = null;
  private nextPrepareFailure: WritingDemoQueueFailure | null = null;
  private nextStartFailure: WritingDemoStartFailure | null = null;
  private startReaderUnavailable = false;
  private readonly starts = new Map<string, WritingDemoQueuedAdmission>();
  private readonly preparations = new Map<string, AgentWritingQueuePrepareStatusResponse>();
  private readonly compactions = new Map<string, string>();
  private readonly cancellations = new Map<string, AgentWritingQueueCancelStatusResponse>();
  constructor(private readonly submission: AgentSubmissionGateway,
    private readonly ensureAvailable: (id: string) => void = () => undefined,
    private readonly materialize?: WritingDemoQueueMaterializer,
    private readonly inspectFrozen?: (scope: AgentWritingReceipt) => AgentSubmissionPreview | null,
    private readonly startIo?: WritingDemoQueuedStartIo,
    private readonly compactCancellation?: (intent: AgentWritingQueueCancelRequest) => (() => void) | null) {}
  readCompactionRetirement(scope: AgentWritingReceipt, inputId: string) {
    return [...this.cancellations.values()].find(value => value.status.type === "committed"
      && value.intent.conversationId === scope.conversationId && value.intent.manuscriptId === scope.manuscriptId
      && value.intent.previewRequestId === scope.requestId && value.intent.inputId === inputId);
  }
  /** Bind before restoring a snapshot; a mismatched restored epoch is fenced. */
  bindHandoffEnvironment(epoch:()=>string):void{
    if(this.handoffEnvironment||this.handoffs.size)throw new Error("MOCK_HANDOFF_ENVIRONMENT_ALREADY_BOUND");
    this.handoffEnvironment=epoch;this.handoffEpoch=epoch();
  }
  handoffDataEpoch():string{
    if(this.handoffEnvironment && this.handoffEnvironment()!==this.handoffEpoch)throw {code:"conflict"};
    return this.handoffEpoch;
  }
  /** One-shot I/O fault; afterCommit retains the durable receipt for exact retry. */
  failNextEdit(failure: WritingDemoQueueFailure | null): void { this.nextEditFailure = failure; }
  failNextCancel(failure: WritingDemoQueueFailure | null): void { this.nextCancelFailure = failure; }
  failNextPrepare(failure: WritingDemoQueueFailure | null): void {this.nextPrepareFailure = failure;}
  failNextStart(failure: WritingDemoStartFailure | null): void {this.nextStartFailure = failure;}
  failStartStatus(unavailable: boolean): void {this.startReaderUnavailable = unavailable;}
  /** Native stop pauses the durable queue before publishing a terminal event. */
  pauseForStop(scope: AgentWritingQueueScope): void {
    const current = this.read(scope);
    if (!current.paused) this.put(scope, { ...current, revision: current.revision + 1, paused: true });
  }
  deleteConversation(id: string): void {
    for(const [key,intent] of this.handoffs)if(intent.conversationId===id)this.handoffs.delete(key);
    for(const [key,resolution] of this.handoffResolutions)if(resolution.intent.conversationId===id)this.handoffResolutions.delete(key);
    for (const [scope, value] of this.records) if (value.conversationId === id) this.records.delete(scope);
    for (const [scope, value] of this.previews) if (value.preview.conversationId === id) this.previews.delete(scope);
    for (const [request, value] of this.edits) if (value.result.conversationId === id) this.edits.delete(request);
    for (const [request, value] of this.receipts) if (value.conversationId === id) this.receipts.delete(request);
    for (const [request, value] of this.cancellations) if (value.intent.conversationId === id) this.cancellations.delete(request);
    for (const [request, value] of this.preparations) if (value.intent.submission.conversationId === id) this.preparations.delete(request);
    for (const [request, conversationId] of this.compactions) if (conversationId === id) this.compactions.delete(request);
    for (const [request, entry] of this.starts) if (entry.intent.start.conversationId === id) this.starts.delete(request);
  }
  private async once<T>(identity: string, conversationId: string, request: unknown, action: () => Promise<T>, retention: "receipt" | "pending" = "receipt"): Promise<T> {
    this.ensureAvailable(conversationId);
    const serialized = JSON.stringify(request), previous = this.receipts.get(identity);
    if (previous) {
      if (previous.request !== serialized) throw { code: "conflict" };
      return structuredClone(await previous.result) as T;
    }
    const result = Promise.resolve().then(action), entry = { request: serialized, result, conversationId };
    this.receipts.set(identity, entry);
    try { return structuredClone(await result); }
    catch (error) { if (this.receipts.get(identity) === entry) this.receipts.delete(identity); throw error; }
    finally {if (retention === "pending" && this.receipts.get(identity) === entry) this.receipts.delete(identity);}
  }
  private read(scope: AgentWritingQueueScope): AgentWritingQueueResponse {
    this.ensureAvailable(scope.conversationId);
    return structuredClone(this.records.get(key(scope)) ?? decodeAgentWritingQueueResponse({ schemaVersion: 1,
      manuscriptId: scope.manuscriptId, conversationId: scope.conversationId,
      revision: 0, paused: false, preparingInputId: null, inputs: [] }));
  }
  private put(scope: AgentWritingQueueScope, next: AgentWritingQueueResponse): AgentWritingQueueResponse {
    this.ensureAvailable(scope.conversationId);
    const value = decodeAgentWritingQueueResponse(next); this.records.set(key(scope), value); return structuredClone(value);
  }
  /** Mock authority boundary: the native queue transaction performs the same
   * immutable snapshot and ownership checks before accepting a handoff. */
  claimSteeringHandoff(value:AgentSteeringHandoffIntent,activeTurn:string|null):{epoch:string;replayed:boolean}{
    const intent=decodeAgentSteeringHandoffIntent(structuredClone(value));
    if(intent.scope.kind!=="manuscript"||intent.expectedEpoch!==this.handoffDataEpoch())throw {code:"conflict"};
    const existing=this.handoffs.get(intent.steeringInputId);
    if(existing){if(JSON.stringify(existing)!==JSON.stringify(intent))throw {code:"conflict"};return {epoch:this.handoffEpoch,replayed:true};}
    if(this.handoffResolutions.has(intent.operationId))throw {code:"conflict"};
    if(activeTurn!==intent.turnId||this.edits.has(intent.operationId)||this.preparations.has(intent.operationId)||this.cancellations.has(intent.operationId)
      ||this.compactions.has(intent.operationId)||this.starts.has(intent.operationId)
      ||[...this.handoffs.values()].some(item=>item.operationId===intent.operationId || item.conversationId===intent.conversationId && item.queueInputId===intent.queueInputId))throw {code:"conflict"};
    const current=this.read({conversationId:intent.conversationId,manuscriptId:intent.scope.manuscriptId}),input=current.inputs[intent.queueIndex];
    if(current.revision!==intent.expectedRevision||current.preparingInputId||current.paused!==intent.queuePaused||!input||input.inputId!==intent.queueInputId
      ||!!input.paused!==intent.inputPaused||input.text!==intent.text||JSON.stringify(input.userInput??[])!==JSON.stringify(intent.userInput)
      ||JSON.stringify(input.origin??null)!==JSON.stringify(intent.origin))throw {code:"conflict"};
    this.handoffs.set(intent.steeringInputId,intent);return {epoch:this.handoffEpoch,replayed:false};
  }
  private handoffOwns(conversationId:string,inputId:string):boolean{
    return [...this.handoffs.values()].some(intent=>intent.conversationId===conversationId&&intent.queueInputId===inputId);
  }
  readSteeringHandoffs(conversationId:string,expectedEpoch?:string):readonly AgentSteeringHandoffIntent[]{
    const epoch=this.handoffDataEpoch();this.ensureAvailable(conversationId);
    if(expectedEpoch!==undefined&&expectedEpoch!==epoch)throw {code:"conflict"};
    return structuredClone([...this.handoffs.values()].filter(intent=>intent.conversationId===conversationId));
  }
  steeringHandoffResolution(intent:AgentSteeringHandoffIntent,epoch:string):"unconfirmed"|"finished"|"released"|"canceled"{
    this.ensureAvailable(intent.conversationId);
    if(epoch!==this.handoffDataEpoch()||intent.expectedEpoch!==null&&intent.expectedEpoch!==epoch)throw {code:"conflict"};
    const saved=this.handoffResolutions.get(intent.operationId);
    if(!saved)return "unconfirmed";
    if(JSON.stringify(saved.intent)!==JSON.stringify(intent))throw {code:"conflict"};
    return saved.action==="finish"?"finished":saved.action==="release"?"released":"canceled";
  }
  settleSteeringHandoff(intent:AgentSteeringHandoffIntent,epoch:string,action:HandoffResolution["action"],operationId:string,
    history:"confirmed"|"unconfirmed"|"incompleteOrConflicting",admission:"absent"|"matching"|"conflicting",terminal:boolean,discardAdmission?:()=>void):{epoch:string;replayed:boolean}{
    this.ensureAvailable(intent.conversationId);
    if(epoch!==this.handoffDataEpoch()||intent.expectedEpoch!==epoch)throw {code:"conflict"};
    const prior=this.handoffResolutions.get(intent.operationId);
    if(prior){if(prior.action!==action||prior.operationId!==operationId||JSON.stringify(prior.intent)!==JSON.stringify(intent))throw {code:"conflict"};return {epoch,replayed:true};}
    if(this.edits.has(operationId)||this.preparations.has(operationId)||this.cancellations.has(operationId)||this.compactions.has(operationId)||this.starts.has(operationId)
      ||[...this.handoffResolutions.values()].some(item=>item.operationId===operationId)||this.handoffUsesOperation(operationId))throw {code:"conflict"};
    const saved=this.handoffs.get(intent.steeringInputId);
    if(action==="cancel"){
      if(saved || [...this.handoffs.values()].some(item=>item.operationId===intent.operationId))throw {code:"conflict"};
    }else{
      if(!saved||JSON.stringify(saved)!==JSON.stringify(intent)||intent.scope.kind!=="manuscript")throw {code:"conflict"};
      if(action==="finish"&&(history!=="confirmed"||admission!=="matching")||action==="release"&&(history!=="unconfirmed"||admission==="conflicting"||!terminal||admission==="matching"&&!discardAdmission))throw {code:"conflict"};
      const scope={conversationId:intent.conversationId,manuscriptId:intent.scope.manuscriptId},queue=this.read(scope);
      const input=queue.inputs.find(item=>item.inputId===intent.queueInputId);
      if(!input||input.text!==intent.text||JSON.stringify(input.userInput??[])!==JSON.stringify(intent.userInput)||JSON.stringify(input.origin??null)!==JSON.stringify(intent.origin))throw {code:"conflict"};
      this.put(scope,{...queue,revision:queue.revision+(action==="finish"||!queue.paused?1:0),paused:action==="release"?true:queue.paused,
        inputs:action==="finish"?queue.inputs.filter(item=>item.inputId!==intent.queueInputId):queue.inputs});
      if(action==="release")discardAdmission?.();
      this.handoffs.delete(intent.steeringInputId);
    }
    this.handoffResolutions.set(intent.operationId,{intent:structuredClone(intent),action,operationId});return {epoch,replayed:false};
  }
  private handoffUsesOperation(operationId:string):boolean{
    return [...this.handoffs.values()].some(intent=>intent.operationId===operationId)||this.handoffResolutions.has(operationId)
      ||[...this.handoffResolutions.values()].some(item=>item.operationId===operationId);
  }
  /** Mock transaction boundary used only by the native compaction I/O adapter. */
  assertCompactionClaim(request: AgentWritingQueuedCompactionPrepareRequest): AgentWritingQueueResponse {
    const scope = { conversationId: request.submission.conversationId, manuscriptId: request.submission.context.manuscriptId };
    const current = this.read(scope), head = current.inputs.find(input => !input.paused);
    if (current.revision !== request.expectedQueueRevision || current.paused || current.preparingInputId || this.handoffOwns(current.conversationId,request.inputId)
      || head?.inputId !== request.inputId || head.text !== request.submission.text
      || JSON.stringify(head.userInput ?? []) !== JSON.stringify(request.submission.userInput ?? [])) throw { code: "conflict" };
    return current;
  }
  claimCompaction(request: AgentWritingQueuedCompactionPrepareRequest): AgentWritingQueueResponse {
    const current = this.assertCompactionClaim(request);
    const result = this.put(current, { ...current, revision: current.revision + 1, preparingInputId: request.inputId });
    this.compactions.set(request.submission.requestId, request.submission.conversationId); return result;
  }
  settleCompaction(request: AgentWritingQueuedCompactionSettleRequest, original: AgentWritingQueuedCompactionPrepareRequest,
    outcome: AgentQueuedCompactionOutcome): AgentWritingQueueResponse {
    const current = this.read(request), input = current.inputs.find(item => item.inputId === request.inputId);
    if (current.revision !== request.expectedQueueRevision || current.preparingInputId !== request.inputId
      || input?.text !== original.submission.text || JSON.stringify(input?.userInput ?? []) !== JSON.stringify(original.submission.userInput ?? [])) throw { code: "conflict" };
    return this.put(request, { ...current, revision: current.revision + 1, preparingInputId: null,
      inputs: outcome === "completed" ? current.inputs.filter(item => item.inputId !== request.inputId) : current.inputs,
      paused: outcome === "completed" ? current.paused : true });
  }
  readonly gateway: AgentInputQueueGateway = {
    read: async scope => this.read(scope),
    edit: async value => {
      this.ensureAvailable(value.conversationId);
      const request = decodeAgentWritingQueueEditRequest(value), receipt = this.edits.get(request.requestId);
      if (this.handoffUsesOperation(request.requestId) || this.preparations.has(request.requestId) || this.cancellations.has(request.requestId) || this.compactions.has(request.requestId) || this.starts.has(request.requestId)) throw {code: "conflict"};
      if (receipt) {
        if (receipt.request !== JSON.stringify(request)) throw { code: "conflict" };
        return structuredClone(receipt.result);
      }
      const failure = this.nextEditFailure;
      if (failure !== "unavailable") this.nextEditFailure = null;
      if (failure === "rejected") throw { code: "conflict" };
      if (failure === "beforeCommit") throw new Error("Simulated queue transport failure before commit");
      if (failure === "unavailable") throw new Error("Simulated queue transport remains unavailable");
      const current = this.read(request);
      if (current.revision !== request.expectedRevision) throw { code: "conflict" };
      const edit = request.edit;
      if(((edit.type==="update"||edit.type==="remove") && this.handoffOwns(request.conversationId,edit.inputId))
        || edit.type==="restore" && this.handoffOwns(request.conversationId,edit.sourceInputId))throw {code:"conflict"};
      if (current.preparingInputId && (edit.type === "restore" || edit.type === "reorder" || edit.type === "resume"
        || edit.type === "resumeInput" || ((edit.type === "update" || edit.type === "remove" || edit.type === "pauseInput")
          && edit.inputId === current.preparingInputId))) throw { code: "conflict" };
      let inputs = [...current.inputs], paused = current.paused;
      switch (edit.type) {
        case "restore": {
          if (inputs.some(item => item.inputId === edit.inputId || item.inputId === edit.sourceInputId)) throw { code: "conflict" };
          const removed = [...this.edits.values()].some(value => {
            if (value.result.conversationId !== request.conversationId) return false;
            const prior = decodeAgentWritingQueueEditRequest(JSON.parse(value.request));
            return prior.edit.type === "remove" && prior.edit.inputId === edit.sourceInputId;
          });
          if (!removed) throw { code: "conflict" };
          const source = [...this.edits.values()].filter(value => value.result.conversationId === request.conversationId)
            .sort((a, b) => b.result.revision - a.result.revision)
            .flatMap(value => value.result.inputs).find(item => item.inputId === edit.sourceInputId);
          if (!source) throw { code: "conflict" };
          inputs.splice(Math.min(edit.index, inputs.length), 0, {...source, inputId: edit.inputId, text: edit.text, userInput: edit.userInput ?? []});
          break;
        }
        case "add": if (inputs.some(item => item.inputId === edit.inputId)) throw { code: "conflict" };
          inputs.push({ inputId: edit.inputId, text: edit.text, userInput:edit.userInput??[] }); break;
        case "update": if (!inputs.some(item => item.inputId === edit.inputId)) throw { code: "notFound" };
          inputs = inputs.map(item => item.inputId === edit.inputId ? { ...item, text: edit.text, userInput:edit.userInput??[] } : item); break;
        case "remove": inputs = inputs.filter(item => item.inputId !== edit.inputId); break;
        case "reorder": if (edit.inputIds.length !== inputs.length || new Set(edit.inputIds).size !== inputs.length
          || edit.inputIds.some(id => !inputs.some(item => item.inputId === id))) throw { code: "conflict" };
          inputs = edit.inputIds.map(id => inputs.find(item => item.inputId === id)!); break;
        case "pauseInput": case "resumeInput": {
          if (!inputs.some(item => item.inputId === edit.inputId)) throw { code: "conflict" };
          if (edit.type === "resumeInput" && paused) { inputs = inputs.map(item => ({...item, paused: true})); paused = false; }
          inputs = inputs.map(item => item.inputId === edit.inputId ? {...item, paused: edit.type === "pauseInput"} : item);
          break;
        }
        case "pause": paused = true; break;
        case "resume": paused = false; break;
      }
      const result = this.put(request, { ...current, revision: current.revision + 1, inputs, paused });
      this.edits.set(request.requestId, { request: JSON.stringify(request), result });
      if (failure === "afterCommit") throw new Error("Simulated lost queue receipt after commit");
      return structuredClone(result);
    },
  };
  readonly preview: AgentInputQueuePreviewGateway = {
    recover: async scope => {
      this.ensureAvailable(scope.conversationId); const preview = this.previews.get(key(scope)), queue = this.read(scope);
      return preview && queue.preparingInputId === preview.inputId ? structuredClone({...preview, queue}) : null;
    },
    prepare: async input => {
      const request = decodeAgentWritingQueuePreviewRequest(input), prior = this.preparations.get(request.submission.requestId);
      this.ensureAvailable(request.submission.conversationId);
      if (prior) {
        if (!sameAgentWritingQueuePreviewRequest(prior.intent, request) || prior.status.type !== "committed") return prepareConflict(request);
        return structuredClone(prior.status.result);
      }
      if (this.handoffUsesOperation(request.submission.requestId) || this.edits.has(request.submission.requestId) || this.cancellations.has(request.submission.requestId) || this.compactions.has(request.submission.requestId)) return prepareConflict(request);
      const pending = this.previews.get(key({conversationId: request.submission.conversationId, manuscriptId: request.submission.context.manuscriptId}));
      if (pending?.preview.requestId === request.submission.requestId || this.inspectFrozen?.({conversationId: request.submission.conversationId,
        manuscriptId: request.submission.context.manuscriptId, requestId: request.submission.requestId})) return prepareConflict(request);
      const fault = this.nextPrepareFailure; if (fault !== "unavailable") this.nextPrepareFailure = null;
      if (fault === "rejected") return prepareConflict(request);
      if (fault === "beforeCommit" || fault === "unavailable") throw new Error("AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT");
      return this.once(`prepare:${request.submission.requestId}`, request.submission.conversationId, request, async () => {
      const scope = { manuscriptId: request.submission.context.manuscriptId, conversationId: request.submission.conversationId };
      const current = this.read(scope);
      const head = current.inputs.find(input => !input.paused);
      if (current.revision !== request.expectedRevision || current.paused || current.preparingInputId || this.handoffOwns(current.conversationId,request.inputId)
        || head?.inputId !== request.inputId || head.text !== request.submission.text || JSON.stringify(head.userInput??[])!==JSON.stringify(request.submission.userInput??[])) return prepareConflict(request);
      const materialized = this.materialize ? await this.materialize(request.submission)
        : {preview: await this.submission.prepareWriting(request.submission), commit: (accept: () => void) => accept()};
      let result!: AgentWritingQueuePreviewResponse;
      materialized.commit(() => {
        if (this.read(scope).revision !== current.revision) return prepareConflict(request);
        const queue = decodeAgentWritingQueueResponse({...current, revision: current.revision + 1, preparingInputId: request.inputId});
        result = decodeAgentWritingQueuePreview({schemaVersion: 1, inputId: request.inputId, preview: materialized.preview, queue});
        const receipt: AgentWritingQueuePrepareStatusResponse = {schemaVersion: 1, intent: request, status: {type: "committed", result,
          proof: {schemaVersion: 1, operationId: request.submission.requestId, conversationId: scope.conversationId,
            family: "agent.queue.preview.prepare", epoch: crypto.randomUUID(), sqlFence: String(queue.revision), replayed: false, result: null}}};
        this.put(scope, queue); this.previews.set(key(scope), result); this.preparations.set(request.submission.requestId, receipt);
      });
      if (fault === "afterCommit") throw new Error("AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST");
      return structuredClone(result);
    });},
    prepareStatus: async value => {
      const intent = decodeAgentWritingQueuePreviewRequest(value); this.ensureAvailable(intent.submission.conversationId);
      const prior = this.preparations.get(intent.submission.requestId);
      if (prior) {if (!sameAgentWritingQueuePreviewRequest(prior.intent, intent)) return prepareConflict(intent); return structuredClone(prior);}
      if (this.handoffUsesOperation(intent.submission.requestId) || this.edits.has(intent.submission.requestId) || this.cancellations.has(intent.submission.requestId) || this.compactions.has(intent.submission.requestId)) return prepareConflict(intent);
      const pending = this.previews.get(key({conversationId: intent.submission.conversationId, manuscriptId: intent.submission.context.manuscriptId}));
      if (pending?.preview.requestId === intent.submission.requestId || this.inspectFrozen?.({conversationId: intent.submission.conversationId,
        manuscriptId: intent.submission.context.manuscriptId, requestId: intent.submission.requestId})) return {schemaVersion: 1, intent, status: {type: "unconfirmed", detail: {
        operationId: intent.submission.requestId, epoch: null, sqlFence: null, phase: "queue-prepare", sqliteCode: null, sqliteExtendedCode: null, safeCause: "legacy_prepare_without_intent_proof",
      }}};
      return {schemaVersion: 1, intent, status: this.nextPrepareFailure === "unavailable" ? {type: "unconfirmed", detail: {
        operationId: intent.submission.requestId, epoch: null, sqlFence: null, phase: "prepare", sqliteCode: null, sqliteExtendedCode: null, safeCause: "mock_owner_unavailable",
      }} : {type: "notFound"}};
    },
    cancel: async input => {
      const request = decodeAgentWritingQueueCancelRequest(input), previous = this.cancellations.get(request.requestId);
      if (previous) {
        if (JSON.stringify(previous.intent) !== JSON.stringify(request) || previous.status.type !== "committed") return cancelConflict(request);
        return structuredClone(previous.status.queue);
      }
      if (this.handoffUsesOperation(request.requestId) || this.edits.has(request.requestId) || this.preparations.has(request.requestId) || this.starts.has(request.requestId) || this.receipts.has(`prepare:${request.requestId}`) || this.receipts.has(`start:${request.requestId}`)) return cancelConflict(request);
      const fault = this.nextCancelFailure; if (fault !== "unavailable") this.nextCancelFailure = null;
      if (fault === "rejected") return cancelConflict(request);
      if (fault === "beforeCommit" || fault === "unavailable") throw new Error("Simulated cancellation transport unavailable");
      const current = this.read(request), preview = this.previews.get(key(request));
      const releaseCompact = this.compactCancellation?.(request);
      if (current.revision !== request.expectedRevision || (!releaseCompact && preview?.preview.requestId !== request.previewRequestId)
        || current.preparingInputId !== request.inputId) return cancelConflict(request);
      const queue = this.put(request, { ...current, revision: current.revision + 1, preparingInputId: null, paused: true });
      this.previews.delete(key(request));
      releaseCompact?.();
      this.cancellations.set(request.requestId, {schemaVersion: 1, intent: request, status: {type: "committed", queue,
        proof: {schemaVersion: 1, operationId: request.requestId, conversationId: request.conversationId,
          family: "agent.queue.preview.cancel", epoch: crypto.randomUUID(), sqlFence: String(queue.revision), replayed: false, result: null}}});
      if (fault === "afterCommit") throw new Error("Simulated lost cancellation acknowledgement");
      return queue;
    },
    cancelStatus: async input => {
      const intent = decodeAgentWritingQueueCancelRequest(input); this.ensureAvailable(intent.conversationId);
      const previous = this.cancellations.get(intent.requestId);
      if (previous) {if (JSON.stringify(previous.intent) !== JSON.stringify(intent)) return cancelConflict(intent); return structuredClone(previous);}
      if (this.handoffUsesOperation(intent.requestId) || this.edits.has(intent.requestId) || this.preparations.has(intent.requestId) || this.starts.has(intent.requestId) || this.receipts.has(`prepare:${intent.requestId}`) || this.receipts.has(`start:${intent.requestId}`)) return cancelConflict(intent);
      return {schemaVersion: 1, intent, status: this.nextCancelFailure === "unavailable" ? {type: "unconfirmed", detail: {
        operationId: intent.requestId, epoch: null, sqlFence: null, phase: "cancel", sqliteCode: null, sqliteExtendedCode: null, safeCause: "mock_owner_unavailable",
      }} : {type: "notFound"}};
    },
    start: input => {
      const request = decodeAgentWritingQueuedStartRequest(input);
      return this.once(`start:${request.start.requestId}`, request.start.conversationId, request, () => this.start(request), "pending");
    },
    startStatus: input => this.startStatus(decodeAgentWritingQueuedStartRequest(input)),
  };
  private unconfirmed(intent: AgentWritingQueuedStartRequest, safeCause = "mock_start_reader_unavailable"): AgentWritingQueuedStartStatusResponse {
    return {schemaVersion: 1, intent, status: {type: "unconfirmed", detail: {operationId: intent.start.requestId,
      epoch: null, sqlFence: null, phase: "queue-start-status", sqliteCode: null, sqliteExtendedCode: null, safeCause}}};
  }
  private matchingPreview(request: AgentWritingQueuedStartRequest, preview: AgentSubmissionPreview): void {
    decodeAgentWritingQueuedStartStatusResponse({schemaVersion: 1, intent: request, status: {type: "prepared", preview}});
  }
  private async startStatus(intent: AgentWritingQueuedStartRequest): Promise<AgentWritingQueuedStartStatusResponse> {
    this.ensureAvailable(intent.start.conversationId);
    if (this.startReaderUnavailable) return this.unconfirmed(intent);
    const entries = [...this.starts.values()].filter(entry => entry.intent.inputId === intent.inputId
      || (entry.intent.start.conversationId === intent.start.conversationId && entry.intent.start.requestId === intent.start.requestId));
    const pending = this.previews.get(key(intent.start)), prepared = this.preparations.get(intent.start.requestId);
    const pendingMatches = pending?.inputId === intent.inputId || pending?.preview.requestId === intent.start.requestId;
    let record: AgentSubmissionPreview | null;
    try {record = this.inspectFrozen ? this.inspectFrozen(intent.start) : (pendingMatches ? pending?.preview : null) ?? entries[0]?.preview ?? null;}
    catch {return this.unconfirmed(intent, "queue_start_submission_invalid");}
    if (!record) {
      let evidence = !!(entries.length || pendingMatches || prepared);
      try {evidence ||= !!await this.startIo?.inspect(intent.start);} catch {return this.unconfirmed(intent, "queue_start_history_invalid");}
      return evidence ? this.unconfirmed(intent, "queue_start_submission_missing") : {schemaVersion: 1, intent, status: {type: "notFound"}};
    }
    try {this.matchingPreview(intent, record);} catch {return this.unconfirmed(intent, "queue_start_intent_mismatch");}
    if (this.compactions.has(intent.start.requestId)) return this.unconfirmed(intent, "queue_start_purpose_mismatch");
    if (!entries.length && prepared?.intent.inputId !== intent.inputId && pending?.inputId !== intent.inputId) return this.unconfirmed(intent, "queue_start_origin_mismatch");
    let started: AgentWritingStartResponse | null | undefined;
    try {started = await this.startIo?.inspect(intent.start, record);} catch {return this.unconfirmed(intent, "queue_start_history_invalid");}
    if (entries.length > 1) return this.unconfirmed(intent, "queue_start_admission_ambiguous");
    const admission = entries[0];
    if (admission) {
      if (admission.intent.inputId !== intent.inputId || admission.intent.expectedRevision !== intent.expectedRevision
        || admission.intent.start.conversationId !== intent.start.conversationId || admission.intent.start.requestId !== intent.start.requestId)
        return this.unconfirmed(intent, "queue_start_admission_mismatch");
      if (started) return {schemaVersion: 1, intent, status: {type: "started", result: started}};
      if (executionModeForModel(record.input.model, record.input.reasoningEffort) !== intent.start.mode) return this.unconfirmed(intent, "queue_start_mode_mismatch");
      return {schemaVersion: 1, intent, status: {type: "admitted", preview: structuredClone(record)}};
    }
    if (started) return this.unconfirmed(intent, "queue_start_admission_missing");
    if (executionModeForModel(record.input.model, record.input.reasoningEffort) !== intent.start.mode) return this.unconfirmed(intent, "queue_start_mode_mismatch");
    if (this.nextStartFailure === "unavailable") return this.unconfirmed(intent);
    const queue = this.read(intent.start), original = prepared?.status.type === "committed" ? prepared.status.result : null;
    const head = queue.inputs.find(input => input.inputId === intent.inputId), source = original?.queue.inputs.find(input => input.inputId === intent.inputId);
    if (pending?.inputId !== intent.inputId || queue.preparingInputId !== intent.inputId || pending.preview.requestId !== intent.start.requestId)
      return this.unconfirmed(intent, "queue_start_original_claim_absent");
    if (!original || original.queue.revision > intent.expectedRevision || intent.expectedRevision > queue.revision || JSON.stringify(head) !== JSON.stringify(source))
      return this.unconfirmed(intent, "queue_start_legacy_preparation_unconfirmed");
    return {schemaVersion: 1, intent, status: {type: "prepared", preview: structuredClone(record)}};
  }
  private async start(request: AgentWritingQueuedStartRequest): Promise<AgentWritingStartResponse> {
    const previous = this.starts.get(request.start.requestId);
    if (previous && !sameAgentWritingQueuedStartRequest(previous.intent, request)) throw {code: "conflict"};
    const record = this.inspectFrozen ? this.inspectFrozen(request.start) : previous?.preview ?? this.previews.get(key(request.start))?.preview;
    if (this.inspectFrozen && !record) throw {code: "recoveryRequired"};
    if (record) this.matchingPreview(request, record);
    const started = await this.startIo?.inspect(request.start, record ?? undefined);
    if (started) return started;
    const failure = this.nextStartFailure; if (failure !== "unavailable") this.nextStartFailure = null;
    if (failure === "beforeAdmission") throw "AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT";
    if (failure === "unavailable") throw {code: "unavailable"};
    this.startIo?.validate(request.start);
    if (!previous) {
      const current = this.read(request.start), preview = this.previews.get(key(request.start));
      if (current.revision !== request.expectedRevision || current.preparingInputId !== request.inputId || preview?.inputId !== request.inputId
        || preview.preview.requestId !== request.start.requestId || [...this.starts.values()].some(entry => entry.intent.inputId === request.inputId)) throw {code: "preparation", reason: "conflict"};
      this.matchingPreview(request, preview.preview);
      const queue = decodeAgentWritingQueueResponse({...current, revision: current.revision + (current.paused ? 2 : 1), paused: false,
        preparingInputId: null, inputs: current.inputs.filter(input => input.inputId !== request.inputId)});
      const admission = {intent: structuredClone(request), preview: structuredClone(preview.preview)};
      this.put(request.start, queue); this.starts.set(request.start.requestId, admission); this.previews.delete(key(request.start));
    }
    if (failure === "afterAdmission") throw "AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST";
    // Replaying an admitted request must not lift a user's later Pause.
    if (this.read(request.start).paused) throw {code: "recoveryRequired"};
    const result = await this.submission.startWriting(request.start);
    if (failure === "afterStarted") throw "AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST";
    return result;
  }
}
