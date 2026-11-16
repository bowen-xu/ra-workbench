import {decodeAgentPaperSubmissionPrepareRequest, decodeAgentPaperPrepareResult, decodeAgentPaperPrepareStatusResponse,
  sameAgentPaperPrepareRequest, type AgentPaperSubmissionPrepareRequest, type AgentPaperSubmissionPreview,
  decodeAgentPaperStartRequest, decodeAgentPaperStartStatusResponse, decodeAgentStartResponse, sameAgentPaperStartRequest,
  decodeAgentPaperQueueEditRequest, decodeAgentQueueEditProof, decodeAgentQueueEditStatusResponse, type AgentQueueEditProof,
  type AgentPaperQueueEditRequest, type AgentPaperQueueScope, type AgentPaperQueueResponse,
  decodeAgentQueueState, type AgentQueueState, type AgentPaperPrepareProof, type AgentPaperPrepareStatusResponse, type AgentPaperStartRequest, type AgentPaperStartStatusResponse, type AgentStartResponse} from "@research-assistant/core/agent";
import type {AgentInputQueueController, AgentQueueEditInspection, AgentPaperPrepareProvenGateway, AgentPaperPrepareStatusGateway, AgentPaperSubmissionGateway, AgentPaperStartStatusGateway} from "@research-assistant/data/agent";
import {flushStorage} from "../../../../../../apps/web/src/core/public/storage";
import { decodeConversationDraftAcknowledgmentToken, type ConversationDraftAcknowledgmentToken } from "../../../../../../apps/web/src/core/public/agentSubmission/conversationDraftStore";

export interface PaperPreparationContext {
  readonly sourceCount: number; readonly imageCount: number; readonly queuedInputId: string | null;
  readonly draftAcknowledgment: ConversationDraftAcknowledgmentToken | null;
  readonly queueInput?: AgentQueueState["inputs"][number];
  readonly queueRevision?: number;
}
export interface PaperPreparationSnapshot {
  readonly phase: "idle" | "saving" | "sending" | "unknown" | "checking" | "unconfirmed" | "ready" | "started" | "clearing" | "complete" | "localError";
  readonly intent: AgentPaperSubmissionPrepareRequest | null; readonly context: PaperPreparationContext | null;
  readonly result: AgentPaperSubmissionPreview | null; readonly status: AgentPaperPrepareStatusResponse["status"] | null;
  readonly recovered: boolean; readonly cause: unknown;
  readonly start: {readonly request: AgentPaperStartRequest; readonly result: AgentStartResponse | null} | null;
  readonly startStatus: AgentPaperStartStatusResponse["status"] | null;
  readonly queueRemoval: {readonly request: AgentPaperQueueEditRequest; readonly proof: AgentQueueEditProof | null} | null;
  readonly prepareProof: AgentPaperPrepareProof | null; readonly requireProof: boolean;
  readonly prepareEpoch: string | null; readonly guarded: boolean;
}
const initial = (): PaperPreparationSnapshot => ({phase: "idle", intent: null, context: null, result: null, status: null, recovered: false, cause: null, start: null, startStatus: null, queueRemoval: null, prepareProof: null, requireProof: false, prepareEpoch: null, guarded: false});
const key = (paper: string, conversation: string) => `research-assistant:paper-prepare:v1:${encodeURIComponent(paper)}:${conversation.toLowerCase()}`;
const uuid = (value: unknown): value is string => typeof value === "string" && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu.test(value) && value !== "00000000-0000-0000-0000-000000000000";
function context(value: unknown, intent: AgentPaperSubmissionPrepareRequest): PaperPreparationContext {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("PAPER_PREPARE_RECOVERY_INVALID");
  const row = value as Record<string, unknown>;
  const hasQueueInput = Object.hasOwn(row, "queueInput");
  if (Object.keys(row).sort().join() !== (hasQueueInput ? "draftAcknowledgment,imageCount,queueInput,queueRevision,queuedInputId,sourceCount" : "draftAcknowledgment,imageCount,queuedInputId,sourceCount")
    || !Number.isSafeInteger(row.sourceCount) || (row.sourceCount as number) < 0 || !Number.isSafeInteger(row.imageCount)
    || (row.imageCount as number) < 0 || (row.imageCount as number) > (row.sourceCount as number)
    || (row.queuedInputId !== null && !uuid(row.queuedInputId))) throw new Error("PAPER_PREPARE_RECOVERY_INVALID");
  const acknowledgment = row.draftAcknowledgment === null ? null : decodeConversationDraftAcknowledgmentToken(row.draftAcknowledgment);
  if (acknowledgment) {
    const [kind, paper, conversation] = JSON.parse(acknowledgment.originKey) as [string, string, string | null];
    if (kind !== "paper" || paper !== intent.paperId || (conversation !== null && conversation !== intent.conversationId)) throw new Error("PAPER_PREPARE_RECOVERY_INVALID");
  }
  const queuedInputId = typeof row.queuedInputId === "string" ? row.queuedInputId.toLowerCase() : null;
  const queueInput = hasQueueInput ? decodeAgentQueueState({schemaVersion: 1, conversationId: intent.conversationId,
    revision: 0, paused: false, preparingInputId: null, inputs: [row.queueInput]}).inputs[0] : undefined;
  if (hasQueueInput && (!Number.isSafeInteger(row.queueRevision) || (row.queueRevision as number) < 0
    || !queueInput || !queuedInputId || queueInput.inputId !== queuedInputId || queueInput.paused
    || !sameAgentPaperPrepareRequest({...intent, text: queueInput.text, userInput: queueInput.userInput ?? []}, intent))) throw new Error("PAPER_PREPARE_RECOVERY_INVALID");
  return {sourceCount: row.sourceCount as number, imageCount: row.imageCount as number,
    queuedInputId, draftAcknowledgment: acknowledgment, ...(queueInput ? {queueInput, queueRevision: row.queueRevision as number} : {})};
}

