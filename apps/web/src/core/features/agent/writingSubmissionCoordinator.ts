import type { AgentSubmissionPreview, AgentWritingPrepareRequest, AgentWritingQueuePreviewRequest, AgentWritingStartRequest, AgentWritingQueuedStartRequest, AgentWritingStartResponse, AgentTurnOutcome, AgentQueueAdmission, AgentWritingQueueScope, AgentWritingQueueCancelRequest, AgentWorkspaceBuffer } from "@research-assistant/core/agent";
import type { AgentSubmissionGateway, AgentTurnGateway, AgentInputQueuePreviewGateway } from "@research-assistant/data/agent";
import {decodeAgentWritingQueueCancelRequest} from "@research-assistant/core/agent";
import { isAgentStartRejection } from "./agentPreparationFailure";
import { WritingQueueCancellation, sameWritingQueueCancellationIntent, type WritingQueueCancellationRecovery } from "./writingQueueCancellation";
import { getAppStorage } from "../../runtime/appStorage";
import {WritingQueuePreparation} from "./writingQueuePreparation";
import {WritingQueuedStart, type WritingQueuedStartSnapshot} from "./writingQueuedStart";
import type {WritingQueuedStartQueuePort} from "./writingQueuedStartQueue";
import {WritingPreparation, type WritingPreparationSnapshot} from "./writingPreparation";
import type {WritingDraftAcknowledgmentPort} from "./writingDraftAcknowledgment";
import type {ConversationDraftAcknowledgmentToken} from "./conversationDraftStore";
import {WritingSendStopIntents, type WritingSendStopIntent} from "./writingSendStopIntent";
import {WritingQueueStopHolds, type WritingQueueStopHold} from "./writingQueueStopHold";
import type {WritingSubmissionController as WritingSubmissionControllerPort} from "@research-assistant/workbench-sdk/agent-writing";

import {initialWritingSubmissionSnapshot as initial, writingPreparationSubmissionPatch, writingQueuedStartSubmissionPatch, isOriginalWritingPreviewCurrent, writingSubmissionWorkspaceBuffers, type WritingSubmissionSnapshot, type WritingSubmissionAdmission} from "./writingSubmissionState";

/** Core retains this conversation coordinator; Native owns admitted execution.
 * Disposing prevents late UI changes; it never cancels a native run implicitly.
 */
