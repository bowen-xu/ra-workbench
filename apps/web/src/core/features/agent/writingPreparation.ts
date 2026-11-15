import {decodeAgentWritingPrepareRequest, decodeAgentWritingPrepareResult, decodeAgentWritingPrepareStatusResponse,
  sameAgentWritingPrepareRequest, decodeAgentWorkspaceBuffer, type AgentWritingPrepareRequest, type AgentSubmissionPreview,
  decodeAgentWritingStartRequest, decodeAgentWritingStartResponse, decodeAgentWritingStartStatusResponse, sameAgentWritingStartRequest,
  type AgentWritingStartRequest, type AgentWritingStartResponse, type AgentWritingStartStatusResponse,
  type AgentWorkspaceBuffer, type AgentWritingPrepareStatusResponse, type AgentWritingQueueScope} from "@research-assistant/core/agent";
import type {AgentSubmissionGateway} from "@research-assistant/data/agent";
import {flushStorage, storagePersistence} from "../../runtime/storagePersistence";
import {writingPreparationRejection} from "./writingPreparationRejection";
import {isAgentStartRejection} from "./agentPreparationFailure";
import {decodeWritingDraftAcknowledgment, type WritingDraftAcknowledgmentPort} from "./writingDraftAcknowledgment";
import type {ConversationDraftAcknowledgmentToken} from "./conversationDraftStore";
import {WritingDispatchAdmissionClosed} from "./writingDispatchAdmission";

export interface WritingPreparedStart {
  readonly request: AgentWritingStartRequest;
  readonly result: AgentWritingStartResponse | null;
  readonly priorUnknown: boolean;
  readonly knownRejected: boolean;
  readonly rejection?: string | {readonly code: string; readonly reason?: "conflict"} | null;
}

export interface WritingPreparationSnapshot {
  readonly phase: "idle" | "saving" | "sending" | "unknown" | "checking" | "notFound" | "unconfirmed" | "rejected" | "ready" | "clearing" | "localError" | "complete" | "retired";
  readonly intent: AgentWritingPrepareRequest | null;
  readonly result: AgentSubmissionPreview | null;
  readonly status: AgentWritingPrepareStatusResponse["status"] | null;
  readonly knownCommitted: boolean;
  readonly priorUnknown: boolean;
  readonly knownRejected: boolean;
  readonly recovered: boolean;
  readonly workspaceBuffers: readonly AgentWorkspaceBuffer[] | null;
  readonly workspaceReady: boolean;
  readonly draftAcknowledgment: ConversationDraftAcknowledgmentToken | null;
  readonly start: WritingPreparedStart | null;
  readonly startStatus: AgentWritingStartStatusResponse["status"] | null;
  readonly cause: unknown;
}
const initial = (): WritingPreparationSnapshot => ({phase: "idle", intent: null, result: null, status: null, knownCommitted: false, priorUnknown: false,
  knownRejected: false, recovered: false, workspaceBuffers: null, workspaceReady: false, draftAcknowledgment: null, start: null, startStatus: null, cause: null});
const scope = (intent: AgentWritingPrepareRequest) => ({conversationId: intent.conversationId, manuscriptId: intent.context.manuscriptId});
const key = (value: AgentWritingQueueScope) => `research-assistant:writing-prepare:v1:${value.manuscriptId.toLowerCase()}:${value.conversationId.toLowerCase()}`;
function startRejection(cause: unknown): Exclude<WritingPreparedStart["rejection"], undefined> {
  if (!isAgentStartRejection(cause)) return null;
  if (typeof cause === "string") return cause;
  const value = cause as {code: string}; return value.code === "preparation" ? {code: value.code, reason: "conflict"} : {code: value.code};
}

/** The ordinary preparation substate belongs to the existing submission owner.
 * The complete local request is a locator; native proof remains authoritative. */
