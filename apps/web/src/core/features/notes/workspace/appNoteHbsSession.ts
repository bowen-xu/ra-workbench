import { createNoteDocumentAdapter, HbsSession, type DocumentCheckpoint, type DocumentPreservation } from "@research-assistant/core/hbs";
import {normalizeNoteTitle,pageEntryTargets, type NoteDocumentV1} from "@research-assistant/core/notes";
import { createDocumentPort, createDocumentPreservationPort } from "@research-assistant/data/hbs";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import { directTextDiff } from "../../../components/hbs/codeMirrorHbsPolicy";
import { HbsDeliveryPort } from "../../../components/hbs/deliveryPort";
import { getHbsDocumentGateway, getPendingHbsDocumentCommit, type AppHbsDocumentGateway } from "../../../services/hbsDocumentGateway";
import { EDITOR_AUTOSAVE_IDLE_MS } from "../../../services/editorAutosave";
import { subscribeTextPatches, textPatchGateway } from "../../agent/textPatchGateway";
import type { NoteExternalSaveLease } from "../editor/noteExternalSaveLease";
import type { NoteSaveCoordinator, NoteSaveStatus } from "../editor/noteSaveController";
import { registerNoteReviewSavePort, notifyNoteReviewSaveState, availableTextInstallPort, type NoteReviewSaveState } from "../editor/noteTextEditorPorts";
import type { NoteRecoveryDraft } from "./noteRecovery";

interface Options {
  readonly loaded: NoteDocumentSnapshot;
  readonly gateway?: AppHbsDocumentGateway;
  readonly recovery?: NoteRecoveryDraft;
  readonly allowRecovery?: boolean;
  readonly debounceMs?: number;
  readonly onSnapshot: (snapshot: NoteDocumentSnapshot) => void;
  readonly onDurable: (snapshot: NoteDocumentSnapshot) => void;
  readonly onStatus: (status: NoteSaveStatus, message?: string) => void;
  readonly onRelatedChange?: (ids:readonly string[])=>void;
}

/** One retained note owns one HBS session. This adapter supplies the production
 * idle-save policy and Agent admission; it has no independent document writer. */
export class AppNoteHbsSession implements NoteSaveCoordinator {
  readonly session: HbsSession<NoteDocumentV1>;
  /** First publication replaces provisional metadata without rebuilding the
   * live model, editor instance, selection or native undo history. */
  adoptCreatedMetadata(page: NoteDocumentSnapshot["page"]): void {
    if(page.id !== this.metadata.id)throw new Error("MODEL_IDENTITY_MISMATCH：新页面回执身份不匹配");
    if(page.revision >= this.metadata.revision)this.metadata = page;
  }
  private metadata: NoteDocumentSnapshot["page"];
  private timer: ReturnType<typeof setTimeout> | undefined;
  private unsubscribe: (() => void) | undefined;
  private unregisterReview: (() => void) | undefined;
  private unregisterPatches: (() => void) | undefined;
  private disposed = false;
  private lastContent: NoteDocumentV1;
  private lastBaseline: NoteDocumentV1;
  private lastVersion: number;
  private reviewState: NoteReviewSaveState = { status: "saved" };
  private blocked: string | undefined;
  private legacy: NoteRecoveryDraft | undefined;
  private external: { dispatched: boolean; invalidated: boolean; expected: number } | undefined;
  private reconcilingReceipt = false;
  private preservationFailure: string | undefined;
  private readonly preservationReadBlocked: boolean;
  private unknownResult: string | undefined;
  private activeRequestId: string | undefined;
  private recoveredRequest: ReturnType<typeof getPendingHbsDocumentCommit>;
  private retrying: Promise<void> | undefined;
  private preservedState: ReturnType<HbsSession<NoteDocumentV1>["getSnapshot"]> | undefined;
  private readonly gateway: AppHbsDocumentGateway;
  private lastAuthority:NoteDocumentV1;

