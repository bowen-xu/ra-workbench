import {decodeAgentContextPrepareIntent, decodeAgentContextPrepareStatusResponse, sameAgentContextPrepareIntent,
  decodeAgentContextStartRequest, decodeAgentContextStartStatusResponse, decodeAgentStartResponse, sameAgentContextStartRequest,
  type AgentContextPrepareIntent, type AgentContextPrepareRequest, type AgentContextPrepareStatusResponse,
  decodeAgentContextQueueEditRequest, type AgentContextQueueEditRequest,
  type AgentContextStartRequest, type AgentStartResponse} from "@research-assistant/core/agent";
import type {AgentContextPrepareProvenGateway, AgentContextPrepareStatusGateway, AgentContextStartGateway} from "@research-assistant/data/agent";
import {flushStorage} from "../../../runtime/storagePersistence";
import type {IndependentQueueResume} from "./independentQueueResume";

type Committed = Extract<AgentContextPrepareStatusResponse["status"], {type: "committed"}>;
interface RecordValue {
  readonly schemaVersion: 1 | 2 | 3 | 4;
  readonly intent: AgentContextPrepareIntent;
  readonly preserveDraft: boolean;
  readonly epoch: string | null;
  readonly committed: Committed | null;
  readonly start?: {readonly request: AgentContextStartRequest; readonly result: AgentStartResponse | null} | null;
  readonly draft?: {readonly text: string | null; readonly input: string | null; readonly version: string | null} | null;
  readonly resume?: {readonly request: AgentContextQueueEditRequest; readonly acknowledged: boolean} | null;
}
export interface IndependentPreparationSnapshot {
  readonly phase: "idle" | "saving" | "sending" | "unknown" | "checking" | "unconfirmed" | "admitted" | "resumePending" | "ready" | "starting" | "started" | "localError";
  readonly record: RecordValue | null;
  readonly cause: unknown;
  readonly ownerSettled?: boolean;
}

