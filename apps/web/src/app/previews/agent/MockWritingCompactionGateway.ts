import {
  agentWritingCompactionStartRequest, decodeAgentCompactionStartRequest, decodeAgentInterruptResponse, decodeAgentWritingPrepareRequest,
  decodeAgentStartResponse, decodeAgentWritingQueuedCompactionPrepareRequest, decodeAgentWritingQueuedCompactionPreview,
  decodeAgentWritingQueuedCompactionReadRequest, decodeAgentWritingQueuedCompactionSettleRequest,
  decodeAgentWritingCompactionPrepareRequest, decodeAgentWritingPrepareResult, decodeAgentWritingCompactionPrepareStatusResponse,
  decodeAgentWritingCompactionStartStatusResponse,
  type AgentCompactionStartRequest, type AgentQueuedCompactionOutcome, type AgentWritingCompactionPrepareRequest,
  type AgentWritingCompactionPreview, type AgentWritingQueuedCompactionPrepareRequest,
  type AgentWritingQueuedCompactionPreview, type AgentWritingQueuedCompactionSettleResponse,
  type AgentInterruptRequest, type AgentWritingReceipt,
  type AgentWritingQueueResponse, type AgentWritingPrepareRequest, type AgentSubmissionPreview, type AgentWritingQueueCancelRequest,
} from "@research-assistant/core/agent";
import type { AgentWritingCompactionGateway } from "@research-assistant/data/agent";
import type { WritingDiffDemoHistory } from "./WritingDiffDemoHistory";
import type { WritingDiffDemoQueue, WritingDemoQueueFailure } from "./WritingDiffDemoQueue";
import type { MockAgentCommandGateway } from "../../../core/features/agent/MockAgentCommandGateway";
import type { WritingSubmissionAdmission } from "../../../core/features/agent/writingSubmissionController";
import type { AgentCommandRequest, AgentCommandUsage } from "../../../core/features/agent/agentCommandGateway";
import {mockWritingStartHasHistoricalManuscript, mockWritingStartUnconfirmed, observeMockWritingStart} from "../../../core/features/agent/MockWritingStartObservation";
import {mockWritingPreparationReceipt, mockWritingPreparationStatus, mockWritingPreparationUnconfirmed, requireMockWritingPreparation,
  type MockWritingPreparationReceipt, type MockWritingPreparationPurpose} from "../../../core/features/agent/MockWritingPreparation";

interface Record {
  readonly intent: string;
  readonly prepared: AgentWritingCompactionPreview;
  readonly queued: AgentWritingQueuedCompactionPrepareRequest | null;
  readonly claimQueue: AgentWritingQueueResponse | null;
  readonly itemId: string;
  readonly preparation?: MockWritingPreparationReceipt;
  started: boolean;
  outcome: AgentQueuedCompactionOutcome | null;
}
type Stage = "prepare" | "start" | "settle";
interface PreparationPort {
  materialize(request: AgentWritingPrepareRequest, purpose?: "manualCompact" | "queuedCompact"): Promise<{
    readonly preview: AgentSubmissionPreview;
    /** Synchronous acceptance checks run before the frozen input is published. */
    commit(accept: () => void): void;
  }>;
  inspect?(receipt: AgentWritingReceipt): {readonly preview: AgentSubmissionPreview; readonly purpose?: MockWritingPreparationPurpose} | null;
}
const key = (scope: { conversationId: string; requestId: string }) => JSON.stringify([scope.conversationId, scope.requestId]);
const inputKey = (conversationId: string, inputId: string) => JSON.stringify([conversationId, inputId]);
function canonicalIntent(value: string): string {
  const row = JSON.parse(value) as {request: AgentWritingCompactionPrepareRequest; queued: AgentWritingQueuedCompactionPrepareRequest | null};
  return JSON.stringify({request: {...row.request, request: decodeAgentWritingPrepareRequest(row.request.request)},
    queued: row.queued === null ? null : decodeAgentWritingQueuedCompactionPrepareRequest(row.queued)});
}