export class WritingSubmissionController implements WritingSubmissionControllerPort {
  private snapshot: WritingSubmissionSnapshot = initial;
  private listeners = new Set<() => void>();
  private epoch = 0;
  private disposed = false;
  private recoveredAdmission: AgentQueueAdmission | null = null;
  private queued: { inputId: string; expectedRevision: number } | null = null;
  private readonly starting: WritingQueuedStart | null;
  private startQueue: WritingQueuedStartQueuePort | null = null;
  private admissionObserver: ((value: WritingSubmissionAdmission) => void) | undefined;
  private delivered: string | null = null;
  private readonly cancellation: WritingQueueCancellation | null;
  private readonly preparation: WritingQueuePreparation | null;
  private readonly writingPreparation: WritingPreparation;
  private readonly contextAcceptors = new Set<(requestId: string) => Promise<void>>();
  private contextAcceptanceRequired = false;
  private canceledPreparation: AgentWritingQueueCancelRequest | null = null;
  private acknowledgment: (() => void) | undefined;
  private readonly sendStopIntents: WritingSendStopIntents;
  private readonly queueStopHolds: WritingQueueStopHolds;
  private readonly recoveryScope: AgentWritingQueueScope | undefined;
  private queueStopHold: WritingQueueStopHold | null = null;
  private sendStopIntent: WritingSendStopIntent | null = null;
  private stopAttempted = false;
  private stoppingForSend: Promise<void> | null = null;
  private savingSendStop: Promise<void> | null = null;
  constructor(gateway: AgentSubmissionGateway, private readonly turns: AgentTurnGateway,
    private readonly queueGateway?: AgentInputQueuePreviewGateway, recovery?: WritingQueueCancellationRecovery,
    private readonly isAdmissionDisabled: () => boolean = () => false) {
    this.sendStopIntents = new WritingSendStopIntents(recovery?.storage ?? getAppStorage());
    this.queueStopHolds = new WritingQueueStopHolds(recovery?.storage ?? getAppStorage());
    this.recoveryScope = recovery?.scope;
    this.cancellation = queueGateway ? new WritingQueueCancellation(queueGateway, recovery?.storage ?? getAppStorage(), state => {
      if (state.intent) this.canceledPreparation = structuredClone(state.intent);
      if (this.starting?.unresolved()) {this.update({cancellation: state}); return;}
      if (state.phase === "ready") { this.queued = null; this.recoveredAdmission = null; this.acknowledgment = undefined; this.update({...initial, cancellation: state}); }
      else { const busy = ["persisting", "sending", "checking", "clearing"].includes(state.phase);
        this.update({phase: busy ? "canceling" : "error", error: busy ? null : "cancel", cancellation: state}); }
    }, recovery?.scope) : null;
    this.preparation = queueGateway ? new WritingQueuePreparation(queueGateway, recovery?.storage ?? getAppStorage(), state => {
      if (this.starting?.unresolved()) {this.update({preparation: state}); return;}
      if (state.phase === "retired") {this.queued = null; this.acknowledgment = undefined; this.update({...initial, preparation: state}); return;}
      if (state.phase === "complete") {this.update({preparation: state}); return;}
      const intent = state.intent;
      if (intent) this.queued = {inputId: intent.inputId, expectedRevision: intent.expectedRevision};
      const busy = ["persisting", "sending", "checking", "clearing", "ready"].includes(state.phase);
      this.update({phase: busy ? "preparing" : "error", error: busy ? null : "prepare", preparation: state,
        request: state.recovered ? null : intent?.submission ?? null});
    }, this.cancellation?.unresolved() ? undefined : recovery?.scope, this.isAdmissionDisabled) : null;
    this.starting = queueGateway ? new WritingQueuedStart(queueGateway, recovery?.storage ?? getAppStorage(), state => this.receiveStart(state),
      (intent, result) => this.observeStarted(intent, result), async intent => {
        await this.preparation?.retireStarted(intent, this.starting?.getSnapshot().draftAcknowledgment);
        const prepared = this.preparation?.getSnapshot();
        if (prepared?.intent?.submission.requestId === intent.start.requestId && this.preparation?.retainsLocator()) throw prepared.cause ?? new Error("QUEUE_START_PREPARE_CLEANUP_FAILED");
        await this.cancellation?.retireAfterStarted(intent);
      }, recovery?.scope, this.isAdmissionDisabled) : null;
    this.writingPreparation = new WritingPreparation(gateway, recovery?.storage ?? getAppStorage(), state => this.receiveWritingPreparation(state),
      this.starting?.unresolved() || this.cancellation?.unresolved() || this.preparation?.unresolved() ? undefined : recovery?.scope, this.isAdmissionDisabled);
    this.writingPreparation.configureStartObserver(async (request, result) => {
      await this.acceptContext(request.requestId);
      this.deliverStarted(request, result, !this.writingPreparation.getSnapshot().draftAcknowledgment);
    });
    if (recovery?.scope) {
      try {
        this.queueStopHold = this.queueStopHolds.read(recovery.scope);
        if (this.queueStopHold) this.update({heldQueueInputId: this.queueStopHold.inputId});
      } catch {this.update({queueStopHoldUnavailable: true});}
      try {
        this.sendStopIntent = this.sendStopIntents.read(recovery.scope);
        if (this.sendStopIntent) {
          this.update({cancelSendRequested: true});
          queueMicrotask(() => {void this.reconcileSendCancellation();});
        }
      } catch { this.update({sendStopIntentUnavailable: true}); }
    }
  }
  getSnapshot = (): WritingSubmissionSnapshot => this.snapshot;
  isOriginalPreviewCurrent(preview: AgentSubmissionPreview): boolean {
    return !this.disposed && isOriginalWritingPreviewCurrent(this.snapshot.preview, preview);
  }
  hasQueuedPreparation = (): boolean => this.queued !== null || !!this.cancellation?.unresolved() || !!this.preparation?.unresolved() || !!this.starting?.unresolved();
  hasQueuedStartRecovery = (): boolean => !!this.starting?.unresolved();
  hasWritingPreparation = (): boolean => this.writingPreparation.unresolved();
  configureDraftAcknowledgments(port: WritingDraftAcknowledgmentPort): void {
    this.writingPreparation.configureDraftAcknowledgments(port);
    this.preparation?.configureDraftAcknowledgments(port);
    this.starting?.configureDraftAcknowledgments(port);
  }
  configureContextAcceptance(accept: (requestId: string) => Promise<void>): () => void {
    this.contextAcceptanceRequired = true; this.contextAcceptors.add(accept);
    return () => {this.contextAcceptors.delete(accept);};
  }
  private async acceptContext(requestId: string): Promise<void> {
    const accept = [...this.contextAcceptors].at(-1);
    if (!accept && this.contextAcceptanceRequired) throw new Error("WRITING_CONTEXT_ACCEPTANCE_HOST_UNAVAILABLE");
    await accept?.(requestId);
  }
  private receiveWritingPreparation(state: WritingPreparationSnapshot): void {
    if (state.phase === "retired") this.acknowledgment = undefined;
    this.update(writingPreparationSubmissionPatch(state, this.snapshot));
    if (state.phase !== "retired" && state.start?.result) this.requestAcceptedStop();
  }
  configureQueuedStart(queue: WritingQueuedStartQueuePort, observer?: (value: WritingSubmissionAdmission) => void): void {
    this.startQueue = queue; if (observer) this.admissionObserver = observer;
  }
  private receiveStart(state: WritingQueuedStartSnapshot): void {
    this.update(writingQueuedStartSubmissionPatch(state));
    if (state.result) this.update({admission: state.result, outcome: this.snapshot.outcome ?? state.result.outcome});
    if (state.result) this.requestAcceptedStop();
    if (state.phase === "done") {
      if (this.cancellation?.unresolved()) this.update({phase: "error", error: "cancel", cancellation: this.cancellation.getSnapshot()});
      else if (this.preparation?.unresolved()) this.update({phase: "error", error: "prepare", preparation: this.preparation.getSnapshot()});
    }
  }
  private async observeStarted(intent: AgentWritingQueuedStartRequest, result: AgentWritingStartResponse): Promise<void> {
    await this.acceptContext(intent.start.requestId);
    this.deliverStarted(intent.start, result, !this.starting?.getSnapshot().draftAcknowledgment);
  }
  private deliverStarted(start: AgentWritingStartRequest, result: AgentWritingStartResponse, legacyAcknowledgment: boolean): void {
    if (this.disposed) return;
    this.queued = null;
    this.update({phase: "started", admission: result, outcome: this.snapshot.outcome ?? result.outcome});
    const identity = JSON.stringify([start.conversationId, start.manuscriptId, start.requestId, start.contextSnapshotId, result.turnId]);
    if (this.admissionObserver && this.delivered !== identity) {
      this.admissionObserver({scope: {conversationId: start.conversationId, manuscriptId: start.manuscriptId,
        requestId: start.requestId, contextSnapshotId: start.contextSnapshotId}, turnId: result.turnId}); this.delivered = identity;
    }
    const acknowledgment = this.acknowledgment; this.acknowledgment = undefined; if (legacyAcknowledgment) acknowledgment?.();
  }
  async checkQueuedStart(observer?: (value: WritingSubmissionAdmission) => void): Promise<void> {
    if (observer) this.admissionObserver = observer; await this.starting?.check();
  }
  async retryQueuedStart(allowResume = false, observer?: (value: WritingSubmissionAdmission) => void): Promise<void> {
    if (observer) this.admissionObserver = observer;
    const fallback: WritingQueuedStartQueuePort = {read: async intent => {
      const current = await this.queueGateway?.recover({conversationId: intent.start.conversationId, manuscriptId: intent.start.manuscriptId});
      if (!current || current.inputId !== intent.inputId || current.preview.requestId !== intent.start.requestId) throw new Error("QUEUE_START_QUEUE_READ_FAILED"); return current.queue;
    }, resume: async () => {throw new Error("QUEUE_START_QUEUE_RESUME_FAILED");}};
    await this.starting?.retry(this.startQueue ?? fallback, allowResume);
  }
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
  private update(patch: Partial<WritingSubmissionSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }
  private current(epoch: number): boolean { return !this.disposed && epoch === this.epoch; }
  /** Record the exact queue item before attempting its durable pause. */
  async holdQueueInput(scope: AgentWritingQueueScope, inputId: string): Promise<void> {
    if (this.disposed || this.recoveryScope && (scope.conversationId !== this.recoveryScope.conversationId || scope.manuscriptId !== this.recoveryScope.manuscriptId))
      throw new Error("WRITING_QUEUE_STOP_HOLD_SCOPE_MISMATCH");
    if (this.queueStopHold && this.queueStopHold.inputId !== inputId) throw new Error("WRITING_QUEUE_STOP_HOLD_CONFLICT");
    const hold = {conversationId:scope.conversationId,manuscriptId:scope.manuscriptId,inputId};
    this.queueStopHold = hold;
    this.update({heldQueueInputId:inputId});
    try {await this.queueStopHolds.save(hold);}
    catch (cause) {if (!this.disposed) this.update({queueStopHoldUnavailable:true});throw cause;}
  }
  /** Call only after an authoritative read proves the item paused or absent. */
  async clearQueueInputHold(scope: AgentWritingQueueScope, inputId: string): Promise<boolean> {
    const held = this.queueStopHold;
    if (!held || held.inputId !== inputId || held.conversationId !== scope.conversationId || held.manuscriptId !== scope.manuscriptId) return false;
    try {await this.queueStopHolds.clear(held);}
    catch (cause) {if (!this.disposed) this.update({queueStopHoldUnavailable:true});throw cause;}
    if (this.queueStopHold === held) {
      this.queueStopHold = null;
      if (!this.disposed) this.update({heldQueueInputId:null,queueStopHoldUnavailable:false});
    }
    return true;
  }
  /** Retry only the original hold's local persistence; never fabricate a pause. */
  async retryQueueStopHold(scope: AgentWritingQueueScope): Promise<boolean> {
    if (this.disposed || this.recoveryScope && (scope.conversationId !== this.recoveryScope.conversationId || scope.manuscriptId !== this.recoveryScope.manuscriptId)) return false;
    try {
      const held = this.queueStopHold ?? this.queueStopHolds.read(scope);
      if (held) await this.queueStopHolds.save(held);
      this.queueStopHold = held;
      if (!this.disposed) this.update({heldQueueInputId: held?.inputId ?? null, queueStopHoldUnavailable: false});
      return true;
    } catch {if (!this.disposed) this.update({queueStopHoldUnavailable:true});return false;}
  }
  /** A cancellation is attached to the original persisted request; true means
   * the intent was recorded, not that an in-flight native operation stopped. */
  async cancelPendingSend(expectedRequestId: string): Promise<boolean> {
    if (this.disposed) return false;
    const request = this.snapshot.writingPreparation?.intent ?? this.snapshot.preparation?.intent?.submission ?? this.snapshot.request;
    const preview = this.snapshot.preview;
    const requestId = request?.requestId ?? preview?.requestId ?? this.snapshot.startRecovery?.intent?.start.requestId;
    const conversationId = request?.conversationId ?? preview?.conversationId ?? this.snapshot.startRecovery?.intent?.start.conversationId;
    const manuscriptId = request?.context.manuscriptId ?? preview?.manuscriptId ?? this.snapshot.startRecovery?.intent?.start.manuscriptId;
    if (!requestId || requestId !== expectedRequestId || !conversationId || !manuscriptId) return false;
    const intent = {conversationId, manuscriptId, requestId};
    if (this.sendStopIntent && this.sendStopIntent.requestId !== requestId) return false;
    if (this.sendStopIntent && this.snapshot.sendStopFailed && !this.snapshot.stopPending) this.stopAttempted = false;
    this.sendStopIntent = intent;
    this.update({cancelSendRequested: true});
    const saving = this.sendStopIntents.save(intent);
    this.savingSendStop = saving;
    try {await saving;}
    catch (cause) {this.update({sendStopIntentUnavailable:true}); throw cause;}
    finally {
      if (this.savingSendStop === saving) this.savingSendStop = null;
      this.requestAcceptedStop();
    }
    if (!this.disposed) await this.reconcileSendCancellation();
    return true;
  }
  isCancelPending(requestId?: string): boolean {
    return !!this.sendStopIntent && (requestId === undefined || this.sendStopIntent.requestId === requestId);
  }
  async retrySendStopIntent(scope: AgentWritingQueueScope): Promise<boolean> {
    if (this.disposed || this.recoveryScope && (scope.conversationId !== this.recoveryScope.conversationId || scope.manuscriptId !== this.recoveryScope.manuscriptId)) return false;
    try {
      const intent = this.sendStopIntent ?? this.sendStopIntents.read(scope);
      if (!intent || intent.conversationId !== scope.conversationId || intent.manuscriptId !== scope.manuscriptId) return false;
      this.sendStopIntent = intent; await this.sendStopIntents.save(intent);
      if (!this.disposed) this.update({cancelSendRequested:true,sendStopIntentUnavailable:false});
      if (this.snapshot.sendStopFailed && !this.snapshot.stopPending) this.stopAttempted = false;
      await this.reconcileSendCancellation();
      return true;
    } catch {if (!this.disposed) this.update({sendStopIntentUnavailable:true});return false;}
  }
  private async clearSendCancellation(intent: WritingSendStopIntent): Promise<void> {
    if (this.sendStopIntent !== intent) return;
    try {await this.sendStopIntents.clear(intent);}
    catch {if (!this.disposed) this.update({sendStopIntentUnavailable:true}); return;}
    if (this.sendStopIntent === intent) {
      this.sendStopIntent = null; this.stopAttempted = false;
      if (!this.disposed) this.update({cancelSendRequested: false,sendStopIntentUnavailable:false,sendStopFailed:false,sendStopNeedsReview:false});
    }
  }
  private requestAcceptedStop(): void {
    const intent = this.sendStopIntent, state = this.snapshot;
    if (this.savingSendStop) return;
    if (!intent || this.disposed || this.stopAttempted || !state.admission || state.phase !== "started"
      || state.admission.requestId !== intent.requestId || state.admission.conversationId !== intent.conversationId) return;
    this.stopAttempted = true;
    if (state.outcome) {void this.clearSendCancellation(intent); return;}
    this.update({sendStopFailed:false});
    const work = this.stop().then(async () => {
      if (!this.disposed && (this.snapshot.stopRequested || this.snapshot.outcome)) await this.clearSendCancellation(intent);
      else if (!this.disposed) this.update({sendStopFailed:true});
    });
    this.stoppingForSend = work;
    void work.finally(() => {if (this.stoppingForSend === work) this.stoppingForSend = null;});
  }
  private async advanceSendCancellation(): Promise<void> {
    const intent = this.sendStopIntent;
    if (!intent || this.disposed) return;
    if (this.savingSendStop) {try {await this.savingSendStop;} catch {return;}}
    if (this.sendStopIntent !== intent || this.disposed) return;
    if (this.snapshot.admission?.requestId === intent.requestId && (this.snapshot.stopRequested || this.snapshot.outcome)) {
      await this.clearSendCancellation(intent); return;
    }
    this.requestAcceptedStop();
    if (this.stoppingForSend) {await this.stoppingForSend; return;}
    await this.retireUnattemptedQueuedStart(intent.requestId);
    const state = this.snapshot;
    if (state.admission?.requestId === intent.requestId) return;
    const ordinary = this.writingPreparation.getSnapshot();
    if (ordinary.intent?.requestId === intent.requestId
      && (!ordinary.start && ordinary.status?.type === "notFound" || ordinary.start && ordinary.startStatus?.type === "prepared")) {
      await this.writingPreparation.retireVerifiedUnstarted();
      if (this.writingPreparation.getSnapshot().phase === "retired") {await this.clearSendCancellation(intent); return;}
    }
    if (ordinary.intent?.requestId === intent.requestId && !ordinary.start) {
      if (ordinary.knownCommitted && ["preview", "error"].includes(state.phase)) {
        if (await this.abandonWritingPreparation() !== null) await this.clearSendCancellation(intent);
      } else if (ordinary.knownRejected && !ordinary.priorUnknown) {
        await this.retireWritingPreparation();
        if (this.writingPreparation.getSnapshot().phase === "retired") await this.clearSendCancellation(intent);
      }
    } else if (ordinary.intent?.requestId === intent.requestId && ordinary.start?.knownRejected && !ordinary.start.priorUnknown) {
      if (await this.abandonWritingPreparation() !== null) await this.clearSendCancellation(intent);
    } else if (this.queued && state.preview?.requestId === intent.requestId && (state.phase === "preview" || state.phase === "starting") && !this.starting?.unresolved()) {
      if (state.phase === "starting") this.update({phase: "preview"});
      await this.cancelQueued(crypto.randomUUID());
      if (this.cancellation?.getSnapshot().phase === "ready") await this.clearSendCancellation(intent);
    }
  }
  /** Only inspect the original operation. A missing receipt must never cause a resend. */
  private async reconcileSendCancellation(): Promise<void> {
    const intent = this.sendStopIntent;
    if (!intent || this.disposed) return;
    this.update({sendStopNeedsReview:false});
    const start = this.starting?.getSnapshot(), ordinary = this.writingPreparation.getSnapshot(), queued = this.preparation?.getSnapshot(), cancel = this.cancellation?.getSnapshot();
    if (cancel?.intent?.previewRequestId === intent.requestId && ["unknown", "localError", "notFound", "unconfirmed"].includes(cancel.phase)) {
      await this.checkQueuedCancellation();
      if (this.cancellation?.getSnapshot().phase === "ready") await this.clearSendCancellation(intent);
    } else if (start?.intent?.start.requestId === intent.requestId && !start.result && ["unknown", "localError", "notFound", "unconfirmed"].includes(start.phase)) {
      await this.starting?.check();
    } else if (ordinary.intent?.requestId === intent.requestId && ["unknown", "localError", "notFound", "unconfirmed"].includes(ordinary.phase)) {
      await this.writingPreparation.check();
    } else if (queued?.intent?.submission.requestId === intent.requestId && ["unknown", "localError", "notFound", "unconfirmed"].includes(queued.phase)) {
      await this.preparation?.check();
      await this.installPreparedCurrent(this.epoch);
    }
    if (!this.disposed) {
      await this.advanceSendCancellation();
      const unresolved = [this.starting?.getSnapshot().phase, this.writingPreparation.getSnapshot().phase,
        this.preparation?.getSnapshot().phase, this.cancellation?.getSnapshot().phase];
      if (this.isCancelPending(intent.requestId) && unresolved.some(phase => phase && ["unknown", "localError", "notFound", "unconfirmed", "rejected"].includes(phase)))
        this.update({sendStopNeedsReview:true});
    }
  }
  async prepare(request: AgentWritingPrepareRequest, acknowledgment?: () => void, workspaceBuffers?: readonly AgentWorkspaceBuffer[], draftAcknowledgment?: ConversationDraftAcknowledgmentToken | null): Promise<void> {
    if (this.disposed || this.isCancelPending() || this.snapshot.sendStopIntentUnavailable || this.snapshot.phase !== "idle") return;
    if (this.starting?.restore({conversationId: request.conversationId, manuscriptId: request.context.manuscriptId})) return;
    if (this.cancellation?.restore({conversationId: request.conversationId, manuscriptId: request.context.manuscriptId})) return;
    if (this.preparation?.restore({conversationId: request.conversationId, manuscriptId: request.context.manuscriptId})) return;
    if (!this.queued && this.writingPreparation.restore({conversationId: request.conversationId, manuscriptId: request.context.manuscriptId})) return;
    // Freeze the composer/file list before the first async boundary.
    const frozen = structuredClone(request);
    this.acknowledgment = acknowledgment;
    this.pendingDraftAcknowledgment = draftAcknowledgment;
    const epoch = ++this.epoch;
    this.update({ ...initial, phase: "preparing", request: frozen });
    if (!this.queued) this.pendingWorkspaceBuffers = workspaceBuffers;
    try {await this.loadPreview(epoch, false); await this.reconcileSendCancellation();} finally {if (!this.queued) this.pendingWorkspaceBuffers = undefined; this.pendingDraftAcknowledgment = undefined;}
  }
  async prepareQueued(request: AgentWritingPrepareRequest, inputId: string, expectedRevision: number, acknowledgment?: () => void, workspaceBuffers?: readonly AgentWorkspaceBuffer[], draftAcknowledgment?: ConversationDraftAcknowledgmentToken | null): Promise<void> {
    if (this.disposed || this.snapshot.queueStopHoldUnavailable || this.queueStopHold?.inputId === inputId
      || this.isCancelPending() || this.snapshot.sendStopIntentUnavailable || !this.queueGateway || this.snapshot.phase !== "idle") return;
    this.queued = { inputId, expectedRevision };
    this.pendingWorkspaceBuffers = workspaceBuffers;
    try {await this.prepare(request, acknowledgment, undefined, draftAcknowledgment);} finally {this.pendingWorkspaceBuffers = undefined;}
  }
  private pendingWorkspaceBuffers: readonly AgentWorkspaceBuffer[] | undefined;
  private pendingDraftAcknowledgment: ConversationDraftAcknowledgmentToken | null | undefined;
  getWorkspaceBuffers(requestId: string): readonly AgentWorkspaceBuffer[] | null {
    return writingSubmissionWorkspaceBuffers(requestId, this.preparation?.getSnapshot(), this.writingPreparation.getSnapshot());
  }
  reviewUnstartedAdmission(value: AgentQueueAdmission): void {
    if (this.disposed || !this.queueGateway || this.snapshot.phase !== "idle") return;
    this.recoveredAdmission = structuredClone(value);
    ++this.epoch;
    this.update({ ...initial, phase: "preview", preview: this.recoveredAdmission.preview });
  }
  async recoverQueued(scope: AgentWritingQueueScope): Promise<void> {
    if (this.starting?.restore(scope)) return;
    if (this.cancellation?.restore(scope)) return;
    if (this.preparation?.restore(scope)) return;
    if (this.disposed || !this.queueGateway || !(this.snapshot.phase === "idle"
      || (this.snapshot.phase === "error" && this.snapshot.error === "recover"))) return;
    const epoch = ++this.epoch;
    this.update({ ...initial, phase: "preparing" });
    try {
      const result = await this.queueGateway.recover({ ...scope });
      if (!this.current(epoch)) return;
      if (!result) { this.update(initial); return; }
      this.queued = { inputId: result.inputId, expectedRevision: result.queue.revision };
      this.update({ phase: "preview", preview: result.preview, queue: result.queue });
    } catch {
      if (this.current(epoch)) this.update({ phase: "error", error: "recover" });
    }
  }
  private async loadPreview(epoch: number, restore: boolean): Promise<void> {
    const request = this.snapshot.request;
    if (!request) return;
    try {
      if (this.queued) {
        if (!this.queueGateway) throw new Error("Queue gateway unavailable");
        // Recovery reads the frozen record without touching changed editor files.
        // Explicit retry still prepares the exact original request when no receipt exists.
        if (restore) await this.preparation?.check();
        else await this.preparation?.begin({...this.queued, submission: structuredClone(request)}, this.pendingWorkspaceBuffers, this.pendingDraftAcknowledgment);
        await this.installPreparedCurrent(epoch);
        return;
      }
      await this.writingPreparation.begin(request, this.pendingWorkspaceBuffers, this.pendingDraftAcknowledgment);
    } catch {
      if (this.current(epoch)) this.update({ phase: "error", error: "prepare" });
    }
  }
  private async installPreparedCurrent(epoch: number): Promise<void> {
    const state = this.preparation?.getSnapshot(), original = state?.result;
    if (!this.current(epoch) || !this.queueGateway || !state?.knownCommitted || !["ready", "complete"].includes(state.phase) || !original) return;
    try {
      const current = await this.queueGateway.recover({conversationId: original.preview.conversationId, manuscriptId: original.preview.manuscriptId});
      if (!this.current(epoch)) return;
      if (!current || current.inputId !== original.inputId || current.preview.requestId !== original.preview.requestId) {
        // A successful current read can prove that this old preparation no
        // longer owns a claim. It says nothing about a model task's outcome.
        if (this.preparation?.retainsLocator()) await this.preparation.retireClaim();
        else {this.queued = null; this.update({...initial, preparation: state});}
        return;
      }
      if (current.preview.contextSnapshotId !== original.preview.contextSnapshotId || current.queue.preparingInputId !== original.inputId)
        throw new Error("QUEUE_PREPARE_CLAIM_CHANGED");
      this.queued = {inputId: original.inputId, expectedRevision: current.queue.revision};
      this.update({phase: "preview", error: null, preview: original.preview, queue: current.queue, request: state.recovered ? null : state.intent?.submission ?? this.snapshot.request});
    } catch (cause: unknown) {
      if (this.current(epoch)) this.update({phase: "error", error: "prepare", preview: null, queue: null,
        preparation: {...state, cause}});
    }
  }
  /** Read the original queued preparation; ordinary preparations keep their
   * own full intent in this same submission owner. Neither path invents R. */
  async restoreQueuedPreparation(): Promise<void> {
    if (this.disposed || this.snapshot.phase !== "error" || this.snapshot.error !== "prepare") return;
    const epoch = ++this.epoch;
    if (this.preparation?.getSnapshot().intent) {await this.preparation.check(); await this.installPreparedCurrent(epoch);}
    else if (this.queued) await this.recoverQueuedLegacy(epoch);
  }
  async checkWritingPreparation(observer?: (value: WritingSubmissionAdmission) => void): Promise<void> {if (observer) this.admissionObserver = observer; if (!this.disposed) await this.writingPreparation.check();}
  async retryWritingPreparation(observer?: (value: WritingSubmissionAdmission) => void): Promise<void> {if (observer) this.admissionObserver = observer; if (!this.disposed) await this.writingPreparation.retry();}
  async retireWritingPreparation(): Promise<void> {if (!this.disposed) await this.writingPreparation.retire();}
  async abandonWritingPreparation(): Promise<string | null> {
    const state = this.snapshot, original = this.writingPreparation.getSnapshot();
    if (this.disposed || !this.writingPreparation.unresolved() || !original.knownCommitted || !original.intent || original.start && (!original.start.knownRejected || original.start.priorUnknown || original.start.result)
      || !(state.phase === "preview" || state.phase === "error" && (state.error === "workspace" || state.error === "prepare" || state.error === "startRejected"))) return null;
    ++this.epoch; await this.writingPreparation.abandonCommitted();
    return !this.disposed && this.writingPreparation.getSnapshot().phase === "retired" ? original.intent.text : null;
  }
  async retryQueuedPreparation(): Promise<void> {
    if (this.disposed || this.snapshot.phase !== "error" || this.snapshot.error !== "prepare") return;
    const epoch = ++this.epoch;
    if (this.preparation?.getSnapshot().intent) {await this.preparation.retry(); await this.installPreparedCurrent(epoch);}
    else if (this.queued) await this.recoverQueuedLegacy(epoch);
  }
  async retireQueuedPreparation(hold: (intent: AgentWritingQueuePreviewRequest, eligible: () => boolean) => Promise<void>): Promise<void> {
    if (this.disposed || this.snapshot.error !== "prepare") return;
    const epoch = ++this.epoch; await this.preparation?.retire(hold); await this.installPreparedCurrent(epoch);
  }
  private async recoverQueuedLegacy(epoch: number): Promise<void> {
    const request = this.snapshot.request; if (!request || !this.queueGateway || !this.queued) return;
    try {
      const current = await this.queueGateway.recover({conversationId: request.conversationId, manuscriptId: request.context.manuscriptId});
      if (!current || current.inputId !== this.queued.inputId || current.preview.requestId !== request.requestId) throw new Error("QUEUE_PREPARE_LEGACY_READ_ONLY");
      if (this.current(epoch)) this.update({phase: "preview", error: null, request: null, preview: current.preview, queue: current.queue});
    } catch {if (this.current(epoch)) this.update({phase: "error", error: "prepare"});}
  }
  /** Keep a failed editable-copy preparation recoverable across view changes. */
  async observeWorkspacePreparation(requestId: string, ready: boolean, cause?: unknown): Promise<boolean> {
    const admissionError = this.snapshot.error === "start" || this.snapshot.error === "startRejected" ? this.snapshot.error : null;
    if (this.disposed || this.snapshot.preview?.requestId !== requestId
      || !(this.snapshot.phase === "preview" || this.snapshot.error === "workspace" || this.snapshot.error === "start" || this.snapshot.error === "startRejected"
        || (this.snapshot.error === "prepare" && this.snapshot.preparation?.knownCommitted))) return false;
    if (ready) {
      await this.preparation?.finishWorkspace(requestId);
      await this.writingPreparation.finishWorkspace(requestId);
      if (this.disposed || this.snapshot.preview?.requestId !== requestId || this.preparation?.retainsLocator() && !this.preparation.canStart()) return false;
      if (this.writingPreparation.getSnapshot().intent?.requestId === requestId && !this.writingPreparation.canStart()) return false;
    }
    this.update({phase: ready && !admissionError ? "preview" : "error", error: admissionError ?? (ready ? null : "workspace"), workspaceFailure: ready ? null : cause}); return ready;
  }
  async confirm(mode: AgentWritingStartRequest["mode"], onAdmitted?: (value: WritingSubmissionAdmission) => void): Promise<void> {
    if (this.snapshot.sendStopIntentUnavailable) return;
    if (this.isCancelPending()) {await this.advanceSendCancellation(); return;}
    if (this.starting?.unresolved()) {await this.retryQueuedStart(false, onAdmitted); return;}
    if (onAdmitted) this.admissionObserver = onAdmitted;
    const originalStart = this.writingPreparation.getSnapshot().start;
    if (originalStart && this.writingPreparation.unresolved()) {await this.writingPreparation.confirmStart(originalStart.request); return;}
    const state = this.snapshot;
    if (this.disposed || this.preparation?.retainsLocator() && !this.preparation.canStart() || this.writingPreparation.retainsLocator() && !this.writingPreparation.canStart() || !state.preview || !(state.phase === "preview" || (state.phase === "error" && (state.error === "start" || (state.error === "startRejected" && this.queued))))) return;
    const preview = state.preview;
    const epoch = ++this.epoch;
    this.update({ phase: "starting", error: null });
    try {
      const start: AgentWritingStartRequest = { conversationId: preview.conversationId,
        requestId: preview.requestId, manuscriptId: preview.manuscriptId,
        contextSnapshotId: preview.contextSnapshotId, model: preview.input.model, mode };
      if (this.recoveredAdmission) {
        if (!this.queueGateway) throw new Error("Queue gateway unavailable");
        this.admissionObserver = onAdmitted;
        await this.starting?.begin({inputId: this.recoveredAdmission.inputId, expectedRevision: this.recoveredAdmission.expectedRevision, start}, preview.turnId, this.preparation?.getSnapshot().draftAcknowledgment); return;
      } else if (this.queued) {
        if (!this.queueGateway || !state.queue) throw new Error("Queue preview unavailable");
        const current = await this.queueGateway.recover({conversationId: preview.conversationId, manuscriptId: preview.manuscriptId});
        if (!this.current(epoch)) return;
        if (this.isCancelPending(preview.requestId)) {await this.advanceSendCancellation(); return;}
        if (!current || current.inputId !== this.queued.inputId || current.preview.requestId !== preview.requestId
          || current.preview.contextSnapshotId !== preview.contextSnapshotId || current.queue.preparingInputId !== this.queued.inputId) throw new Error("QUEUE_PREPARE_CLAIM_CHANGED");
        this.admissionObserver = onAdmitted;
        await this.starting?.begin({inputId: this.queued.inputId, expectedRevision: current.queue.revision, start}, preview.turnId, this.preparation?.getSnapshot().draftAcknowledgment); return;
      } else {
        await this.writingPreparation.confirmStart(start);
      }
    } catch (reason) {
      if (this.current(epoch)) this.update({ phase: "error", error: state.error !== "start" && isAgentStartRejection(reason) ? "startRejected" : "start" });
    } finally {await this.reconcileSendCancellation();}
  }
  /** Cancellation is persisted, retaining and pausing the queue head. Retry the
   * exact cancellation identity after a lost response; never locally dismiss it.
   */
  async beginPreparedQueueCancellation(value: AgentWritingQueueCancelRequest): Promise<void> {
    const intent = decodeAgentWritingQueueCancelRequest(value);
    const cancellation = this.cancellation, scope = this.recoveryScope, state = this.snapshot;
    if (this.disposed || !cancellation || !scope || scope.conversationId.toLowerCase() !== intent.conversationId
      || scope.manuscriptId.toLowerCase() !== intent.manuscriptId || this.isQueuedCancellationCommitted(intent)) return;
    const previous = cancellation.getSnapshot().intent;
    const matching = !!previous && sameWritingQueueCancellationIntent(previous, intent);
    if (this.hasWritingPreparation() || this.hasQueuedStartRecovery() || this.preparation?.unresolved()
      || this.isCancelPending() || state.stopPending || state.cancelSendRequested || state.sendStopIntentUnavailable
      || state.queueStopHoldUnavailable || state.heldQueueInputId || this.queued && !state.outcome
      || !(state.phase === "idle" || state.phase === "started" && state.outcome || matching)) return;
    ++this.epoch;
    await cancellation.begin(intent);
    await this.clearCanceledPreparation();
  }
  isQueuedCancellationCommitted(value: AgentWritingQueueCancelRequest): boolean {
    const state = this.cancellation?.getSnapshot();
    return !this.disposed && state?.phase === "ready" && state.knownCommitted && !!this.canceledPreparation
      && sameWritingQueueCancellationIntent(this.canceledPreparation, decodeAgentWritingQueueCancelRequest(value));
  }
  async cancelQueued(requestId: string): Promise<void> {
    const pendingStart = this.starting?.getSnapshot().intent;
    if (pendingStart && this.starting?.hasUnattemptedStart()) await this.retireUnattemptedQueuedStart(pendingStart.start.requestId);
    if (this.starting?.unresolved()) return;
    if (this.disposed || this.snapshot.phase === "canceling") return;
    if (this.cancellation?.unresolved()) { await this.cancellation.retry(); await this.clearCanceledPreparation(); return; }
    const state = this.snapshot;
    if (this.disposed || !this.queued || !this.queueGateway || !state.preview || !state.queue
      || !(state.phase === "preview" || (state.phase === "error" && (state.error === "cancel" || state.error === "workspace")))) return;
    const intent: AgentWritingQueueCancelRequest = { conversationId: state.preview.conversationId, manuscriptId: state.preview.manuscriptId,
      requestId, inputId: this.queued.inputId, expectedRevision: state.queue.revision, previewRequestId: state.preview.requestId };
    ++this.epoch; await this.cancellation?.begin(intent); await this.clearCanceledPreparation();
  }
  private async retireUnattemptedQueuedStart(requestId: string): Promise<void> {
    const pending = this.starting?.getSnapshot().intent;
    if (!pending || pending.start.requestId !== requestId || !this.starting?.hasUnattemptedStart() || !this.queueGateway || this.disposed) return;
    const current = await this.queueGateway.recover({conversationId: pending.start.conversationId, manuscriptId: pending.start.manuscriptId});
    if (this.disposed || !current || current.inputId !== pending.inputId || current.preview.requestId !== requestId
      || current.preview.contextSnapshotId !== pending.start.contextSnapshotId || current.queue.preparingInputId !== pending.inputId) return;
    if (!await this.starting.retireVerifiedUnstarted() || this.disposed) return;
    this.queued = {inputId: current.inputId, expectedRevision: current.queue.revision}; this.recoveredAdmission = null;
    this.update({phase: "preview", error: null, preview: current.preview, queue: current.queue});
  }
  private async clearCanceledPreparation(): Promise<void> {
    if (!this.disposed && this.cancellation?.getSnapshot().phase === "ready" && this.canceledPreparation)
      await this.preparation?.retireCanceled(this.canceledPreparation);
  }
  async checkQueuedCancellation(): Promise<void> {if (this.starting?.unresolved()) return; await this.cancellation?.check(); await this.clearCanceledPreparation();}
  async retryQueuedCancellation(readCurrent?: AgentInputQueuePreviewGateway["recover"]): Promise<void> {if (this.starting?.unresolved()) return; await this.cancellation?.retry(readCurrent); await this.clearCanceledPreparation();}
  /** Only durable history, never a stop acknowledgement, can finish this UI run. */
  observeOutcome(turnId: string, outcome: AgentTurnOutcome): void {
    if (this.disposed || (this.snapshot.admission?.turnId ?? this.snapshot.preview?.turnId) !== turnId) return;
    if (this.snapshot.phase === "starting" || this.snapshot.phase === "started"
      || (this.snapshot.phase === "error" && this.snapshot.error === "start")) this.update({ outcome, stopPending: false });
  }
  async stop(): Promise<void> {
    const state = this.snapshot;
    if (this.disposed || state.phase !== "started" || !state.admission || state.outcome || state.stopPending || state.stopRequested) return;
    const epoch = this.epoch;
    this.update({ stopPending: true, error: null });
    try {
      const response = await this.turns.interrupt({ conversationId: state.admission.conversationId, turnId: state.admission.turnId });
      if (this.current(epoch)) this.update({ stopPending: false, stopRequested: response.cancellationRequested });
    } catch {
      if (this.current(epoch)) this.update({ stopPending: false, error: "stop" });
    }
  }
  /** Return the frozen text for the host to restore without overwriting new input.
   * Ambiguous starts cannot be discarded: retry/reconcile their original receipt.
   */
  dismiss(): string | null {
    if (this.writingPreparation.unresolved() || this.starting?.unresolved() || this.preparation?.unresolved() || this.cancellation?.unresolved() || (this.queued && !this.snapshot.outcome) || this.disposed || this.snapshot.phase === "starting" || (this.snapshot.phase === "error" && this.snapshot.error === "start" && !this.snapshot.outcome) || (this.snapshot.phase === "started" && !this.snapshot.outcome)) return null;
    const text = this.snapshot.outcome === "completed" ? null : this.snapshot.request?.text ?? null;
    ++this.epoch;
    this.queued = null;
    this.recoveredAdmission = null;
    this.acknowledgment = undefined;
    this.update(initial);
    return text;
  }
  dispose(): void { this.disposed = true; this.writingPreparation.dispose(); this.starting?.dispose(); this.preparation?.dispose(); this.cancellation?.dispose(); ++this.epoch; this.acknowledgment = undefined; this.contextAcceptors.clear(); this.listeners.clear(); }

  /**
   * Detach a view without cancelling the submission owner.  A writing panel is
   * keyed by conversation, so switching conversations unmounts the view while
   * prepare/start requests may still be in flight.  The conversation registry
   * keeps this controller alive and a later view subscribes to its snapshot.
   */
  detach(): void { this.listeners.clear(); }
}