export class WritingPreparation {
  private state = initial();
  private disposed = false;
  private reserved = false;
  private clearing = false;
  private retiring = false;
  private pending: Promise<void> | null = null;
  private prepareMayHaveDispatched = false;
  private startMayHaveDispatched = false;
  private drafts: WritingDraftAcknowledgmentPort | null = null;
  private observed: ((request: AgentWritingStartRequest, result: AgentWritingStartResponse) => void | Promise<void>) | null = null;
  constructor(private readonly gateway: AgentSubmissionGateway, private readonly storage: Storage,
    private readonly changed: (state: WritingPreparationSnapshot) => void, ownScope?: AgentWritingQueueScope,
    private readonly isAdmissionDisabled: () => boolean = () => false) {if (ownScope) this.restore(ownScope);}
  getSnapshot = () => this.state;
  unresolved = () => !["idle", "complete", "retired"].includes(this.state.phase);
  retainsLocator = () => this.reserved || this.clearing;
  configureDraftAcknowledgments(port: WritingDraftAcknowledgmentPort): void {this.drafts = port;}
  configureStartObserver(observer: (request: AgentWritingStartRequest, result: AgentWritingStartResponse) => void | Promise<void>): void {this.observed = observer;}
  canStart = () => this.state.knownCommitted && this.state.workspaceReady && !this.retiring && this.state.phase === "ready";
  private update(patch: Partial<WritingPreparationSnapshot>): void {if (!this.disposed) {this.state = {...this.state, ...patch}; this.changed(this.state);}}
  private buffers(intent: AgentWritingPrepareRequest, value: unknown): readonly AgentWorkspaceBuffer[] | null {
    if (value == null) return intent.context.files.some(file => file.buffer != null) ? null : [];
    if (!Array.isArray(value)) throw new Error("WRITING_PREPARE_RECOVERY_INVALID");
    const buffers = value.map(decodeAgentWorkspaceBuffer);
    if (new Set(buffers.map(buffer => buffer.path)).size !== buffers.length || intent.context.files.some(file => file.buffer && !buffers.some(buffer =>
      buffer.path === file.relativePath && buffer.revision === file.buffer!.revision && buffer.text === file.buffer!.text && buffer.expectedDiskSha256 === file.expectedDiskSha256)))
      throw new Error("WRITING_PREPARE_RECOVERY_INVALID");
    return buffers;
  }
  private read(ownScope: AgentWritingQueueScope) {
    const raw = this.storage.getItem(key(ownScope)); if (raw === null) return null;
    try {
      const value: unknown = JSON.parse(raw);
      if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(field => !["schemaVersion", "intent", "knownRejected", "rejection", "result", "workspaceBuffers", "workspaceReady", "draftAcknowledgment", "start"].includes(field))
        || !("schemaVersion" in value) || value.schemaVersion !== 1 && value.schemaVersion !== 2 || !("intent" in value) || !("knownRejected" in value) || typeof value.knownRejected !== "boolean"
        || value.schemaVersion === 2 && (!("draftAcknowledgment" in value) || !("start" in value))) throw new Error();
      const intent = decodeAgentWritingPrepareRequest(value.intent);
      if (intent.conversationId !== ownScope.conversationId.toLowerCase() || intent.context.manuscriptId !== ownScope.manuscriptId.toLowerCase()) throw new Error();
      const result = "result" in value && value.result != null ? decodeAgentWritingPrepareResult(value.result, intent) : null;
      let cause: unknown = "rejection" in value ? writingPreparationRejection(value.rejection, intent) : null;
      if (value.knownRejected && cause === null) throw new Error();
      if (!("workspaceReady" in value) || typeof value.workspaceReady !== "boolean" || value.workspaceReady && !result) throw new Error();
      const draftAcknowledgment = decodeWritingDraftAcknowledgment("draftAcknowledgment" in value ? value.draftAcknowledgment : null, ownScope);
      let start: WritingPreparedStart | null = null;
      if ("start" in value && value.start != null) {
        const row = value.start as Partial<WritingPreparedStart>;
        if (!row || typeof row !== "object" || Object.keys(row).some(field => !["request", "result", "priorUnknown", "knownRejected", "rejection"].includes(field))
          || !Object.hasOwn(row, "result") || typeof row.priorUnknown !== "boolean" || typeof row.knownRejected !== "boolean" || !result || !value.workspaceReady) throw new Error();
        const request = decodeAgentWritingStartRequest(row.request), started = row.result == null ? null : decodeAgentWritingStartResponse(row.result);
        this.validateStart(request, result);
        if (started) this.validateStarted(started, request, result);
        if (row.priorUnknown && row.knownRejected || started && row.knownRejected) throw new Error();
        const rejection = startRejection(row.rejection);
        if (row.knownRejected && !rejection) throw new Error();
        if (row.knownRejected) cause = rejection;
        start = {request, result: started, priorUnknown: row.priorUnknown, knownRejected: row.knownRejected, rejection};
      }
      return {intent, result, knownRejected: value.knownRejected, cause, workspaceReady: value.workspaceReady,
        workspaceBuffers: this.buffers(intent, "workspaceBuffers" in value ? value.workspaceBuffers : null), draftAcknowledgment, start};
    } catch {throw new Error("WRITING_PREPARE_RECOVERY_INVALID");}
  }
  restore(ownScope: AgentWritingQueueScope): boolean {
    if (this.disposed || this.unresolved()) return true;
    try {const saved = this.read(ownScope); if (saved) {this.reserved = true; this.prepareMayHaveDispatched = true; this.startMayHaveDispatched = saved.start !== null; this.update({...initial(), ...saved, phase: "unknown", recovered: true,
      knownCommitted: saved.result !== null, priorUnknown: !saved.knownRejected, workspaceReady: saved.start !== null && saved.workspaceReady,
      start: saved.start ? {...saved.start, priorUnknown: saved.start.result === null && !saved.start.knownRejected || saved.start.priorUnknown} : null});}}
    catch (cause: unknown) {this.update({...initial(), phase: "localError", recovered: true, cause});}
    return this.unresolved();
  }
  private async persist(): Promise<void> {
    const intent = this.state.intent; if (!intent) throw new Error("WRITING_PREPARE_RECOVERY_INVALID");
    const saved = this.read(scope(intent));
    if ((this.reserved && !saved) || saved && (!sameAgentWritingPrepareRequest(saved.intent, intent)
      || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)
      || saved.start && (!this.state.start || !sameAgentWritingStartRequest(saved.start.request, this.state.start.request)))) throw new Error("WRITING_PREPARE_RECOVERY_CONFLICT");
    this.storage.setItem(key(scope(intent)), JSON.stringify({schemaVersion: 2, intent, knownRejected: this.state.knownRejected,
      rejection: this.state.knownRejected ? writingPreparationRejection(this.state.cause, intent) ?? saved?.cause : null, result: this.state.result,
      workspaceBuffers: this.state.workspaceBuffers, workspaceReady: this.state.workspaceReady, draftAcknowledgment: this.state.draftAcknowledgment, start: this.state.start}));
    this.reserved = true; await flushStorage(this.storage);
  }
  private async retained(run: () => Promise<void>): Promise<void> {
    if (this.disposed) return; if (this.pending) return this.pending;
    const pending = run(); this.pending = pending; try {await pending;} finally {if (this.pending === pending) this.pending = null;}
  }
  begin(value: AgentWritingPrepareRequest, workspaceBuffers?: readonly AgentWorkspaceBuffer[], draftAcknowledgment?: ConversationDraftAcknowledgmentToken | null): Promise<void> {
    return this.retained(async () => {
      if (this.unresolved()) return; const intent = decodeAgentWritingPrepareRequest(value); if (this.restore(scope(intent))) return;
      this.prepareMayHaveDispatched = false; this.startMayHaveDispatched = false;
      this.retiring = false; this.update({...initial(), phase: "saving", intent, workspaceBuffers: this.buffers(intent, workspaceBuffers),
        draftAcknowledgment: decodeWritingDraftAcknowledgment(draftAcknowledgment, scope(intent))}); await this.send();
    });
  }
  private async committed(result: AgentSubmissionPreview): Promise<void> {
    this.update({knownCommitted: true, result, knownRejected: false, cause: null});
    try {await this.persist(); if (!this.disposed) this.update({phase: "ready"});}
    catch (cause: unknown) {this.update({phase: "localError", cause});}
  }
  private async send(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed || this.state.knownCommitted) return;
    this.update({phase: "saving", cause: null});
    try {await this.persist(); await this.flushDraft();} catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return;
    const request = structuredClone(intent);
    this.update({phase: "sending"});
    if (this.disposed) return;
    if ((!this.prepareMayHaveDispatched || this.state.knownRejected && !this.state.priorUnknown) && this.isAdmissionDisabled()) {
      this.update({phase: "localError", cause: new WritingDispatchAdmissionClosed("prepare", intent.conversationId, intent.context.manuscriptId, intent.requestId)}); return;
    }
    this.prepareMayHaveDispatched = true;
    try {const result = decodeAgentWritingPrepareResult(await this.gateway.prepareWriting(request), intent); if (!this.disposed) await this.committed(result);}
    catch (cause: unknown) {
      if (this.disposed) return; const known = !this.state.priorUnknown && writingPreparationRejection(cause, intent) !== null;
      this.update({phase: known ? "rejected" : "unknown", knownRejected: known, priorUnknown: !known, cause});
      if (known) try {await this.persist();} catch (local: unknown) {this.update({phase: "localError", cause: local});}
    }
  }
  private async inspect(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return;
    if (this.state.phase === "complete" || this.state.phase === "retired") return;
    if (this.retiring) {await this.clear(true); return;}
    if (this.state.start) {await this.inspectStart(); return;}
    if (this.state.knownCommitted && this.state.result) {
      await this.committed(this.state.result);
      return;
    }
    const cause = this.state.cause; this.update({phase: "checking", recovered: true, cause: null});
    try {
      const response = decodeAgentWritingPrepareStatusResponse(await this.gateway.prepareWritingStatus(structuredClone(intent)));
      if (!sameAgentWritingPrepareRequest(response.intent, intent)) throw new Error("WRITING_PREPARE_STATUS_MISMATCH");
      if (this.disposed) return; this.update({status: response.status});
      if (response.status.type === "committed") await this.committed(response.status.result);
      else this.update({phase: response.status.type, cause: this.state.knownRejected || !this.prepareMayHaveDispatched && cause instanceof WritingDispatchAdmissionClosed ? cause : null});
    } catch (cause: unknown) {this.update({phase: "unknown", cause});}
  }
  check = (): Promise<void> => this.retained(() => this.inspect());
  retry = (): Promise<void> => this.retained(async () => {
    try {if (storagePersistence(this.storage)?.error) await storagePersistence(this.storage)!.retry(); if (this.state.draftAcknowledgment) await this.drafts?.retry();}
    catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return; await this.inspect();
    if (this.state.start) {if (!this.state.start.result && this.state.startStatus?.type === "prepared") await this.sendStart();}
    else if (this.state.phase === "notFound" || this.state.phase === "unconfirmed") await this.send();
  });
  private async clear(retired = false): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return; this.update({phase: "clearing", cause: null});
    try {
      const saved = this.read(scope(intent));
      if ((!saved && !this.clearing) || saved && (!sameAgentWritingPrepareRequest(saved.intent, intent)
        || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)
        || saved.start && (!this.state.start || !sameAgentWritingStartRequest(saved.start.request, this.state.start.request)))) throw new Error("WRITING_PREPARE_RECOVERY_CONFLICT");
      this.storage.removeItem(key(scope(intent))); this.clearing = true; await flushStorage(this.storage);
      this.reserved = false; this.clearing = false; this.update({phase: retired ? "retired" : "complete", cause: null});
    } catch (cause: unknown) {this.update({phase: "localError", cause});}
  }
  async finishWorkspace(requestId: string): Promise<void> {
    if (this.pending) {await this.pending; if (!this.disposed) return this.finishWorkspace(requestId); return;}
    return this.retained(async () => {
      if (!this.state.knownCommitted || this.state.intent?.requestId !== requestId || this.state.phase === "complete" || this.state.phase === "retired") return;
      if (!this.state.workspaceReady) {this.update({workspaceReady: true, phase: "saving"}); try {await this.persist();} catch (cause: unknown) {this.update({phase: "localError", cause}); return;}}
      this.update({phase: "ready", cause: null});
    });
  }
  retire = (): Promise<void> => this.retained(async () => {
    if (!this.state.knownRejected || this.state.priorUnknown || this.state.knownCommitted) return;
    await this.inspect(); if (!this.disposed && this.state.phase === "notFound") {this.retiring = true; await this.clear(true);}
  });
  /** Only retire a definitely unstarted request after checking its original identity. */
  retireVerifiedUnstarted = (): Promise<void> => this.retained(async () => {
    const state = this.state;
    if (!state.intent || state.knownCommitted && !state.start) return;
    if (state.start) {
      if (state.start.result || state.start.priorUnknown || state.startStatus?.type !== "prepared") return;
      if (state.priorUnknown && this.startMayHaveDispatched) return;
    } else if (state.priorUnknown || state.status?.type !== "notFound") return;
    const saved = this.read(scope(state.intent));
    if (state.start ? saved?.start?.result || saved?.start?.priorUnknown : saved?.result) return;
    this.retiring = true; await this.clear(true);
  });
  abandonCommitted = (): Promise<void> => this.retained(async () => {
    if (!this.state.knownCommitted || this.state.start && (!this.state.start.knownRejected || this.state.start.priorUnknown || this.state.start.result)) return;
    this.retiring = true; await this.clear(true);
  });

  private async flushDraft(): Promise<void> {
    if (!this.state.draftAcknowledgment) return;
    if (!this.drafts) throw new Error("WRITING_PREPARE_DRAFT_ACK_UNAVAILABLE");
    await this.drafts.flush();
  }
  private validateStart(request: AgentWritingStartRequest, preview: AgentSubmissionPreview): void {
    if (request.requestId !== preview.requestId || request.conversationId !== preview.conversationId || request.manuscriptId !== preview.manuscriptId
      || request.contextSnapshotId !== preview.contextSnapshotId || JSON.stringify(request.model) !== JSON.stringify(preview.input.model)) throw new Error("WRITING_PREPARE_START_MISMATCH");
  }
  private validateStarted(result: AgentWritingStartResponse, request: AgentWritingStartRequest, preview: AgentSubmissionPreview): void {
    if (result.requestId !== request.requestId || result.conversationId !== request.conversationId || result.turnId !== preview.turnId)
      throw new Error("WRITING_PREPARE_START_MISMATCH");
  }
  confirmStart(value: AgentWritingStartRequest): Promise<void> {
    return this.retained(async () => {
      if (this.state.start) {await this.inspectStart(); if (this.state.startStatus?.type === "prepared" && !this.state.start.result) await this.sendStart(); return;}
      if (!this.canStart() || !this.state.result) return;
      const request = decodeAgentWritingStartRequest(value); this.validateStart(request, this.state.result);
      this.startMayHaveDispatched = false;
      this.update({start: {request, result: null, priorUnknown: false, knownRejected: false}, startStatus: null});
      await this.sendStart();
    });
  }
  private async sendStart(): Promise<void> {
    const start = this.state.start, preview = this.state.result; if (!start || !preview || start.result || this.disposed) return;
    this.update({phase: "saving", cause: null});
    try {await this.persist(); await this.flushDraft();} catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return;
    const request = structuredClone(start.request);
    this.update({phase: "sending", startStatus: null});
    if (this.disposed) return;
    if ((!this.startMayHaveDispatched || start.knownRejected && !start.priorUnknown) && this.isAdmissionDisabled()) {
      this.update({phase: "localError", cause: new WritingDispatchAdmissionClosed("start", start.request.conversationId, start.request.manuscriptId, start.request.requestId)}); return;
    }
    this.startMayHaveDispatched = true;
    let result: AgentWritingStartResponse;
    try {
      result = decodeAgentWritingStartResponse(await this.gateway.startWriting(request));
      this.validateStarted(result, start.request, preview);
    } catch (cause: unknown) {
      if (this.disposed) return; const knownRejected = !start.priorUnknown && isAgentStartRejection(cause);
      this.update({phase: knownRejected ? "rejected" : "unknown", start: {...start, knownRejected, priorUnknown: !knownRejected,
        rejection: knownRejected ? startRejection(cause) : null}, cause});
      try {await this.persist();} catch (local: unknown) {this.update({phase: "localError", cause: local});} return;
    }
    if (!this.disposed) await this.finishStart(result);
  }
  private async inspectStart(): Promise<void> {
    const start = this.state.start, preview = this.state.result; if (!start || !preview || this.disposed) return;
    if (start.result) {await this.finishStart(start.result); return;}
    const priorCause = this.state.cause ?? start.rejection;
    this.update({phase: "checking", startStatus: null, cause: null});
    try {
      const response = decodeAgentWritingStartStatusResponse(await this.gateway.startStatus(structuredClone(start.request)));
      if (!sameAgentWritingStartRequest(response.intent, start.request)) throw new Error("WRITING_PREPARE_START_MISMATCH");
      if (this.disposed) return;
      if (response.status.type === "started") {this.validateStarted(response.status.result, start.request, preview); await this.finishStart(response.status.result);}
      else {
        if (response.status.type === "prepared" && response.status.preview.turnId !== preview.turnId) throw new Error("WRITING_PREPARE_START_MISMATCH");
        this.update({phase: response.status.type === "prepared" ? "ready" : response.status.type, startStatus: response.status,
          cause: start.knownRejected || !this.startMayHaveDispatched && priorCause instanceof WritingDispatchAdmissionClosed ? priorCause : null});
      }
    } catch (cause: unknown) {this.update({phase: "unknown", cause});}
  }
  private async finishStart(result: AgentWritingStartResponse): Promise<void> {
    const start = this.state.start, intent = this.state.intent; if (!start || !intent || this.disposed) return;
    if (this.clearing) {await this.clear(); return;}
    this.update({phase: "clearing", start: {...start, result, knownRejected: false}, cause: null});
    try {
      await this.persist(); if (this.disposed) return;
      await this.observed?.(start.request, result); if (this.disposed) return;
      if (this.state.draftAcknowledgment) {
        if (!this.drafts) throw new Error("WRITING_PREPARE_DRAFT_ACK_UNAVAILABLE");
        await this.drafts.consume(this.state.draftAcknowledgment, scope(intent));
      }
      if (!this.disposed) await this.clear();
    } catch (cause: unknown) {this.update({phase: "localError", cause});}
  }
  dispose(): void {this.disposed = true;}
}