/** Mock native persistence and external compaction I/O. The production reviewed
 * submission owner, history reader and queue controllers still drive the UI. */
export class MockWritingCompactionGateway {
  private readonly records = new Map<string, Record>();
  private readonly current = new Map<string, string>();
  private readonly latest = new Map<string, string>();
  private readonly preparations = new Map<string, { intent: string; result: Promise<Record> }>();
  private readonly receipts = new Map<string, { intent: string; result: AgentWritingQueuedCompactionSettleResponse }>();
  private readonly running = new Map<string, AbortController>();
  private readonly usages = new Map<string, AgentCommandUsage>();
  private readonly faults = new Map<Stage, WritingDemoQueueFailure>();
  private disposed = false;
  private readonly prepareEpoch = crypto.randomUUID();
  private prepareReaderUnavailable = false;
  private startReaderUnavailable = false;
  nextOutcome: AgentQueuedCompactionOutcome = "completed";
  constructor(private readonly preparation: PreparationPort, private readonly history: WritingDiffDemoHistory,
    private readonly queue: WritingDiffDemoQueue, private readonly commands: MockAgentCommandGateway) {}

  failNext(stage: Stage, failure: WritingDemoQueueFailure | null): void {
    if (failure === null) this.faults.delete(stage); else this.faults.set(stage, failure);
  }
  failPrepareStatus(unavailable: boolean): void {this.prepareReaderUnavailable = unavailable;}
  failStartStatus(unavailable: boolean): void {this.startReaderUnavailable = unavailable;}
  hasPrepared(request: { conversationId: string; requestId: string }): boolean { return this.records.has(key(request)); }
  prepareCancellation(intent: AgentWritingQueueCancelRequest): (() => void) | null {
    const record = this.records.get(key({conversationId: intent.conversationId, requestId: intent.previewRequestId}));
    if (!record) return null;
    const identity = inputKey(intent.conversationId, intent.inputId);
    if (!record.queued || record.queued.inputId !== intent.inputId || record.prepared.preview.manuscriptId !== intent.manuscriptId
      || record.started || record.outcome || this.current.get(identity) !== intent.previewRequestId) throw {code: "conflict"};
    return () => {this.current.delete(identity);};
  }
  deleteConversation(conversationId: string): void {
    for (const [identity, record] of this.records) if (record.prepared.preview.conversationId === conversationId) this.records.delete(identity);
    for (const identity of this.current.keys()) if ((JSON.parse(identity) as string[])[0] === conversationId) this.current.delete(identity);
    for (const identity of this.latest.keys()) if ((JSON.parse(identity) as string[])[0] === conversationId) this.latest.delete(identity);
    for (const [identity, receipt] of this.receipts) if (receipt.result.queue.conversationId === conversationId) this.receipts.delete(identity);
    this.usages.delete(conversationId);
  }
  async readUsage(request: AgentCommandRequest): Promise<AgentCommandUsage> {
    this.available(request.conversationId);
    const fallback = await this.commands.readUsage(request);
    return this.usages.get(request.conversationId) ?? fallback;
  }
  snapshot() { return structuredClone({ records: this.records, current: this.current, latest: this.latest, receipts: this.receipts, usages: this.usages }); }
  restore(snapshot: ReturnType<MockWritingCompactionGateway["snapshot"]>): void {
    this.records.clear(); for (const entry of snapshot.records) this.records.set(entry[0], entry[1]);
    this.current.clear(); for (const entry of snapshot.current) this.current.set(entry[0], entry[1]);
    this.latest.clear(); for (const entry of snapshot.latest) this.latest.set(entry[0], entry[1]);
    this.receipts.clear(); for (const entry of snapshot.receipts) this.receipts.set(entry[0], entry[1]);
    this.usages.clear(); for (const entry of snapshot.usages ?? []) this.usages.set(entry[0], entry[1]);
    for (const record of this.records.values()) if (record.started && record.outcome === null) {
      record.outcome = "interrupted";
      this.history.finishCompaction(this.admission(record), record.itemId, "interrupted");
    }
  }
  private fault(stage: Stage): WritingDemoQueueFailure | undefined {
    const value = this.faults.get(stage);
    if (value !== "unavailable") this.faults.delete(stage);
    if (value === "rejected") throw { code: "invalidInput" };
    if (value === "beforeCommit" || value === "unavailable") throw new Error(`Simulated compaction ${stage} transport unavailable before acknowledgment`);
    return value;
  }
  private available(conversationId: string): void {
    if (this.disposed) throw new Error("模拟压缩网关已关闭");
    this.history.assertAvailable(conversationId);
  }
  private admission(record: Record): WritingSubmissionAdmission {
    const preview = record.prepared.preview;
    return { scope: { conversationId: preview.conversationId, manuscriptId: preview.manuscriptId,
      requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId }, turnId: preview.turnId };
  }
  private record(receipt: AgentWritingReceipt): Record {
    this.available(receipt.conversationId);
    const value = this.records.get(key(receipt));
    if (!value || value.prepared.preview.manuscriptId !== receipt.manuscriptId) throw { code: "notFound" };
    return value;
  }
  private frozenPreparation(record: Record) {
    const row: {request: unknown} = JSON.parse(record.intent), original = decodeAgentWritingCompactionPrepareRequest(row.request);
    return {...original, preview: record.prepared.preview, purpose: "manualCompact" as const,
      ...(record.preparation ? {preparation: record.preparation} : {})};
  }
  private async prepare(request: AgentWritingCompactionPrepareRequest, queued: AgentWritingQueuedCompactionPrepareRequest | null): Promise<Record> {
    this.available(request.request.conversationId);
    const identity = key(request.request), intent = JSON.stringify({ request, queued });
    const existing = this.records.get(identity);
    if (existing) {
      if (canonicalIntent(existing.intent) !== canonicalIntent(intent)) throw { code: "confirmationMismatch" };
      if (queued === null) requireMockWritingPreparation(mockWritingPreparationStatus(request.request,
        this.frozenPreparation(existing), "manualCompact", existing.prepared.expectedRevision));
      return existing;
    }
    const pending = this.preparations.get(identity);
    if (pending) {
      if (pending.intent !== intent) throw { code: "confirmationMismatch" };
      return pending.result;
    }
    const result = this.capture(request, queued, intent);
    this.preparations.set(identity, { intent, result });
    try { return await result; } finally { this.preparations.delete(identity); }
  }
  private async capture(request: AgentWritingCompactionPrepareRequest, queued: AgentWritingQueuedCompactionPrepareRequest | null, intent: string): Promise<Record> {
    if (request.request.text.trim() !== "/compact") throw { code: "invalidInput" };
    this.history.assertWatermark(request.request.conversationId, request.expectedRevision);
    if (queued) {
      this.queue.assertCompactionClaim(queued);
      const identity = inputKey(request.request.conversationId, queued.inputId);
      if (this.current.has(identity)) throw { code: "conflict" };
      const previous = this.latest.get(identity);
      if ((previous ?? null) !== queued.previousRequestId) throw { code: "confirmationMismatch" };
      if (previous) {
        const old = this.records.get(key({ conversationId: request.request.conversationId, requestId: previous }));
        const retirement = old?.queued ? this.queue.readCompactionRetirement(old.prepared.preview, old.queued.inputId) : null;
        if (!old?.queued || (!old.outcome && !retirement) || old.outcome === "completed") throw { code: "confirmationMismatch" };
      }
    }
    const prepared = await this.preparation.materialize(request.request, queued ? "queuedCompact" : "manualCompact"), preview = prepared.preview;
    if (!queued) try {decodeAgentWritingPrepareResult(preview, request.request);} catch {throw {code: "invalidInput"};}
    const itemId = crypto.randomUUID();
    let record!: Record;
    prepared.commit(() => {
      this.available(preview.conversationId);
      this.history.assertWatermark(preview.conversationId, request.expectedRevision);
      const claimQueue = queued ? this.queue.claimCompaction(queued) : null;
      record = { intent, prepared: { preview, expectedRevision: request.expectedRevision }, queued,
        claimQueue, itemId, started: false, outcome: null,
        ...(!queued ? {preparation: mockWritingPreparationReceipt(request.request, preview, "manualCompact", request.expectedRevision, this.prepareEpoch)} : {})};
      this.records.set(key(preview), record);
      if (queued) {
        const identity = inputKey(preview.conversationId, queued.inputId);
        this.current.set(identity, preview.requestId); this.latest.set(identity, preview.requestId);
      }
    });
    return record;
  }
  private async queuedState(record: Record, original = false): Promise<AgentWritingQueuedCompactionPreview> {
    const claim = record.queued;
    if (!claim) throw { code: "confirmationMismatch" };
    const preview = record.prepared.preview;
    const latestQueue = await this.queue.gateway.read(preview);
    const current = this.current.get(inputKey(preview.conversationId, claim.inputId)) === preview.requestId
      && latestQueue.preparingInputId === claim.inputId;
    const retirement = !original ? this.queue.readCompactionRetirement(preview, claim.inputId) : null;
    return decodeAgentWritingQueuedCompactionPreview({ schemaVersion: 1, kind: "compact", inputId: claim.inputId,
      requestId: preview.requestId, claimRevision: claim.expectedQueueRevision, ...record.prepared,
      queue: original || !current ? record.claimQueue : latestQueue, outcome: original ? null : record.outcome,
      current: original || current, ...(retirement ? {retirement} : {}) });
  }
  private async execute(record: Record, controller: AbortController, outcome: AgentQueuedCompactionOutcome): Promise<void> {
    try {
      const preview = record.prepared.preview;
      const request = { conversationId: preview.conversationId, requestId: preview.requestId, signal: controller.signal };
      const before = await this.readUsage(request), result = await this.commands.compact(request);
      if (record.outcome === null) {
        this.usages.set(preview.conversationId, outcome === "completed" ? result.usage : before);
        record.outcome = outcome;
        this.history.finishCompaction(this.admission(record), record.itemId, outcome);
      }
    } catch {
      if (record.outcome === null) {
        record.outcome = controller.signal.aborted ? "canceled" : "failed";
        this.history.finishCompaction(this.admission(record), record.itemId, record.outcome);
      }
    } finally { this.running.delete(key(record.prepared.preview)); }
  }
  private async start(input: AgentCompactionStartRequest) {
    const request = decodeAgentCompactionStartRequest(input);
    if (request.host.kind !== "manuscript") throw { code: "hostUnavailable" };
    const record = this.record({ ...request, manuscriptId: request.host.manuscriptId });
    if (JSON.stringify(agentWritingCompactionStartRequest(record.prepared)) !== JSON.stringify(request)) throw { code: "confirmationMismatch" };
    const alreadyStarted = record.started;
    if (!alreadyStarted) {
      this.history.assertWatermark(request.conversationId, request.expectedRevision);
      if (record.queued) {
        const queue = await this.queue.gateway.read(record.prepared.preview);
        if (queue.preparingInputId !== record.queued.inputId
          || this.current.get(inputKey(request.conversationId, record.queued.inputId)) !== request.requestId) throw { code: "confirmationMismatch" };
      }
      const controller = new AbortController(), outcome = this.nextOutcome;
      this.nextOutcome = "completed";
      this.history.startCompaction(this.admission(record), record.prepared.preview.input.model, record.itemId);
      record.started = true;
      this.running.set(key(request), controller);
      void this.execute(record, controller, outcome);
    }
    return decodeAgentStartResponse({ schemaVersion: 1, conversationId: request.conversationId, requestId: request.requestId,
      turnId: record.prepared.preview.turnId, alreadyStarted, outcome: alreadyStarted ? record.outcome : null });
  }
  async interrupt(request: AgentInterruptRequest) {
    const record = [...this.records.values()].find(value => value.prepared.preview.conversationId === request.conversationId
      && value.prepared.preview.turnId === request.turnId && value.started);
    if (!record) return null;
    const cancellationRequested = record.outcome === null;
    if (cancellationRequested) {
      record.outcome = "canceled";
      this.running.get(key(record.prepared.preview))?.abort();
      this.history.finishCompaction(this.admission(record), record.itemId, "canceled");
    }
    return decodeAgentInterruptResponse({ schemaVersion: 1, ...request, cancellationRequested });
  }
  readonly gateway: AgentWritingCompactionGateway = {
    startStatus: async input => {
      const intent = decodeAgentCompactionStartRequest(input);
      if (intent.host.kind !== "manuscript" || intent.host.manuscriptId === "00000000-0000-0000-0000-000000000000") throw {code: "invalidInput"};
      const record = this.records.get(key(intent));
      let status;
      try {
        if (this.startReaderUnavailable) throw new Error("mock_writing_start_reader_unavailable");
        const original = record ? this.frozenPreparation(record) : null;
        if (record?.queued || record && (record.prepared.preview.manuscriptId !== intent.host.manuscriptId
          || record.prepared.preview.conversationId !== intent.conversationId || record.prepared.preview.requestId !== intent.requestId
          || record.prepared.expectedRevision !== intent.expectedRevision || original?.expectedRevision !== intent.expectedRevision))
          status = mockWritingStartUnconfirmed(intent.requestId, "writing_start_purpose_mismatch");
        else if (record && (record.prepared.preview.contextSnapshotId !== intent.contextSnapshotId || JSON.stringify(record.prepared.preview.input.model) !== JSON.stringify(intent.model)))
          status = mockWritingStartUnconfirmed(intent.requestId, "writing_start_intent_mismatch");
        else if (record && !mockWritingStartHasHistoricalManuscript(record.prepared.preview, intent.host.manuscriptId))
          status = mockWritingStartUnconfirmed(intent.requestId, "writing_start_scope_unconfirmed");
        else {
          const scope = {...intent, manuscriptId: intent.host.manuscriptId};
          const other = !record && this.preparation.inspect?.(scope);
          if (other) status = mockWritingStartUnconfirmed(intent.requestId, "writing_start_purpose_mismatch");
          else {
            const result = await observeMockWritingStart(this.history.gateway, intent, record?.prepared.preview ?? null, "compact");
            status = result ? {type: "started" as const, result} : record ? {type: "prepared" as const, preview: record.prepared.preview} : {type: "notFound" as const};
          }
        }
        return decodeAgentWritingCompactionStartStatusResponse({schemaVersion: 1, intent, status});
      } catch {return decodeAgentWritingCompactionStartStatusResponse({schemaVersion: 1, intent,
        status: mockWritingStartUnconfirmed(intent.requestId, "writing_start_history_unavailable")});}
    },
    prepareWriting: async input => {
      const request = decodeAgentWritingCompactionPrepareRequest(input), fault = this.fault("prepare"), value = await this.prepare(request, null);
      if (fault === "afterCommit") throw new Error("Simulated lost compaction preparation receipt");
      return structuredClone(value.prepared);
    },
    prepareWritingStatus: async input => {
      const intent = decodeAgentWritingCompactionPrepareRequest(input), request = intent.request;
      this.available(request.conversationId);
      const record = this.records.get(key(request));
      let status;
      try {
        const original = record ? this.frozenPreparation(record) : undefined;
        status = this.prepareReaderUnavailable ? mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_reader_unavailable")
          : record?.queued ? mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_purpose_mismatch")
          : original && (original.expectedRevision !== intent.expectedRevision || record!.prepared.expectedRevision !== intent.expectedRevision)
            ? mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_identity_mismatch")
            : mockWritingPreparationStatus(request, original, "manualCompact", intent.expectedRevision);
      } catch {status = mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_record_invalid");}
      if (!record && status.type === "notFound" && this.preparation.inspect?.({conversationId: request.conversationId,
        requestId: request.requestId, manuscriptId: request.context.manuscriptId})) status = mockWritingPreparationUnconfirmed(request, "mock_writing_prepare_record_without_proof");
      return decodeAgentWritingCompactionPrepareStatusResponse({schemaVersion: 1, intent, status: status.type === "committed"
        ? {...status, result: {preview: status.result, expectedRevision: intent.expectedRevision}} : status});
    },
    restoreWriting: async request => {const record = this.record(request); if (!record.queued) decodeAgentWritingPrepareResult(record.prepared.preview, this.frozenPreparation(record).request); return structuredClone(record.prepared);},
    start: async request => {
      const fault = this.fault("start"), value = await this.start(request);
      if (fault === "afterCommit") throw new Error("Simulated lost compaction start receipt");
      return value;
    },
    prepareWritingQueued: async input => {
      const request = decodeAgentWritingQueuedCompactionPrepareRequest(input), fault = this.fault("prepare");
      const value = await this.prepare({ request: request.submission, expectedRevision: request.expectedRevision }, request);
      const response = await this.queuedState(value, true);
      if (fault === "afterCommit") throw new Error("Simulated lost queued compaction preparation receipt");
      return response;
    },
    readWritingQueued: async input => {
      const request = decodeAgentWritingQueuedCompactionReadRequest(input);
      this.available(request.conversationId);
      const identity = inputKey(request.conversationId, request.inputId), queue = await this.queue.gateway.read(request);
      const ordinary = await this.queue.preview.recover(request);
      if (request.requestId === null && queue.preparingInputId === request.inputId && ordinary?.inputId === request.inputId)
        return {schemaVersion: 1, kind: "submission", preview: {...ordinary, queue}};
      const requestId = request.requestId ?? this.current.get(identity)
        ?? (queue.inputs.some(item => item.inputId === request.inputId) ? this.latest.get(identity) : undefined);
      const record = requestId ? this.records.get(key({ ...request, requestId })) : undefined;
      if (record) {
        if (record.queued?.inputId !== request.inputId || record.prepared.preview.manuscriptId !== request.manuscriptId) throw { code: "confirmationMismatch" };
        return this.queuedState(record);
      }
      return ordinary?.inputId === request.inputId && (request.requestId === null || request.requestId === ordinary.preview.requestId)
        ? { schemaVersion: 1, kind: "submission", preview: {...ordinary, queue} } : null;
    },
    settleWritingQueued: async input => {
      const request = decodeAgentWritingQueuedCompactionSettleRequest(input), intent = JSON.stringify(request);
      const fault = this.fault("settle"), previous = this.receipts.get(request.operationId);
      if (previous) {
        if (previous.intent !== intent) throw { code: "confirmationMismatch" };
        return structuredClone(previous.result);
      }
      const record = this.record(request), claim = record.queued;
      if (!claim || claim.inputId !== request.inputId || claim.expectedQueueRevision !== request.claimRevision
        || this.current.get(inputKey(request.conversationId, request.inputId)) !== request.requestId || !record.outcome) throw { code: "confirmationMismatch" };
      const result: AgentWritingQueuedCompactionSettleResponse = { schemaVersion: 1, inputId: request.inputId, requestId: request.requestId,
        outcome: record.outcome, queue: this.queue.settleCompaction(request, claim, record.outcome) };
      this.current.delete(inputKey(request.conversationId, request.inputId));
      this.receipts.set(request.operationId, { intent, result });
      if (fault === "afterCommit") throw new Error("Simulated lost queued compaction settlement receipt");
      return structuredClone(result);
    },
  };
  dispose(): void {
    for (const record of this.records.values()) if (record.started && !record.outcome) {
      record.outcome = "canceled";
      this.running.get(key(record.prepared.preview))?.abort();
      this.history.finishCompaction(this.admission(record), record.itemId, "canceled");
    }
    this.disposed = true;
  }
}
