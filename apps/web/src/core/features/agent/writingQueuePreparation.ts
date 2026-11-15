import {decodeAgentWritingQueuePreviewRequest, decodeAgentWritingQueuePrepareResult, decodeAgentWritingQueuePrepareStatusResponse,
  sameAgentWritingQueuePreviewRequest, decodeAgentSqlImmediateFailure, decodeAgentWorkspaceBuffer, type AgentWorkspaceBuffer, type AgentWritingQueuePreviewRequest, type AgentWritingQueuePreviewResponse,
  type AgentWritingQueuePrepareStatusResponse, type AgentWritingQueueScope, type AgentWritingQueueCancelRequest, type AgentWritingQueuedStartRequest} from "@research-assistant/core/agent";
import type {AgentInputQueuePreviewGateway} from "@research-assistant/data/agent";
import {flushStorage, storagePersistence} from "../../runtime/storagePersistence";
import {WritingDispatchAdmissionClosed} from "./writingDispatchAdmission";
import {writingQueueCancellationDomain} from "./writingQueueCancellation";
import {decodeWritingDraftAcknowledgment, type WritingDraftAcknowledgmentPort} from "./writingDraftAcknowledgment";
import type {ConversationDraftAcknowledgmentToken} from "./conversationDraftStore";

export interface WritingQueuePreparationSnapshot {
  readonly phase: "idle" | "persisting" | "sending" | "unknown" | "checking" | "notFound" | "unconfirmed" | "clearing" | "localError" | "ready" | "complete" | "retired";
  readonly intent: AgentWritingQueuePreviewRequest | null;
  readonly status: AgentWritingQueuePrepareStatusResponse["status"] | null;
  readonly result: AgentWritingQueuePreviewResponse | null;
  readonly knownCommitted: boolean;
  readonly recovered: boolean;
  readonly definitiveRejection: boolean;
  readonly workspaceBuffers: readonly AgentWorkspaceBuffer[] | null;
  readonly draftAcknowledgment: ConversationDraftAcknowledgmentToken | null;
  readonly workspaceReady: boolean;
  readonly cause: unknown;
}
const initial = (): WritingQueuePreparationSnapshot => ({phase: "idle", intent: null, status: null, result: null, knownCommitted: false, recovered: false, definitiveRejection: false, workspaceBuffers: null, draftAcknowledgment: null, workspaceReady: false, cause: null});
const scopeOf = (intent: AgentWritingQueuePreviewRequest) => ({conversationId: intent.submission.conversationId, manuscriptId: intent.submission.context.manuscriptId});
const key = (scope: AgentWritingQueueScope) => `research-assistant:writing-queue-prepare:v1:${scope.manuscriptId.toLowerCase()}:${scope.conversationId.toLowerCase()}`;
function rejectionEvidence(cause: unknown, intent: AgentWritingQueuePreviewRequest) {
  const row = cause && typeof cause === "object" && "code" in cause && cause.code === "storage" && "reason" in cause
    && cause.reason && typeof cause.reason === "object" ? cause.reason as Record<string, unknown> : null;
  if (!row || row.conversationId !== intent.submission.conversationId || row.manuscriptId !== intent.submission.context.manuscriptId
    || row.inputId !== intent.inputId || row.previewRequestId !== intent.submission.requestId || row.operationId !== intent.submission.requestId) return null;
  const domain = writingQueueCancellationDomain(cause), failure = decodeAgentSqlImmediateFailure(cause), detail = failure?.diagnostic;
  const model = detail?.phase === "queue-prepare-model-selection" && ["model_settings_unavailable", "model_provider_disabled", "model_credentials_unavailable",
    "chatgpt_transport_unavailable", "model_protocol_unsupported", "model_preparation_worker_unavailable", "model_transport_unavailable"].includes(detail.safeCause ?? "");
  const materialization = [
    ["DATA_SQL_OPERATION_CONFLICT", "queue-prepare-context", "writing_context_changed"],
    ["DATA_SQL_OPERATION_CONFLICT", "queue-prepare-input", "writing_input_changed"],
    ["DATA_SQL_OPERATION_CONFLICT", "queue-prepare-instructions", "writing_instructions_unauthorized"],
    ["DATA_SQL_INVALID", "queue-prepare-context", "writing_context_invalid"],
    ["DATA_SQL_INVALID", "queue-prepare-context", "writing_context_budget_exceeded"],
    ["DATA_SQL_INVALID", "queue-prepare-instructions", "writing_instructions_scope_invalid"],
    ["DATA_SQL_INVALID", "queue-prepare-instructions", "writing_instructions_budget_exceeded"],
    ["DATA_SQL_INVALID", "queue-prepare-instructions", "writing_instructions_encoding_invalid"],
    ["DATA_SQL_INVALID", "queue-prepare-input", "writing_input_invalid"],
  ].some(([code, phase, safeCause]) => !!detail && failure?.code === code && detail.phase === phase && detail.safeCause === safeCause);
  if (!domain && !(detail?.operationId === intent.submission.requestId && (materialization || (failure?.code === "DATA_SQL_UNAVAILABLE" && model)))) return null;
  return {code: "storage", reason: {conversationId: intent.submission.conversationId, manuscriptId: intent.submission.context.manuscriptId,
    inputId: intent.inputId, previewRequestId: intent.submission.requestId, operationId: intent.submission.requestId,
    code: domain ? "DATA_SQL_DOMAIN" : failure!.code, reason: domain ? domain.safeReason : detail}};
}
/** Durable locator substate of WritingSubmissionController. It owns no model,
 * queue scheduler, admission or second controller registry. */