  private constructor(private readonly options: Options, session: HbsSession<NoteDocumentV1>, private readonly preservation: DocumentPreservation<NoteDocumentV1>, private readonly delivery: HbsDeliveryPort<NoteDocumentV1>) {
    this.gateway = options.gateway ?? getHbsDocumentGateway();
    this.session = session;
    this.metadata = options.loaded.page;
    const state = session.getSnapshot();
    this.lastAuthority=state.observed.saved;
    this.preservationReadBlocked = !!state.preservationError;
    this.lastContent = state.content; this.lastBaseline = state.baselineProjection; this.lastVersion = state.observed.version;
    this.legacy = options.recovery;
    if (options.recovery?.review?.request) this.blocked = `MODEL_LEGACY_REQUEST_UNSETTLED：原请求 ${options.recovery.review.request.requestId} 尚未核对，请显式重试。`;
    else if (options.recovery && options.recovery.baseRevision !== options.loaded.page.revision && !options.recovery.review)
      this.blocked = "MODEL_LEGACY_BASE_MISSING：旧恢复草稿缺少可重建的原始版本，请比较后决定保存。";
    else {
      const pending = getPendingHbsDocumentCommit("note", options.loaded.page.id);
      if (pending) { this.activeRequestId = pending.id; this.recoveredRequest = pending; }
    }
  }

  static async open(options: Options): Promise<AppNoteHbsSession> {
    const gateway = options.gateway ?? getHbsDocumentGateway(), id = options.loaded.page.id;
    const adapter = createNoteDocumentAdapter(directTextDiff);
    const durable = createDocumentPreservationPort(adapter, (operation, value) => operation === "readPreservation"
      ? gateway.readPreservation("note", id) : gateway.savePreservation("note", id, value));
    let writes = Promise.resolve();
    const preservation: DocumentPreservation<NoteDocumentV1> = { load: () => durable.load(), save: value => {
      const write = writes.then(() => durable.save(value)); writes = write.catch(() => {}); return write;
    } };
    let owner: AppNoteHbsSession | undefined;
    const port = createDocumentPort(adapter, (operation, request) => {
      if (operation === "read") return gateway.read("note", id);
      const command = request as Parameters<AppHbsDocumentGateway["commit"]>[2];
      if (owner) { owner.activeRequestId = command.id; owner.changed(); }
      return gateway.commit("note", id, command);
    });
    // Transport admission is independent of local H/B′ editing and S′ writes.
    // Reuse delivery scheduling: new commands wait until the immutable old
    // outbox request has a definite reply. Opening never retries that request.
    const delivery = new HbsDeliveryPort(port);
    if (getPendingHbsDocumentCommit("note", id) || options.recovery?.review?.request) delivery.setAutomatic(false);
    // An explicitly resolved whole-document comparison starts from that result.
    // Legacy material is imported only if the new durable S′ slot is empty.
    const initial: DocumentPreservation<NoteDocumentV1> = { save: value => preservation.save(value), load: async () => {
      if (options.allowRecovery === false) return undefined;
      const stored = await preservation.load();
      if (stored || !options.recovery) return stored;
      const recovery = options.recovery, baseline = recovery.review?.baseline ?? options.loaded.document;
      const projected = adapter.projectManual(baseline, options.loaded.document, recovery.document);
      return { format: 1, kind: adapter.kind, observed: { version: recovery.baseRevision, saved: options.loaded.document, baseline },
        ancestor: options.loaded.document,
        // Legacy review recovery has no separate provenance field. Retain its
        // explicit unconfirmed baseline conservatively rather than silently
        // classifying a previously saved human candidate as untouched input.
        reviewAncestor: recovery.review ? baseline : options.loaded.document, baselineAncestor: baseline,
        content: recovery.document, ordinary: adapter.diff(baseline, projected) };
    } };
    const instance = new AppNoteHbsSession(options, await HbsSession.open({ adapter, port: delivery, preservation: initial,
      requestId: sequence => `${crypto.randomUUID()}:${sequence}` }), preservation, delivery);
    owner = instance;
    // Migration is acknowledged only after the real S′ port has durably
    // accepted the restored projection. The old recovery slot stays on error.
    if (options.recovery && !instance.session.getSnapshot().preservationError) await instance.ensurePreserved();
    return instance;
  }

  /** Install ownership before callbacks are published, including in StrictMode. */
  start(): void {
    if (this.disposed || this.unsubscribe) return;
    this.unsubscribe = this.session.subscribe(() => this.changed());
    this.unregisterReview = registerNoteReviewSavePort(this.metadata.id, {
      flush: async () => { await this.flush(); if (this.hasUnsavedChanges()) throw new Error(this.reviewState.message ?? "MODEL_UNRESOLVED：当前输入已保留，请处理保存状态。"); },
      retry: () => this.retry(), getState: () => this.reviewState,
      synchronize: () => this.synchronize(),
      inspect: () => this.session.getSnapshot(),
      acknowledgeReview:requestId=>this.acknowledgeReview(requestId),
      ...(this.legacy?.review ? { recovery: () => this.legacy?.review } : {}),
    });
    this.unregisterPatches = subscribeTextPatches(() => { void this.synchronize().catch(reason => this.session.record(`MODEL_SYNC_FAILED：${String(reason)}`)); });
    this.changed();
    if (this.metadata.status === "active" && this.isDirty() && !this.blocked) this.schedule();
  }