/** A durable locator for one preparation. A receipt never grants start or retry permission. */
export class IndependentPreparation {
  private state: IndependentPreparationSnapshot = {phase: "idle", record: null, cause: null};
  private raw: string | null;
  private pending: Promise<void> | null = null;
  constructor(private readonly storage: Storage, private readonly key: string, private readonly conversationId: string,
    private readonly status: AgentContextPrepareStatusGateway, private readonly proven: AgentContextPrepareProvenGateway,
    private readonly changed: (state: IndependentPreparationSnapshot) => void = () => undefined,
    private readonly startGateway?: AgentContextStartGateway) {
    this.raw = storage.getItem(key);
    if (this.raw !== null) {
      try {this.state = {phase: "unknown", record: this.decode(this.raw), cause: null};}
      catch (cause) {this.state = {phase: "localError", record: null, cause};}
    }
  }
  getSnapshot = (): IndependentPreparationSnapshot => this.state;
  private set(value: Partial<IndependentPreparationSnapshot>): void {
    this.state = {...this.state, ...value, ...(value.phase && value.phase !== "started" && value.ownerSettled === undefined ? {ownerSettled:false} : {})};
    this.changed(this.state);
  }
  private decode(raw: string): RecordValue {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
    const row = value as Record<string, unknown>;
    if ((row.schemaVersion === 1 ? Object.keys(row).sort().join() !== "committed,epoch,intent,preserveDraft,schemaVersion"
      : row.schemaVersion === 2 ? Object.keys(row).sort().join() !== "committed,epoch,intent,preserveDraft,schemaVersion,start"
      : row.schemaVersion === 3 ? Object.keys(row).sort().join() !== "committed,draft,epoch,intent,preserveDraft,schemaVersion,start"
      : row.schemaVersion !== 4 || Object.keys(row).sort().join() !== "committed,draft,epoch,intent,preserveDraft,resume,schemaVersion,start") || typeof row.preserveDraft !== "boolean") throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
    const intent = decodeAgentContextPrepareIntent(row.intent);
    if (intent.conversationId !== this.conversationId.toLowerCase()) throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
    let committed: Committed | null = null;
    if (row.committed !== null) {
      const decoded = decodeAgentContextPrepareStatusResponse({schemaVersion: 1, intent, expectedEpoch: row.epoch, status: row.committed});
      if (decoded.status.type !== "committed") throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
      committed = decoded.status;
    }
    if (row.epoch !== null && (typeof row.epoch !== "string" || !/^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu.test(row.epoch) || /^0{8}-(?:0{4}-){3}0{12}$/u.test(row.epoch))) throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
    if (committed && row.epoch === null) throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
    let start: RecordValue["start"] = null;
    if ((row.schemaVersion === 2 || row.schemaVersion === 3 || row.schemaVersion === 4) && row.start !== null) {
      if (!row.start || typeof row.start !== "object" || Array.isArray(row.start) || Object.keys(row.start).sort().join() !== "request,result" || !committed) throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
      const value = row.start as Record<string, unknown>, request = decodeAgentContextStartRequest(value.request);
      this.validateStart(request, committed);
      const result = value.result === null ? null : decodeAgentStartResponse(value.result);
      if (result) this.validateStarted(result, request, committed);
      start = {request, result};
    }
    let draft: RecordValue["draft"] = null;
    if ((row.schemaVersion === 3 || row.schemaVersion === 4) && row.draft !== null) {
      if (!row.draft || typeof row.draft !== "object" || Array.isArray(row.draft) || Object.keys(row.draft).sort().join() !== "input,text,version") throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
      const value = row.draft as Record<string, unknown>;
      if ((value.text !== null && typeof value.text !== "string") || (value.input !== null && typeof value.input !== "string") || (value.version !== null && typeof value.version !== "string")) throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
      draft = {text: value.text as string | null, input: value.input as string | null, version: value.version as string | null};
    }
    let resume: RecordValue["resume"] = null;
    if (row.schemaVersion === 4 && row.resume !== null) {
      if (!row.resume || typeof row.resume !== "object" || Array.isArray(row.resume) || Object.keys(row.resume).sort().join() !== "acknowledged,request") throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
      const value = row.resume as Record<string, unknown>, request = decodeAgentContextQueueEditRequest(value.request);
      if (!start?.request.queue || !row.epoch || request.conversationId !== intent.conversationId || request.expectedEpoch !== row.epoch || request.edit.type !== "resume" || typeof value.acknowledged !== "boolean") throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
      resume = {request, acknowledged: value.acknowledged};
    }
    return {schemaVersion: row.schemaVersion as 1 | 2 | 3 | 4, intent, preserveDraft: row.preserveDraft, epoch: typeof row.epoch === "string" ? row.epoch.toLowerCase() : null, committed,
      ...(row.schemaVersion !== 1 ? {start} : {}), ...((row.schemaVersion === 3 || row.schemaVersion === 4) ? {draft} : {}), ...(row.schemaVersion === 4 ? {resume} : {})};
  }
  private validateStart(request: AgentContextStartRequest, committed: Committed): void {
    const preview = committed.result;
    if (request.conversationId !== preview.conversationId || request.requestId !== preview.requestId
      || request.contextSnapshotId !== preview.contextSnapshotId || JSON.stringify(request.queue) !== JSON.stringify(preview.queue ?? null))
      throw new Error("CONTEXT_START_MISMATCH");
  }
  private validateStarted(result: AgentStartResponse, request: AgentContextStartRequest, committed: Committed): void {
    if (result.conversationId !== request.conversationId || result.requestId !== request.requestId || result.turnId !== committed.result.turnId)
      throw new Error("CONTEXT_START_MISMATCH");
  }
  private async verifyPreparation(record: RecordValue): Promise<void> {
    if (!record.epoch || !record.committed) throw new Error("CONTEXT_PREPARE_PROOF_REQUIRED");
    const observed = decodeAgentContextPrepareStatusResponse(await this.status.read(structuredClone(record.intent), {expectedEpoch: record.epoch}));
    if (!sameAgentContextPrepareIntent(record.intent, observed.intent) || observed.expectedEpoch !== record.epoch || observed.status.type !== "committed"
      || observed.status.proof.epoch !== record.committed.proof.epoch || observed.status.proof.sqlFence !== record.committed.proof.sqlFence
      || JSON.stringify(observed.status.result) !== JSON.stringify(record.committed.result)) throw new Error("CONTEXT_PREPARE_PROOF_MISMATCH");
  }
  private async persist(record: RecordValue): Promise<void> {
    if (this.storage.getItem(this.key) !== this.raw) throw new Error("CONTEXT_PREPARE_RECOVERY_CONFLICT");
    const raw = JSON.stringify(record);
    this.storage.setItem(this.key, raw); this.raw = raw;
    await flushStorage(this.storage);
  }
  private retained(run: () => Promise<void>): Promise<void> {
    if (this.pending) return this.pending;
    const pending = Promise.resolve().then(run); this.pending = pending;
    return pending.finally(() => {if (this.pending === pending) this.pending = null;});
  }
  begin(request: AgentContextPrepareRequest, preserveDraft: boolean, draft?: {readonly text: string | null; readonly input: string | null; readonly version: string | null}): Promise<void> {
    return this.retained(async () => {
      if (this.state.phase !== "idle") return;
      const intent = decodeAgentContextPrepareIntent(request);
      if (intent.conversationId !== this.conversationId.toLowerCase()) throw new Error("CONTEXT_PREPARE_RECOVERY_INVALID");
      let record: RecordValue = draft === undefined ? {schemaVersion: 1, intent, preserveDraft, epoch: null, committed: null}
        : {schemaVersion: 3, intent, preserveDraft, epoch: null, committed: null, start: null, draft: structuredClone(draft)};
      this.set({phase: "saving", record, cause: null});
      try {await this.persist(record);} catch (cause) {this.set({phase: "localError", cause}); return;}
      try {
        const observed = decodeAgentContextPrepareStatusResponse(await this.status.read(structuredClone(intent)));
        if (!sameAgentContextPrepareIntent(intent, observed.intent) || observed.expectedEpoch !== null || observed.status.type !== "unconfirmed" || !observed.status.detail.epoch) throw new Error("CONTEXT_PREPARE_EPOCH_UNCONFIRMED");
        record = {...record, epoch: observed.status.detail.epoch};
        this.set({phase: "saving", record}); await this.persist(record);
      } catch (cause) {this.set({phase: "unknown", cause}); return;}
      this.set({phase: "sending"});
      let committed: Committed;
      try {
        committed = await this.proven.prepare(structuredClone(intent), {expectedEpoch: record.epoch!});
        const checked = decodeAgentContextPrepareStatusResponse({schemaVersion: 1, intent, expectedEpoch: record.epoch, status: committed});
        if (checked.status.type !== "committed") throw new Error("CONTEXT_PREPARE_PROOF_REQUIRED");
        committed = checked.status;
      } catch (cause) {
        // The native proven prepare command emits this specific rejection only
        // before its data transaction begins. Other errors remain uncertain.
        if (cause && typeof cause === "object" && !Array.isArray(cause)
          && Object.keys(cause).length === 1 && (cause as {code?: unknown}).code === "modelUnavailable") {
          try {
            if (this.storage.getItem(this.key) !== this.raw) throw new Error("CONTEXT_PREPARE_RECOVERY_CONFLICT");
            this.storage.removeItem(this.key); this.raw = null; await flushStorage(this.storage);
            this.set({phase: "idle", record: null, cause}); return;
          } catch (failure) {this.set({phase: "unknown", cause: failure}); return;}
        }
        this.set({phase: "unknown", cause}); return;
      }
      record = {...record, committed}; this.set({phase: "saving", record});
      try {await this.persist(record); this.set({phase: "ready"});} catch (cause) {this.set({phase: "unknown", cause});}
    });
  }
  check(): Promise<void> {
    return this.retained(async () => {
      let record = this.state.record;
      if (!record || !record.epoch) {this.set({phase: "unknown", cause: new Error("CONTEXT_PREPARE_EPOCH_UNCONFIRMED")}); return;}
      try {
        // A prior write may have reached the in-memory storage but failed to flush.
        // Do not expose a recovered result until its locator is durable.
        await this.persist(record);
        this.set({phase: "checking", cause: null, ownerSettled: false});
        if (record.start) {await this.inspectStart(record); return;}
        const response = decodeAgentContextPrepareStatusResponse(await this.status.read(structuredClone(record.intent), {expectedEpoch: record.epoch}));
        if (!sameAgentContextPrepareIntent(record.intent, response.intent) || response.expectedEpoch !== record.epoch) throw new Error("CONTEXT_PREPARE_STATUS_MISMATCH");
        if (response.status.type === "unconfirmed") {this.set({phase: "unconfirmed", cause: response.status.detail}); return;}
        if (record.committed && (record.committed.proof.epoch !== response.status.proof.epoch || record.committed.proof.sqlFence !== response.status.proof.sqlFence || JSON.stringify(record.committed.result) !== JSON.stringify(response.status.result))) throw new Error("CONTEXT_PREPARE_PROOF_MISMATCH");
        if (!record.committed) {record = {...record, committed: response.status}; this.set({phase: "saving", record}); await this.persist(record);}
        this.set({phase: "ready"});
      } catch (cause) {this.set({phase: "unknown", cause});}
    });
  }
  start(resumeQueue = false): Promise<void> {
    return this.retained(async () => {
      let record = this.state.record;
      if (this.state.phase !== "ready" || !record?.epoch || !record.committed || record.start || !this.startGateway) return;
      try {
        await this.verifyPreparation(record);
        const preview = record.committed.result;
        const request = decodeAgentContextStartRequest({conversationId: preview.conversationId, requestId: preview.requestId,
          contextSnapshotId: preview.contextSnapshotId, queue: preview.queue ?? null, resumeQueue});
        this.validateStart(request, record.committed);
        record = {...record, schemaVersion: record.schemaVersion === 3 ? 3 : 2, start: {request, result: null}};
        this.set({phase: "saving", record}); await this.persist(record);
        this.set({phase: "starting"});
        const result = decodeAgentStartResponse(await this.startGateway.start(structuredClone(request), record.epoch!));
        this.validateStarted(result, request, record.committed!);
        record = {...record, start: {request, result}}; this.set({phase: "saving", record});
        await this.persist(record); this.set({phase: "started"});
      } catch (cause) {this.set({phase: "unknown", cause});}
    });
  }
  /** Explicitly retry the persisted request only after rechecking its admission. */
  continueAdmitted(readQueue: (conversationId: string) => Promise<{conversationId: string; paused: boolean}>): Promise<void> {
    return this.retained(async () => {
      const original = this.state.record;
      if (this.state.phase !== "admitted" || !original?.epoch || !original.start || !original.committed || !this.startGateway) return;
      try {
        await this.persist(original);
        await this.inspectStart(original);
        if (this.state.phase !== "admitted") return;
        const queue = await readQueue(original.intent.conversationId);
        if (queue.conversationId !== original.intent.conversationId || queue.paused) throw new Error("CONTEXT_QUEUE_PAUSED_OR_UNAVAILABLE");
        // Keep the exact original request and its resume flag; neither queue
        // inspection nor receipt replay authorizes lifting a later pause.
        await this.persist(original);
        this.set({phase: "starting", cause: null});
        const result = decodeAgentStartResponse(await this.startGateway.start(structuredClone(original.start.request), original.epoch));
        this.validateStarted(result, original.start.request, original.committed);
        const record = {...original, start: {request: original.start.request, result}};
        this.set({phase: "saving", record}); await this.persist(record); this.set({phase: "started"});
      } catch (cause) {this.set({phase: "unknown", cause});}
    });
  }
  /** Resolve one persisted Resume before releasing either queue or start recovery. */
  continuePaused(queue: IndependentQueueResume, eligible: () => boolean = () => true): Promise<void> {
    return this.retained(async () => {
      let record = this.state.record;
      if (!(this.state.phase === "admitted" || this.state.phase === "resumePending") || !record?.epoch || !record.start?.request.queue || !record.committed || !this.startGateway) return;
      try {
        await this.persist(record);
        await this.inspectStart(record);
        record = this.state.record!;
        if (!record.resume && this.state.phase !== "admitted") return;
        if (!record.resume && !(await queue.read(record.intent.conversationId)).paused) {this.set({phase: "admitted"}); return;}
        const original = record.resume?.request ?? null;
        if (!record.resume?.acknowledged) {
          const outcome = await queue.resume(record.intent.conversationId, record.epoch!, original, async request => {
            if (this.storage.getItem(this.key) !== this.raw || (original && JSON.stringify(original) !== JSON.stringify(request))) throw new Error("CONTEXT_QUEUE_RECOVERY_CONFLICT");
            record = {...record!, schemaVersion: 4, draft: record!.draft ?? null, resume: {request, acknowledged: false}};
            this.set({phase: "saving", record}); await this.persist(record);
          }, () => eligible() && this.storage.getItem(this.key) === this.raw);
          if (outcome === "knownRejected") {
            record = {...record!, schemaVersion: 4, draft: record!.draft ?? null, resume: null};
            this.set({phase: "saving", record}); await this.persist(record);
            await this.inspectStart(record);
            if (this.getSnapshot().phase !== "started") this.set({cause: new Error("CONTEXT_QUEUE_RESUME_REJECTED")});
            return;
          }
          if (outcome !== "acknowledged" || !record!.resume) {this.set({phase: record!.resume ? "resumePending" : "unknown", cause: new Error("CONTEXT_QUEUE_RESUME_UNCONFIRMED")}); return;}
          record = {...record!, resume: {...record!.resume, acknowledged: true}};
          this.set({phase: "saving", record}); await this.persist(record);
        }
        if (!eligible()) {this.set({phase: "resumePending"}); return;}
        record = {...record!, resume: null}; this.set({phase: "saving", record}); await this.persist(record);
        await this.inspectStart(record);
        if (this.state.phase !== "admitted") return;
        if ((await queue.read(record.intent.conversationId)).paused) {this.set({phase: "admitted", cause: new Error("CONTEXT_QUEUE_PAUSED_OR_UNAVAILABLE")}); return;}
        if (!eligible()) return;
        // A fresh explicit continuation rechecks admission after the resume receipt.
        this.set({phase: "starting", cause: null});
        const result = decodeAgentStartResponse(await this.startGateway.start(structuredClone(record.start!.request), record.epoch!));
        this.validateStarted(result, record.start!.request, record.committed!);
        record = {...record, start: {request: record.start!.request, result}};
        this.set({phase: "saving", record}); await this.persist(record); this.set({phase: "started"});
      } catch (cause) {this.set({phase: "unknown", cause});}
    });
  }
  private async inspectStart(record: RecordValue): Promise<void> {
    if (!record.epoch || !record.committed || !record.start || !this.startGateway) throw new Error("CONTEXT_START_STATUS_UNAVAILABLE");
    await this.verifyPreparation(record);
    const response = decodeAgentContextStartStatusResponse(await this.startGateway.read(structuredClone(record.start.request), record.epoch));
    if (response.expectedEpoch !== record.epoch || !sameAgentContextStartRequest(response.intent, record.start.request)) throw new Error("CONTEXT_START_MISMATCH");
    if (response.status.type === "unconfirmed") {this.set({phase: record.resume ? "resumePending" : "unconfirmed", cause: response.status.detail}); return;}
    if (response.status.type === "admitted") {
      if (record.start.result || JSON.stringify(response.status.preview) !== JSON.stringify(record.committed.result)) throw new Error("CONTEXT_START_MISMATCH");
      this.set({phase: record.resume ? "resumePending" : "admitted", cause: null}); return;
    }
    this.validateStarted(response.status.result, record.start.request, record.committed);
    const saved = record.start.result;
    if (saved && (saved.conversationId !== response.status.result.conversationId || saved.requestId !== response.status.result.requestId || saved.turnId !== response.status.result.turnId
      || (saved.runId && response.status.result.runId && saved.runId !== response.status.result.runId)
      || (saved.outcome !== null && response.status.result.outcome !== null && saved.outcome !== response.status.result.outcome))) throw new Error("CONTEXT_START_MISMATCH");
    const result = saved && ((!saved.runId && response.status.result.runId) || (saved.outcome === null && response.status.result.outcome !== null))
      ? {...saved, ...(!saved.runId && response.status.result.runId ? {runId: response.status.result.runId} : {}),
        outcome: saved.outcome ?? response.status.result.outcome} : saved ?? response.status.result;
    if (!saved || result !== saved) {record = {...record, start: {request: record.start.request, result}}; this.set({phase: "saving", record}); await this.persist(record);}
    if (response.status.type === "startedUnobserved" && !result.runId) {
      this.set({phase: "unconfirmed", cause: {code:"AGENT_TASK_IDENTITY_UNAVAILABLE"}}); return;
    }
    this.set({phase: record.resume ? "resumePending" : "started", ownerSettled: !record.resume && (response.status.type === "startedSettled" || response.status.type === "legacyStartedSettled")});
  }
  finish(consume: (record: Readonly<RecordValue>) => void | Promise<void>): Promise<void> {
    return this.retained(async () => {
      const record = this.state.record;
      if (this.state.phase !== "started" || !record?.start?.result || !record.committed || record.resume) return;
      try {
        if (this.storage.getItem(this.key) !== this.raw) throw new Error("CONTEXT_PREPARE_RECOVERY_CONFLICT");
        await consume(record);
        await flushStorage(this.storage);
        if (this.storage.getItem(this.key) !== this.raw) throw new Error("CONTEXT_PREPARE_RECOVERY_CONFLICT");
        this.storage.removeItem(this.key); this.raw = null;
        await flushStorage(this.storage);
        this.set({phase: "idle", record: null, cause: null});
      } catch (cause) {this.set({phase: "unknown", cause});}
    });
  }
}