export class WritingQueuePreparation {
  private state = initial();
  private reserved = false;
  private clearing = false;
  private retiring = false;
  private finishingWorkspace = false;
  private attempted = false;
  private freshAdmissionRequired = false;
  private mayHaveUnknownDispatch = false;
  private disposed = false;
  private pending: Promise<void> | null = null;
  private drafts: WritingDraftAcknowledgmentPort | undefined;
  constructor(private readonly gateway: AgentInputQueuePreviewGateway, private readonly storage: Storage,
    private readonly changed: (state: WritingQueuePreparationSnapshot) => void, scope?: AgentWritingQueueScope,
    private readonly isAdmissionDisabled: () => boolean = () => false) {
    if (scope) this.restore(scope);
  }
  getSnapshot = () => this.state;
  configureDraftAcknowledgments(port?: WritingDraftAcknowledgmentPort): void {this.drafts = port;}
  unresolved = () => !["idle", "complete", "retired"].includes(this.state.phase);
  retainsLocator = () => this.reserved || this.clearing;
  canStart = () => this.state.knownCommitted && ((this.state.phase === "ready" && this.state.workspaceReady)
    || (!this.state.draftAcknowledgment && this.state.phase === "complete"));
  private update(patch: Partial<WritingQueuePreparationSnapshot>): void {
    if (this.disposed) return;
    this.state = {...this.state, ...patch}; this.changed(this.state);
  }
  private workspace(intent: AgentWritingQueuePreviewRequest, value: unknown): readonly AgentWorkspaceBuffer[] | null {
    if (value === undefined || value === null) return intent.submission.context.files.some(file => file.buffer != null) ? null : [];
    if (!Array.isArray(value)) throw new Error("QUEUE_PREPARE_RECOVERY_INVALID");
    const buffers = value.map(decodeAgentWorkspaceBuffer);
    if (new Set(buffers.map(buffer => buffer.path)).size !== buffers.length || intent.submission.context.files.some(file => file.buffer && !buffers.some(buffer =>
      buffer.path === file.relativePath && buffer.revision === file.buffer!.revision && buffer.text === file.buffer!.text && buffer.expectedDiskSha256 === file.expectedDiskSha256))) throw new Error("QUEUE_PREPARE_RECOVERY_INVALID");
    return buffers;
  }
  private read(scope: AgentWritingQueueScope): {intent: AgentWritingQueuePreviewRequest; definitiveRejection: boolean; cause: unknown; knownCommitted: boolean; workspaceBuffers: readonly AgentWorkspaceBuffer[] | null; draftAcknowledgment: ConversationDraftAcknowledgmentToken | null; workspaceReady: boolean} | null {
    const raw = this.storage.getItem(key(scope)); if (raw === null) return null;
    try {
      const value: unknown = JSON.parse(raw);
      if (!value || typeof value !== "object" || Object.keys(value).some(key => !["schemaVersion", "intent", "definitiveRejection", "rejectionCause", "knownCommitted", "workspaceBuffers", "draftAcknowledgment", "workspaceReady"].includes(key))
        || !("schemaVersion" in value) || (value.schemaVersion !== 1 && value.schemaVersion !== 2) || !("intent" in value)
        || (value.schemaVersion === 2 && (!("draftAcknowledgment" in value) || !("workspaceReady" in value) || typeof value.workspaceReady !== "boolean"))
        || (value.schemaVersion === 1 && (("draftAcknowledgment" in value && value.draftAcknowledgment !== null) || ("workspaceReady" in value && value.workspaceReady !== false)))
        || ("definitiveRejection" in value && typeof value.definitiveRejection !== "boolean") || ("knownCommitted" in value && typeof value.knownCommitted !== "boolean")) throw new Error();
      const intent = decodeAgentWritingQueuePreviewRequest(value.intent), own = scopeOf(intent);
      if (own.conversationId !== scope.conversationId.toLowerCase() || own.manuscriptId !== scope.manuscriptId.toLowerCase()) throw new Error();
      const cause = "rejectionCause" in value && value.rejectionCause != null ? rejectionEvidence(value.rejectionCause, intent) : null;
      if ("definitiveRejection" in value && value.definitiveRejection === true && !cause) throw new Error();
      return {intent, definitiveRejection: cause !== null, cause, knownCommitted: "knownCommitted" in value && value.knownCommitted === true,
        draftAcknowledgment: decodeWritingDraftAcknowledgment(value.schemaVersion === 2 && "draftAcknowledgment" in value ? value.draftAcknowledgment : null, own),
        workspaceReady: value.schemaVersion === 2 && "workspaceReady" in value && value.workspaceReady === true,
        workspaceBuffers: this.workspace(intent, "workspaceBuffers" in value ? value.workspaceBuffers : undefined)};
    } catch {throw new Error("QUEUE_PREPARE_RECOVERY_INVALID");}
  }
  restore(scope: AgentWritingQueueScope): boolean {
    if (this.disposed || this.unresolved()) return true;
    try {
      const saved = this.read(scope);
      if (saved) {
        this.reserved = true; this.attempted = true; this.freshAdmissionRequired = false; this.mayHaveUnknownDispatch = true;
        this.retiring = false; this.finishingWorkspace = false; this.clearing = false;
        this.update({...initial(), phase: "unknown", ...saved, workspaceReady: false, recovered: true});
      }
    } catch (cause: unknown) {this.update({...initial(), phase: "localError", cause, recovered: true});}
    return this.unresolved();
  }
  private async persist(intent: AgentWritingQueuePreviewRequest): Promise<void> {
    const saved = this.read(scopeOf(intent));
    if ((this.reserved && !saved) || (saved && (!sameAgentWritingQueuePreviewRequest(saved.intent, intent)
      || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)))) throw new Error("QUEUE_PREPARE_RECOVERY_CONFLICT");
    this.storage.setItem(key(scopeOf(intent)), JSON.stringify({schemaVersion: 2, intent, draftAcknowledgment: this.state.draftAcknowledgment, definitiveRejection: this.state.definitiveRejection,
      rejectionCause: this.state.definitiveRejection ? rejectionEvidence(this.state.cause, intent) ?? saved?.cause ?? null : null,
      knownCommitted: this.state.knownCommitted, workspaceBuffers: this.state.workspaceBuffers, workspaceReady: this.state.workspaceReady})); this.reserved = true;
    await flushStorage(this.storage);
  }
  private async clear(retired = false): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return;
    this.update({phase: "clearing", cause: null});
    try {
      const saved = this.read(scopeOf(intent));
      if ((!saved && !this.clearing) || (saved && (!sameAgentWritingQueuePreviewRequest(saved.intent, intent)
        || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)))) throw new Error("QUEUE_PREPARE_RECOVERY_CONFLICT");
      this.storage.removeItem(key(scopeOf(intent))); this.clearing = true; await flushStorage(this.storage);
      this.reserved = false; this.clearing = false; this.update({phase: retired ? "retired" : "complete", cause: null});
    } catch (cause: unknown) {this.update({phase: "localError", cause});}
  }
  private async retained(action: () => Promise<void>): Promise<void> {
    if (this.disposed) return;
    if (this.pending) return this.pending;
    const pending = action(); this.pending = pending;
    try {await pending;} finally {if (this.pending === pending) this.pending = null;}
  }
  begin(value: AgentWritingQueuePreviewRequest, workspaceBuffers?: readonly AgentWorkspaceBuffer[], draftAcknowledgment?: ConversationDraftAcknowledgmentToken | null): Promise<void> {
    return this.retained(async () => {
      if (this.unresolved()) return;
      const intent = decodeAgentWritingQueuePreviewRequest(value);
      if (this.restore(scopeOf(intent))) return;
      this.attempted = false; this.freshAdmissionRequired = false; this.mayHaveUnknownDispatch = false;
      this.retiring = false; this.finishingWorkspace = false;
      this.update({...initial(), phase: "persisting", intent, workspaceBuffers: this.workspace(intent, workspaceBuffers),
        draftAcknowledgment: decodeWritingDraftAcknowledgment(draftAcknowledgment, scopeOf(intent))}); await this.send();
    });
  }
  private async send(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed || this.state.knownCommitted) return;
    this.update({phase: "persisting", cause: null});
    try {await this.persist(intent); if (this.disposed) return;
      if (this.state.draftAcknowledgment) {if (!this.drafts) throw new Error("QUEUE_PREPARE_DRAFT_ACKNOWLEDGMENT_UNAVAILABLE"); await this.drafts.flush();}
    } catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return;
    const request = structuredClone(intent);
    this.update({phase: "sending"});
    if (this.disposed) return;
    if ((!this.attempted || this.freshAdmissionRequired) && this.isAdmissionDisabled()) {
      this.update({phase: "localError", cause: new WritingDispatchAdmissionClosed("prepare", intent.submission.conversationId, intent.submission.context.manuscriptId, intent.submission.requestId)}); return;
    }
    this.attempted = true; this.freshAdmissionRequired = false;
    try {
      const result = decodeAgentWritingQueuePrepareResult(await this.gateway.prepare(request), intent);
      if (this.disposed) return;
      this.update({result, knownCommitted: true}); await this.retainCommitted();
    } catch (cause: unknown) {
      if (this.disposed) return;
      const rejected = !!rejectionEvidence(cause, intent);
      this.mayHaveUnknownDispatch ||= !rejected;
      this.freshAdmissionRequired = rejected && !this.mayHaveUnknownDispatch;
      this.update({phase: "unknown", cause, definitiveRejection: this.state.definitiveRejection || rejected});
      if (rejected) try {await this.persist(intent);} catch (local: unknown) {this.update({phase: "localError", cause: local});}
    }
  }
  private async inspect(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return;
    this.update({recovered: true});
    if (this.retiring) {await this.clear(true); return;}
    if (this.state.knownCommitted && this.state.result) {
      if (this.finishingWorkspace || this.retiring) await this.clear(this.retiring);
      else if (!["ready", "complete"].includes(this.state.phase)) await this.retainCommitted();
      return;
    }
    const priorCause = this.state.cause;
    this.update({phase: "checking", cause: null, recovered: true});
    try {
      const response = decodeAgentWritingQueuePrepareStatusResponse(await this.gateway.prepareStatus(structuredClone(intent)));
      if (!sameAgentWritingQueuePreviewRequest(response.intent, intent)) throw new Error("QUEUE_PREPARE_PROOF_MISMATCH");
      if (this.disposed) return;
      this.update({status: response.status, phase: response.status.type === "committed" ? "clearing" : response.status.type,
        knownCommitted: this.state.knownCommitted || response.status.type === "committed", result: response.status.type === "committed" ? response.status.result : null,
        cause: response.status.type === "notFound" ? priorCause : null});
      if (response.status.type === "committed") await this.retainCommitted();
    } catch (cause: unknown) {this.update({phase: "unknown", cause});}
  }
  check = (): Promise<void> => this.retained(() => this.inspect());
  retry = (): Promise<void> => this.retained(async () => {
    if (!this.state.intent) return;
    try {if (storagePersistence(this.storage)?.error) await storagePersistence(this.storage)!.retry();
      if (this.state.draftAcknowledgment && this.drafts) await this.drafts.retry();}
    catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return;
    if (this.retiring) {await this.clear(true); return;}
    if (this.state.knownCommitted) {await this.inspect(); return;}
    this.update({recovered: true});
    if (!this.attempted) {await this.send(); return;}
    await this.inspect();
    if (!this.state.knownCommitted && (this.state.phase === "notFound" || this.state.phase === "unconfirmed")) await this.send();
  });
  private async retainCommitted(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return;
    try {await this.persist(intent); this.update({phase: "ready", cause: null});}
    catch (cause: unknown) {this.update({phase: "localError", cause});}
  }
  finishWorkspace(requestId: string): Promise<void> {
    if (this.disposed) return Promise.resolve();
    if (this.pending) return this.pending.then(() => this.finishWorkspace(requestId));
    return this.retained(async () => {
      if (this.state.intent?.submission.requestId !== requestId || !this.state.knownCommitted || this.state.phase === "complete") return;
      this.update({workspaceReady: true, phase: this.state.draftAcknowledgment ? "persisting" : "clearing", cause: null});
      if (this.state.draftAcknowledgment) {this.finishingWorkspace = false; await this.retainCommitted(); return;}
      this.finishingWorkspace = true; await this.clear();
    });
  }
  retireClaim(): Promise<void> {
    return this.retained(async () => {if (this.retainsLocator()) {this.retiring = true; await this.clear(true);}});
  }
  /** A proved cancellation may retire only its own local preparation metadata.
   * Read it silently: an unresolved cancellation keeps presentation authority. */
  retireCanceled(cancel: AgentWritingQueueCancelRequest): Promise<void> {
    return this.retireMatching({conversationId: cancel.conversationId, manuscriptId: cancel.manuscriptId, inputId: cancel.inputId, requestId: cancel.previewRequestId});
  }
  retireStarted(start: AgentWritingQueuedStartRequest, draftAcknowledgment?: ConversationDraftAcknowledgmentToken | null): Promise<void> {
    return this.retireMatching({conversationId: start.start.conversationId, manuscriptId: start.start.manuscriptId,
      inputId: start.inputId, requestId: start.start.requestId}, draftAcknowledgment === undefined ? this.state.draftAcknowledgment
      : decodeWritingDraftAcknowledgment(draftAcknowledgment, {conversationId: start.start.conversationId, manuscriptId: start.start.manuscriptId}));
  }
  private retireMatching(target: AgentWritingQueueScope & {readonly inputId: string; readonly requestId: string}, draftAcknowledgment = this.state.draftAcknowledgment): Promise<void> {
    if (this.disposed) return Promise.resolve();
    if (this.pending) return this.pending.then(() => this.retireMatching(target, draftAcknowledgment));
    return this.retained(async () => {
      const matches = (intent: AgentWritingQueuePreviewRequest) => intent.inputId === target.inputId
        && intent.submission.conversationId === target.conversationId && intent.submission.context.manuscriptId === target.manuscriptId
        && intent.submission.requestId === target.requestId;
      try {
        const saved = this.read(target);
        if (saved && !matches(saved.intent)) return;
        if (this.retainsLocator() && this.state.intent && !matches(this.state.intent)) return;
        if (saved && JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(draftAcknowledgment)) throw new Error("QUEUE_PREPARE_RECOVERY_CONFLICT");
        if (this.retainsLocator() && JSON.stringify(this.state.draftAcknowledgment) !== JSON.stringify(draftAcknowledgment)) throw new Error("QUEUE_PREPARE_RECOVERY_CONFLICT");
        if (saved) {
          this.state = {...initial(), phase: "unknown", ...saved, recovered: true};
          this.reserved = true; this.attempted = true;
        } else if (!(this.clearing && this.state.intent && matches(this.state.intent))) return;
        this.retiring = true; await this.clear(true);
      } catch (cause: unknown) {this.update({phase: "localError", cause});}
    });
  }
  retire = (hold: (intent: AgentWritingQueuePreviewRequest, eligible: () => boolean) => Promise<void>): Promise<void> => this.retained(async () => {
    const intent = this.state.intent; if (!intent || this.attempted && !this.state.definitiveRejection || this.disposed) return;
    try {if (storagePersistence(this.storage)?.error) await storagePersistence(this.storage)!.retry();}
    catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return;
    if (this.retiring) {await this.clear(true); return;}
    await this.inspect();
    if (this.disposed || this.state.status?.type !== "notFound" || this.state.knownCommitted) return;
    try {
      await this.persist(intent); if (this.disposed) return;
      const current = await this.gateway.recover(scopeOf(intent)); if (this.disposed) return;
      if (current?.preview.requestId === intent.submission.requestId) throw new Error("QUEUE_PREPARE_CLAIM_CHANGED");
      await hold(structuredClone(intent), () => !this.disposed && !!this.state.intent && sameAgentWritingQueuePreviewRequest(intent, this.state.intent)); if (this.disposed) return;
      this.retiring = true; await this.clear(true);
    } catch (cause: unknown) {this.update({phase: "localError", cause});}
  });
  dispose(): void {this.disposed = true;}
}