/** Local intent is a recovery locator, not permission to repeat or start a turn. */
export class PaperPreparation {
  private state = initial(); private disposed = false; private reserved = false; private pending: Promise<void> | null = null;
  private finishing = false; private clearing = false;
  private startPort: {submission: Pick<AgentPaperSubmissionGateway, "start">; status: AgentPaperStartStatusGateway} | null = null;
  private queuePort: AgentInputQueueController<AgentPaperQueueScope, AgentPaperQueueResponse> | null = null;
  private queueInspection: AgentQueueEditInspection<AgentPaperQueueEditRequest> | null = null;
  private draftPort: {flush(): Promise<void>; consumeAcknowledgment(token: ConversationDraftAcknowledgmentToken, scope: {hostKind: "paper"; hostId: string; conversationId: string}): Promise<unknown>} | null = null;
  constructor(private readonly prepare: Pick<AgentPaperSubmissionGateway, "prepare">,
    private readonly statusGateway: AgentPaperPrepareStatusGateway, private readonly storage: Storage,
    private readonly changed: (state: PaperPreparationSnapshot) => void, private readonly scope: {paperId: string; conversationId: string},
    private readonly proven?: AgentPaperPrepareProvenGateway) {this.restore();}
  getSnapshot = () => this.state;
  unresolved = () => this.state.phase !== "idle" && this.state.phase !== "complete";
  canRetryFinish = () => this.finishing;
  configureStart(submission: Pick<AgentPaperSubmissionGateway, "start">, status: AgentPaperStartStatusGateway): void {this.startPort = {submission, status};}
  configureQueue(port: AgentInputQueueController<AgentPaperQueueScope, AgentPaperQueueResponse>, inspection: AgentQueueEditInspection<AgentPaperQueueEditRequest>): void {this.queuePort = port; this.queueInspection = inspection;}
  configureDraft(port: NonNullable<PaperPreparation["draftPort"]>): void {this.draftPort = port;}
  private async flushDraft(): Promise<void> {
    if (!this.state.context?.draftAcknowledgment) return;
    if (!this.draftPort) throw new Error("PAPER_PREPARE_DRAFT_ACK_UNAVAILABLE");
    await this.draftPort.flush();
  }
  private validateStart(request: AgentPaperStartRequest, preview: AgentPaperSubmissionPreview): void {
    if (request.conversationId !== preview.conversationId || request.requestId !== preview.requestId || request.paperId !== preview.paperId
      || request.paperContextSnapshotId !== preview.paperContextSnapshotId || request.contextSnapshotId !== preview.contextSnapshotId
      || JSON.stringify(request.model) !== JSON.stringify(preview.input.model)) throw new Error("PAPER_START_MISMATCH");
  }
  private validateStarted(result: AgentStartResponse, request: AgentPaperStartRequest, preview: AgentPaperSubmissionPreview): void {
    if (result.conversationId !== request.conversationId || result.requestId !== request.requestId || result.turnId !== preview.turnId) throw new Error("PAPER_START_MISMATCH");
  }
  private async verifyOriginalProof(): Promise<void> {
    if (!this.state.requireProof) return;
    const intent = this.state.intent, proof = this.state.prepareProof;
    if (!intent || !proof || !this.state.result) throw new Error("PAPER_PREPARE_ORIGINAL_PROOF_UNCONFIRMED");
    const response = decodeAgentPaperPrepareStatusResponse(await this.statusGateway.read(structuredClone(intent), this.state.guarded && this.state.prepareEpoch ? {expectedEpoch: this.state.prepareEpoch} : undefined));
    if (!sameAgentPaperPrepareRequest(response.intent, intent) || response.status.type !== "committed"
      || response.status.proof.epoch !== proof.epoch || response.status.proof.sqlFence !== proof.sqlFence
      || JSON.stringify(response.status.result) !== JSON.stringify(this.state.result)) throw new Error("PAPER_PREPARE_ORIGINAL_PROOF_UNCONFIRMED");
  }
  private update(value: Partial<PaperPreparationSnapshot>): void {if (!this.disposed) {this.state = {...this.state, ...value}; this.changed(this.state);}}
  private read(): {intent: AgentPaperSubmissionPrepareRequest; context: PaperPreparationContext; result: AgentPaperSubmissionPreview | null; start: PaperPreparationSnapshot["start"]; queueRemoval: PaperPreparationSnapshot["queueRemoval"]; prepareProof: AgentPaperPrepareProof | null; requireProof: boolean; prepareEpoch: string | null; guarded: boolean} | null {
    const raw = this.storage.getItem(key(this.scope.paperId, this.scope.conversationId)); if (raw === null) return null;
    try {
      const value: unknown = JSON.parse(raw);
      if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error();
      const row = value as Record<string, unknown>;
      if (row.schemaVersion === 1 ? Object.keys(row).sort().join() !== "context,intent,result,schemaVersion"
        : row.schemaVersion === 2 ? Object.keys(row).sort().join() !== "context,intent,result,schemaVersion,start"
        : row.schemaVersion === 5 ? Object.keys(row).sort().join() !== "context,intent,prepareProof,queueRemoval,result,schemaVersion,start"
        : row.schemaVersion === 6 ? Object.keys(row).sort().join() !== "context,intent,prepareEpoch,prepareProof,queueRemoval,result,schemaVersion,start"
        : (row.schemaVersion !== 3 && row.schemaVersion !== 4) || Object.keys(row).sort().join() !== "context,intent,queueRemoval,result,schemaVersion,start") throw new Error();
      const intent = decodeAgentPaperSubmissionPrepareRequest(row.intent);
      if (intent.paperId !== this.scope.paperId || intent.conversationId !== this.scope.conversationId.toLowerCase()) throw new Error();
      const result = row.result === null ? null : decodeAgentPaperPrepareResult(row.result, intent);
      let start: PaperPreparationSnapshot["start"] = null;
      if (row.schemaVersion !== 1 && row.start !== null) {
        if (!row.start || typeof row.start !== "object" || Array.isArray(row.start) || Object.keys(row.start).sort().join() !== "request,result" || !result) throw new Error();
        const raw = row.start as Record<string, unknown>, request = decodeAgentPaperStartRequest(raw.request);
        this.validateStart(request, result);
        const started = raw.result === null ? null : decodeAgentStartResponse(raw.result);
        if (started) this.validateStarted(started, request, result);
        start = {request, result: started};
      }
      const savedContext = context(row.context, intent);
      if (row.schemaVersion !== 5 && row.schemaVersion !== 6 && (row.schemaVersion === 4) !== (savedContext.queueInput !== undefined)) throw new Error();
      let prepareProof: AgentPaperPrepareProof | null = null;
      if ((row.schemaVersion === 5 || row.schemaVersion === 6) && row.prepareProof !== null) {
        if (!result) throw new Error();
        const decoded = decodeAgentPaperPrepareStatusResponse({schemaVersion: 1, intent, status: {type: "committed", proof: row.prepareProof, result}});
        if (decoded.status.type !== "committed") throw new Error();
        prepareProof = decoded.status.proof;
      }
      if ((row.schemaVersion === 5 || row.schemaVersion === 6) && result && !prepareProof) throw new Error();
      const prepareEpoch = row.schemaVersion === 6 ? row.prepareEpoch : null;
      if (prepareEpoch !== null && !uuid(prepareEpoch) || row.schemaVersion === 6 && prepareProof && prepareProof.epoch !== prepareEpoch) throw new Error();
      let queueRemoval: PaperPreparationSnapshot["queueRemoval"] = null;
      if ((row.schemaVersion === 3 || row.schemaVersion === 4 || row.schemaVersion === 5 || row.schemaVersion === 6) && row.queueRemoval !== null) {
        if (!row.queueRemoval || typeof row.queueRemoval !== "object" || Array.isArray(row.queueRemoval)
          || Object.keys(row.queueRemoval).sort().join() !== "proof,request" || !start?.result || !savedContext.queuedInputId) throw new Error();
        const raw = row.queueRemoval as Record<string, unknown>, request = decodeAgentPaperQueueEditRequest(raw.request);
        const proof = raw.proof === null ? null : decodeAgentQueueEditProof(raw.proof);
        if (proof && (proof.operationId !== request.requestId || proof.conversationId !== request.conversationId
          || request.expectedEpoch !== undefined && proof.epoch !== request.expectedEpoch)
          || request.paperId !== intent.paperId || request.conversationId !== intent.conversationId
          || request.edit.type !== "remove" || request.edit.inputId !== savedContext.queuedInputId) throw new Error();
        queueRemoval = {request, proof};
      }
      return {intent, context: savedContext, result, start, queueRemoval, prepareProof, requireProof: row.schemaVersion === 5 || row.schemaVersion === 6, prepareEpoch: prepareEpoch as string | null, guarded: row.schemaVersion === 6};
    } catch {throw new Error("PAPER_PREPARE_RECOVERY_INVALID");}
  }
  private restore(): void {
    try {const saved = this.read(); if (saved) {this.reserved = true; this.update({...saved, phase: "unknown", recovered: true});}}
    catch (cause: unknown) {this.update({phase: "localError", recovered: true, cause});}
  }
  private async persist(): Promise<void> {
    const {intent, context: current, result, start, queueRemoval, prepareProof, requireProof, prepareEpoch, guarded} = this.state; if (!intent || !current) throw new Error("PAPER_PREPARE_RECOVERY_INVALID");
    const saved = this.read();
    if ((this.reserved && !saved) || saved && (!sameAgentPaperPrepareRequest(saved.intent, intent) || JSON.stringify(saved.context) !== JSON.stringify(current)
      || saved.requireProof !== requireProof || saved.guarded !== guarded || saved.prepareEpoch && saved.prepareEpoch !== prepareEpoch || saved.prepareProof && JSON.stringify(saved.prepareProof) !== JSON.stringify(prepareProof)
      || saved.result && (!result || JSON.stringify(saved.result) !== JSON.stringify(result))
      || saved.start && (!start || !sameAgentPaperStartRequest(saved.start.request, start.request)
        || saved.start.result && JSON.stringify(saved.start.result) !== JSON.stringify(start.result))
      || saved.queueRemoval && (!queueRemoval || JSON.stringify(saved.queueRemoval.request) !== JSON.stringify(queueRemoval.request)
        || saved.queueRemoval.proof && JSON.stringify(saved.queueRemoval.proof) !== JSON.stringify(queueRemoval.proof)))) throw new Error("PAPER_PREPARE_RECOVERY_CONFLICT");
    this.storage.setItem(key(intent.paperId, intent.conversationId), JSON.stringify({schemaVersion: guarded ? 6 : requireProof ? 5 : current.queueInput ? 4 : 3, intent, context: current, result, start, queueRemoval, ...(requireProof ? {prepareProof} : {}), ...(guarded ? {prepareEpoch} : {})}));
    this.reserved = true; await flushStorage(this.storage);
  }
  private async retained(run: () => Promise<void>): Promise<void> {
    if (this.disposed) return; if (this.pending) return this.pending;
    const pending = Promise.resolve().then(run); this.pending = pending; try {await pending;} finally {if (this.pending === pending) this.pending = null;}
  }
  begin(value: AgentPaperSubmissionPrepareRequest, data: PaperPreparationContext): Promise<void> {
    return this.retained(async () => {
      if (this.unresolved()) return;
      if (this.state.phase === "complete") this.update(initial());
      const intent = decodeAgentPaperSubmissionPrepareRequest(value), saved = context(data, intent);
      if (intent.paperId !== this.scope.paperId || intent.conversationId !== this.scope.conversationId.toLowerCase()) throw new Error("PAPER_PREPARE_RECOVERY_INVALID");
      this.update({phase: "saving", intent, context: saved, requireProof: !!this.proven, guarded: !!this.proven});
      try {await this.persist(); await this.flushDraft();} catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
      if (this.proven) {
        try {
          const observed = decodeAgentPaperPrepareStatusResponse(await this.statusGateway.read(structuredClone(intent)));
          if (!sameAgentPaperPrepareRequest(observed.intent, intent) || observed.status.type !== "unconfirmed" || !uuid(observed.status.detail.epoch)) throw new Error("PAPER_PREPARE_ORIGINAL_PROOF_UNCONFIRMED");
          if (this.disposed) return;
          this.update({prepareEpoch: observed.status.detail.epoch.toLowerCase()});
          await this.persist();
        } catch (cause: unknown) {this.update({phase: "unknown", cause}); return;}
      }
      if (this.disposed) return; this.update({phase: "sending"});
      let result: AgentPaperSubmissionPreview;
      let proof: AgentPaperPrepareProof | null = null;
      try {
        if (this.proven) {
          const committed = await this.proven.prepare(structuredClone(intent), {expectedEpoch: this.state.prepareEpoch!});
          const decoded = decodeAgentPaperPrepareStatusResponse({schemaVersion: 1, intent, status: committed});
          if (decoded.status.type !== "committed" || decoded.status.proof.epoch !== this.state.prepareEpoch) throw new Error("PAPER_PREPARE_PROOF_REQUIRED");
          result = decoded.status.result; proof = decoded.status.proof;
        } else result = decodeAgentPaperPrepareResult(await this.prepare.prepare(structuredClone(intent)), intent);
      }
      catch (cause: unknown) {this.update({phase: "unknown", cause}); return;}
      if (this.disposed) return; this.update({phase: "saving", result, prepareProof: proof});
      try {await this.persist(); this.update({phase: "ready", cause: null});} catch (cause: unknown) {this.update({phase: "localError", cause});}
    });
  }
  check = (): Promise<void> => this.retained(async () => {
    if (this.state.start) {await this.inspectStart(); return;}
    const intent = this.state.intent; if (!intent || this.disposed) return;
    try {await this.persist();} catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return; this.update({phase: "checking", cause: null});
    try {
      if (this.state.guarded && !this.state.prepareEpoch) throw new Error("PAPER_PREPARE_ORIGINAL_PROOF_UNCONFIRMED");
      const response = decodeAgentPaperPrepareStatusResponse(await this.statusGateway.read(structuredClone(intent), this.state.guarded ? {expectedEpoch: this.state.prepareEpoch!} : undefined));
      if (!sameAgentPaperPrepareRequest(response.intent, intent)) throw new Error("PAPER_PREPARE_STATUS_MISMATCH");
      if (this.disposed) return;
      if (response.status.type === "unconfirmed") {this.update({phase: "unconfirmed", status: response.status}); return;}
      if (this.state.requireProof && (this.state.prepareProof ? this.state.prepareProof.epoch !== response.status.proof.epoch
        || this.state.prepareProof.sqlFence !== response.status.proof.sqlFence : !this.state.guarded || this.state.prepareEpoch !== response.status.proof.epoch)) throw new Error("PAPER_PREPARE_ORIGINAL_PROOF_UNCONFIRMED");
      if (this.state.result && JSON.stringify(this.state.result) !== JSON.stringify(response.status.result)) throw new Error("PAPER_PREPARE_RESULT_MISMATCH");
      this.update({phase: "saving", status: response.status, result: response.status.result, ...(this.state.requireProof && !this.state.prepareProof ? {prepareProof: response.status.proof} : {})});
      try {await this.persist(); this.update({phase: "ready"});} catch (cause: unknown) {this.update({phase: "localError", cause});}
    } catch (cause: unknown) {this.update({phase: "unknown", cause});}
  });
  confirmStart(value: AgentPaperStartRequest): Promise<void> {
    return this.retained(async () => {
      if (this.state.start) {await this.inspectStart(); return;}
      const preview = this.state.result, port = this.startPort;
      if (this.state.phase !== "ready" || !preview || !port) return;
      try {await this.verifyOriginalProof();} catch (cause: unknown) {this.update({phase: "unknown", cause}); return;}
      const request = decodeAgentPaperStartRequest(value); this.validateStart(request, preview);
      this.update({phase: "saving", start: {request, result: null}, startStatus: null, cause: null});
      try {await this.persist(); await this.flushDraft();} catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
      if (this.disposed) return; this.update({phase: "sending"});
      let result: AgentStartResponse;
      try {result = decodeAgentStartResponse(await port.submission.start(structuredClone(preview), request.mode, this.state.guarded && this.state.prepareEpoch ? {expectedEpoch: this.state.prepareEpoch} : undefined)); this.validateStarted(result, request, preview);}
      catch (cause: unknown) {this.update({phase: "unknown", cause}); return;}
      if (this.disposed) return;
      this.update({phase: "saving", start: {request, result}});
      try {await this.persist(); this.update({phase: "started", cause: null});} catch (cause: unknown) {this.update({phase: "localError", cause});}
    });
  }
  private async inspectStart(): Promise<void> {
    const start = this.state.start, preview = this.state.result, port = this.startPort;
    if (!start || !preview || !port || this.disposed) return;
    try {await this.verifyOriginalProof();} catch (cause: unknown) {this.update({phase: "unknown", cause}); return;}
    try {await this.persist();} catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return; this.update({phase: "checking", cause: null});
    try {
      const response = decodeAgentPaperStartStatusResponse(await port.status.read(structuredClone(start.request), this.state.guarded && this.state.prepareEpoch ? {expectedEpoch: this.state.prepareEpoch} : undefined));
      if (!sameAgentPaperStartRequest(response.intent, start.request)) throw new Error("PAPER_START_MISMATCH");
      if (this.disposed) return;
      if (response.status.type === "unconfirmed") {this.update({phase: "unconfirmed", startStatus: response.status}); return;}
      this.validateStarted(response.status.result, start.request, preview);
      if (start.result && (start.result.conversationId !== response.status.result.conversationId || start.result.requestId !== response.status.result.requestId
        || start.result.turnId !== response.status.result.turnId)) throw new Error("PAPER_START_MISMATCH");
      // A later historical observation may add outcome and reports alreadyStarted=true.
      this.update({phase: "saving", start: {request: start.request, result: start.result ?? response.status.result}, startStatus: response.status});
      try {await this.persist(); this.update({phase: "started"});} catch (cause: unknown) {this.update({phase: "localError", cause});}
    } catch (cause: unknown) {this.update({phase: "unknown", cause});}
  }
  private async settleQueue(intent: AgentPaperSubmissionPrepareRequest, inputId: string): Promise<void> {
    const owner = this.queuePort, inspection = this.queueInspection;
    if (!owner || !inspection) throw new Error("PAPER_PREPARE_QUEUE_UNAVAILABLE");
    const prepared = decodeAgentPaperPrepareStatusResponse(await this.statusGateway.read(structuredClone(intent), this.state.guarded && this.state.prepareEpoch ? {expectedEpoch: this.state.prepareEpoch} : undefined));
    if (!sameAgentPaperPrepareRequest(prepared.intent, intent) || prepared.status.type !== "committed"
      || JSON.stringify(prepared.status.result) !== JSON.stringify(this.state.result)) throw new Error("PAPER_PREPARE_QUEUE_PREPARATION_UNCONFIRMED");
    if (this.disposed) throw new Error("PAPER_PREPARE_OWNER_DISPOSED");
    const epoch = prepared.status.proof.epoch;
    if (this.state.requireProof && (!this.state.prepareProof || this.state.prepareProof.epoch !== epoch
      || this.state.prepareProof.sqlFence !== prepared.status.proof.sqlFence)) throw new Error("PAPER_PREPARE_ORIGINAL_PROOF_UNCONFIRMED");
    if (this.state.queueRemoval?.proof) {
      if (this.state.queueRemoval.proof.epoch !== epoch || BigInt(this.state.queueRemoval.proof.sqlFence) < BigInt(prepared.status.proof.sqlFence))
        throw new Error("PAPER_PREPARE_QUEUE_EPOCH_MISMATCH");
      return;
    }
    const save = async (value: AgentPaperQueueEditRequest) => {
      if (this.disposed) throw new Error("PAPER_PREPARE_OWNER_DISPOSED");
      const request = decodeAgentPaperQueueEditRequest(value);
      if (request.conversationId !== intent.conversationId || request.paperId !== intent.paperId
        || request.edit.type !== "remove" || request.edit.inputId !== inputId || request.expectedEpoch !== epoch
        || this.state.queueRemoval && JSON.stringify(this.state.queueRemoval.request) !== JSON.stringify(request)) throw new Error("PAPER_PREPARE_QUEUE_MISMATCH");
      this.update({queueRemoval: {request, proof: null}}); await this.persist();
      if (this.disposed) throw new Error("PAPER_PREPARE_OWNER_DISPOSED");
    };
    const original = this.state.queueRemoval?.request;
    let confirmed: boolean;
    if (original) {
      if (original.expectedEpoch === undefined) {
        // Old ambiguous requests never carried a data-owner guard. Only a
        // committed original receipt can settle them; never replay the write.
        const legacy = decodeAgentQueueEditStatusResponse(await inspection.inspectEdit(structuredClone(original)), original);
        if (legacy.status.type !== "committed" || legacy.status.queue.inputs.some(input => input.inputId === inputId)
          || legacy.status.proof.epoch !== epoch || BigInt(legacy.status.proof.sqlFence) < BigInt(prepared.status.proof.sqlFence))
          throw new Error("PAPER_PREPARE_QUEUE_UNCONFIRMED");
        if (this.disposed) throw new Error("PAPER_PREPARE_OWNER_DISPOSED");
        this.update({queueRemoval: {request: original, proof: legacy.status.proof}}); await this.persist(); return;
      }
      if (original.expectedEpoch !== epoch) throw new Error("PAPER_PREPARE_QUEUE_EPOCH_MISMATCH");
      if (!owner.restorePending(original, undefined, save)) throw new Error("PAPER_PREPARE_QUEUE_BUSY");
      confirmed = await owner.retry();
    } else {
      const queue = owner.getSnapshot().queue, input = queue?.inputs.find(item => item.inputId === inputId);
      const frozen = this.state.context?.queueInput;
      // Legacy locators have no origin snapshot; keep their conservative rule.
      if (!input || input.paused || (frozen ? queue?.revision !== this.state.context?.queueRevision
        || input.origin?.kind !== frozen.origin?.kind || input.origin?.resourceId !== frozen.origin?.resourceId : !!input.origin)
        || !sameAgentPaperPrepareRequest({...intent, text: input.text, userInput: input.userInput ?? []}, intent))
        throw new Error("PAPER_PREPARE_QUEUE_INPUT_CHANGED");
      confirmed = await owner.edit({type: "remove", inputId}, undefined, save, epoch);
    }
    if (!confirmed || this.disposed || !this.state.queueRemoval) throw new Error("PAPER_PREPARE_QUEUE_UNCONFIRMED");
    const request = this.state.queueRemoval.request;
    const observed = decodeAgentQueueEditStatusResponse(await inspection.inspectEdit(structuredClone(request)), request);
    if (observed.status.type !== "committed" || observed.status.queue.inputs.some(input => input.inputId === inputId) || observed.status.proof.epoch !== epoch
      || BigInt(observed.status.proof.sqlFence) < BigInt(prepared.status.proof.sqlFence)) throw new Error("PAPER_PREPARE_QUEUE_UNCONFIRMED");
    if (this.disposed) throw new Error("PAPER_PREPARE_OWNER_DISPOSED");
    this.update({queueRemoval: {request, proof: observed.status.proof}}); await this.persist();
  }
  /** Finalize only a proven start and, for queued input, its proven removal. */
  finish = (): Promise<void> => this.retained(async () => {
    const {intent, context, start} = this.state;
    if (!intent || !context || !start?.result || (!this.finishing && this.state.phase !== "started")) return;
    if (context.queuedInputId && (!this.queuePort || !this.queueInspection)) return;
    this.finishing = true; this.update({phase: "clearing", cause: null});
    try {
      if (!this.clearing) {
        await this.verifyOriginalProof();
        await this.persist();
        if (this.disposed) return;
        if (context.queuedInputId) await this.settleQueue(intent, context.queuedInputId);
        if (this.disposed) return;
        if (context.draftAcknowledgment) {
          if (!this.draftPort) throw new Error("PAPER_PREPARE_DRAFT_ACK_UNAVAILABLE");
          await this.draftPort.consumeAcknowledgment(context.draftAcknowledgment, {hostKind: "paper", hostId: intent.paperId, conversationId: intent.conversationId});
        }
        if (this.disposed) return;
        await this.persist();
        if (this.disposed) return;
        const saved = this.read();
        if (!saved || !sameAgentPaperPrepareRequest(saved.intent, intent) || JSON.stringify(saved.context) !== JSON.stringify(context)
          || JSON.stringify(saved.result) !== JSON.stringify(this.state.result) || !saved.start
          || !sameAgentPaperStartRequest(saved.start.request, start.request) || JSON.stringify(saved.start.result) !== JSON.stringify(start.result)
          || JSON.stringify(saved.queueRemoval) !== JSON.stringify(this.state.queueRemoval)
          || saved.requireProof !== this.state.requireProof || saved.guarded !== this.state.guarded || saved.prepareEpoch !== this.state.prepareEpoch || JSON.stringify(saved.prepareProof) !== JSON.stringify(this.state.prepareProof))
          throw new Error("PAPER_PREPARE_RECOVERY_CONFLICT");
        this.storage.removeItem(key(intent.paperId, intent.conversationId)); this.clearing = true;
      }
      await flushStorage(this.storage);
      if (this.disposed) return;
      this.reserved = false; this.clearing = false; this.finishing = false; this.update({phase: "complete", cause: null});
    } catch (cause: unknown) {this.update({phase: "localError", cause});}
  });
  dispose(): void {this.disposed = true;}
}