  private snapshot(): NoteDocumentSnapshot {
    const state = this.session.getSnapshot();
    return { page: { ...this.metadata, ...(state.content.title === undefined ? {} : {title: normalizeNoteTitle(state.content.title)}), revision: state.observed.version }, document: state.content };
  }
  private isDirty(): boolean {
    const state = this.session.getSnapshot(), adapter = this.session.adapter;
    return !adapter.equal(state.content, state.observed.saved) || !adapter.equal(state.baselineProjection, state.observed.baseline);
  }
  private changed(): void {
    if (this.disposed) return;
    const state = this.session.getSnapshot();
    if (state.message.startsWith("MODEL_RESULT_UNKNOWN")) this.unknownResult = state.message;
    if (!state.pending) { this.unknownResult = undefined; if (!this.recoveredRequest) this.activeRequestId = undefined; }
    const input = this.lastContent !== state.content || this.lastBaseline !== state.baselineProjection;
    const version = this.lastVersion !== state.observed.version;
    const previousEntries=version?pageEntryTargets(this.lastAuthority):undefined;
    this.lastAuthority=state.observed.saved;
    if (input && this.external && !version && !this.reconcilingReceipt) this.external.invalidated = true;
    this.lastContent = state.content; this.lastBaseline = state.baselineProjection; this.lastVersion = state.observed.version;
    this.options.onSnapshot(this.snapshot());
    if(previousEntries){
      const entries=pageEntryTargets(state.observed.saved);
      const related=[...new Set([...previousEntries,...entries])].filter(id=>previousEntries.has(id)!==entries.has(id));
      // Notify from authoritative S only, never from a cut/paste still in H.
      // Existing metadata readers refresh retained targets without replacing H.
      if(related.length)this.options.onRelatedChange?.(related);
    }
    // Unknown requests require their exact retry, including after reopening.
    // Calling them conflicts disables that action and sends users to a document
    // comparison which cannot acknowledge the original request.
    const status: NoteSaveStatus = this.blocked || this.recoveredRequest || this.unknownResult || this.preservationFailure || state.preservationError || /^(MODEL_SYNC_FAILED|MODEL_EDITOR_INSTALL_FAILED)/u.test(state.message) ? "failed"
      : state.conflicts.length ? "conflict"
      : state.pending || this.external ? "saving" : this.isDirty() ? "dirty" : "saved";
    const message = this.blocked ?? (this.recoveredRequest ? `MODEL_REQUEST_UNSETTLED：原正文保存请求 ${this.recoveredRequest.id} 尚未核对；本地输入仍可编辑和保全，请显式重试。` : undefined) ?? this.unknownResult ?? this.preservationFailure ?? state.preservationError
      ?? (status === "failed" || status === "conflict" ? state.message : state.pending
        ? this.activeRequestId ? `MODEL_DOCUMENT_SAVING：正文保存请求 ${this.activeRequestId} 等待回执；后续输入另行保全。` : "MODEL_DOCUMENT_SAVING：正在保全输入并准备正文保存。" : undefined);
    if (this.reviewState.status !== status || this.reviewState.message !== message || this.reviewState.pendingChanges !== state.changes.length) {
      this.reviewState = { status, pendingChanges: state.changes.length, ...(message ? { message } : {}) };
      this.options.onStatus(status, message); notifyNoteReviewSaveState();
    }
    if (status === "saved") this.options.onDurable(this.snapshot());
    if (this.metadata.status === "active" && input && this.isDirty() && !this.blocked && !this.external && !state.pending) this.schedule();
  }
  private schedule(): void {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => { this.timer = undefined; void this.flush().catch(() => {}); }, this.options.debounceMs ?? EDITOR_AUTOSAVE_IDLE_MS);
  }
  enqueue(document: NoteDocumentV1): void { if (!this.disposed && !this.session.adapter.equal(this.session.getSnapshot().content, document)) this.session.edit(document); }
  async ensurePreserved(): Promise<void> {
    if (this.preservationReadBlocked) throw new Error(this.session.getSnapshot().preservationError ?? "MODEL_RECOVERY_INVALID：原保全记录无法读取，已保留原记录。");
    await this.session.flushPreservation();
    const state = this.session.getSnapshot();
    const previous = this.preservedState;
    if (this.metadata.status !== "active" && !this.preservationFailure && previous && previous.observed === state.observed && previous.content === state.content
      && previous.ancestor === state.ancestor && previous.reviewAncestor === state.reviewAncestor
      && previous.baselineAncestor === state.baselineAncestor && previous.baselineProjection === state.baselineProjection) return;
    const checkpoint: DocumentCheckpoint<NoteDocumentV1> = { format: 1, kind: this.session.adapter.kind,
      observed: state.observed, ancestor: state.ancestor, reviewAncestor: state.reviewAncestor, baselineAncestor: state.baselineAncestor,
      content: state.content, ordinary: this.session.adapter.diff(state.baselineAncestor, state.baselineProjection) };
    try { await this.preservation.save(checkpoint); this.preservedState = state; this.preservationFailure = undefined; }
    catch (reason) { this.preservationFailure = `MODEL_PRESERVATION_FAILED：${reason instanceof Error ? reason.message : String(reason)}`; this.changed(); throw reason; }
    const latest = this.session.getSnapshot();
    if (latest.content !== state.content || latest.baselineProjection !== state.baselineProjection) await this.ensurePreserved();
  }
  async flush(): Promise<void> {
    if (this.disposed) return;
    clearTimeout(this.timer); this.timer = undefined;
    // Already acknowledged, unchanged views only wait for outstanding S′ I/O.
    await this.session.flushPreservation();
    if (!this.hasUnsavedChanges()) return;
    await this.ensurePreserved();
    // Lifecycle status is an application write boundary, not a review decision.
    if (this.metadata.status !== "active") return;
    if (this.blocked || this.recoveredRequest || this.external) return;
    // Unknown outcomes are retried only via the explicit retry action. Closing
    // and autosaving must not dispatch a new/old unresolved command implicitly.
    if (this.unknownResult) return;
    await this.session.save();
    if (this.session.getSnapshot().pending && !this.unknownResult) await new Promise<void>(resolve => {
      const stop = this.session.subscribe(() => { const state = this.session.getSnapshot();
        if (!state.pending || this.unknownResult) { stop(); resolve(); }
      });
    });
    await this.session.flushPreservation();
    this.changed();
  }
  hasUnsavedChanges(): boolean {
    const state = this.session.getSnapshot();
    return !!this.blocked || !!this.recoveredRequest || !!this.external || !!this.preservationFailure || !!state.preservationError || state.pending || this.isDirty() || state.conflicts.length > 0;
  }
  hasExternalChange(): boolean { return !!this.external; }
  isEditingBlocked(): boolean { return !!this.blocked; }
  requiresRequestRetry(): boolean { return !!this.recoveredRequest || !!this.unknownResult || !!this.legacy?.review?.request; }
  restore(document: NoteDocumentV1, status: "dirty" | "conflict"): void {
    this.enqueue(document);
    if (status === "conflict") { this.blocked = "MODEL_LEGACY_BASE_MISSING：已保留恢复内容，请比较保存版本。"; this.changed(); }
  }
  private acceptMetadata(page: NoteDocumentSnapshot["page"]): boolean {
    if (page.id !== this.metadata.id) throw new Error("MODEL_IDENTITY_MISMATCH：笔记回执身份不匹配。");
    if (this.disposed || page.revision < Math.max(this.metadata.revision, this.session.getSnapshot().observed.version)) return false;
    this.metadata = page;
    return true;
  }
  /** Metadata receipts update this owner's page fields before HBS can publish
   * another document snapshot. They never edit H, B or review decisions. */
  updateMetadata(page: NoteDocumentSnapshot["page"]): void {
    if (this.acceptMetadata(page)) this.updateRevision(page.revision);
  }
  updateRevision(revision: number): void {
    if (revision > this.session.getSnapshot().observed.version) void this.synchronize().catch(reason => {
      this.session.record(`MODEL_SYNC_FAILED：${reason instanceof Error ? reason.message : String(reason)}`);
    });
  }
  private async synchronize(receipt = false): Promise<void> {
    const snapshot = await createDocumentPort(this.session.adapter, () => this.gateway.read("note", this.metadata.id)).read();
    this.reconcilingReceipt = receipt;
    try { this.session.receive(snapshot, true, !this.blocked && !this.unknownResult); }
    finally { this.reconcilingReceipt = false; }
    if (this.session.getSnapshot().pending && !this.unknownResult && !this.recoveredRequest) await new Promise<void>(resolve => {
      const stop = this.session.subscribe(() => { const state = this.session.getSnapshot();
        if (!state.pending || this.unknownResult) { stop(); resolve(); }
      });
    });
    await this.ensurePreserved();
  }
  async synchronizeExternal(snapshot: NoteDocumentSnapshot, _baseline?: NoteDocumentV1): Promise<void> {
    if (this.disposed) return;
    this.acceptMetadata(snapshot.page);
    await this.synchronize(true);
    // S′ may finish after this view was replaced. The page's current install
    // port belongs to the new owner and must never receive this owner's H.
    if (this.disposed) return;
    if (this.session.getSnapshot().observed.version < snapshot.page.revision) throw new Error("MODEL_STALE_RECEIPT：权威笔记尚未包含此回执。");
    const port = availableTextInstallPort(this.metadata.id);
    if (!port) return; // The retained view installs current H when it mounts.
    const content = this.session.getSnapshot().content;
    if (!this.session.adapter.equal(port.read(), content)) port.prepareStructure(port.read(), content, undefined, { addToHistory: false })();
  }
  retry(): Promise<void> {
    if (this.retrying) return this.retrying;
    const operation = this.retryOriginalAndSave();
    this.retrying = operation;
    void operation.finally(() => { if (this.retrying === operation) this.retrying = undefined; }).catch(() => {});
    return operation;
  }
  private acknowledgeReview(requestId:string):void {
    if(this.disposed||this.legacy?.review?.request?.requestId!==requestId)return;
    this.legacy=undefined;this.blocked=undefined;this.delivery.setAutomatic(true);this.changed();
  }
  private async retryOriginalAndSave(): Promise<void> {
    if (this.disposed || this.external) return;
    if (this.legacy?.review?.request) {
      const request=this.legacy.review.request;
      await textPatchGateway.resolve(request);
      if (this.disposed) return;
      this.acknowledgeReview(request.requestId);
      await this.synchronize();
    } else if (this.recoveredRequest) {
      const request = this.recoveredRequest;
      await this.gateway.commit("note", this.metadata.id, request);
      if (this.disposed) return;
      if (getPendingHbsDocumentCommit("note", this.metadata.id)?.id === request.id)
        throw new Error(`MODEL_REQUEST_UNSETTLED：原请求 ${request.id} 仍未得到明确回执。`);
      this.recoveredRequest = undefined;
      this.delivery.setAutomatic(true);
      await this.synchronize();
    } else if (this.blocked) { this.changed(); return; }
    if (this.disposed) return;
    await this.session.save();
    if (this.disposed) return;
    await this.ensurePreserved(); this.changed();
  }
  dispose(): void {
    this.disposed = true; clearTimeout(this.timer); this.unsubscribe?.(); this.unregisterReview?.(); this.unregisterPatches?.(); this.session.dispose(); this.delivery.dispose();
  }
  beginExternalChange(expectedRevision: number, baseline: NoteDocumentV1): NoteExternalSaveLease | null {
    if (this.disposed || this.hasUnsavedChanges() || expectedRevision !== this.session.getSnapshot().observed.version
      || !this.session.adapter.equal(baseline, this.session.getSnapshot().content)) return null;
    const owner = { dispatched: false, invalidated: false, expected: expectedRevision }; this.external = owner; this.changed();
    const current = () => !this.disposed && this.external === owner;
    const requireRecovery = () => { if (current()) { this.blocked = "MODEL_EXTERNAL_UNSETTLED：外部修改尚未核对，请恢复原操作。"; this.changed(); } };
    const commit = (snapshot: NoteDocumentSnapshot, revision: number, install: () => undefined): boolean => {
      if (!current() || !owner.dispatched || owner.invalidated || snapshot.page.id !== this.metadata.id
        || revision <= owner.expected || snapshot.page.revision !== revision || this.session.getSnapshot().observed.version !== revision) return false;
      try { if (install() !== undefined) { requireRecovery(); return false; } } catch (reason) { requireRecovery(); throw reason; }
      this.external = undefined; this.blocked = undefined; this.metadata = snapshot.page; this.changed(); return true;
    };
    return { markDispatched: () => { if (!current() || owner.dispatched || owner.invalidated) return false; owner.dispatched = true; return true; },
      commit: (snapshot, install) => commit(snapshot, expectedRevision + 1, install), commitGroup: commit, requireRecovery,
      cancelBeforeDispatch: () => { if (!current() || owner.dispatched) return false; this.external = undefined; this.changed(); return true; },
      rejectDispatched: () => { if (!current() || owner.invalidated) return false; this.external = undefined; this.blocked = undefined; this.changed(); return true; } };
  }
}
