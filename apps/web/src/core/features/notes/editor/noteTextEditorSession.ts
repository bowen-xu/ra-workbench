import { listen, type UnlistenFn } from "../../../runtime/platformEvents";
import { invoke, isDesktopRuntime } from "../../../runtime/platformCore";
import { getAppRuntime, isNativeAgentPreview } from "../../../runtime/appRuntime";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import { decodeNoteDocument, type NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteSaveCoordinator } from "./noteSaveController";
import type { NoteExternalSaveLease } from "./noteExternalSaveLease";
import { holdTextEditor, textInstallPort, checkTextSnapshotReady, registerTextReceiptListener, flushNoteReview, synchronizeNoteReview, type AppliedNotePage } from "./noteTextEditorPorts";
import { sameNoteProjection } from "./noteReviewProjection";
import { noteReviewErrorMessage } from "./noteReviewRecovery";

interface Flush { readonly requestId: string; readonly sessionId: string; readonly pageId: string }
interface Release { readonly requestId: string; readonly pages: readonly AppliedNotePage[] }
interface Receipt { readonly pages: readonly AppliedNotePage[] }
function same(a: unknown, b: unknown): boolean {
  const canonical = (value: unknown) => JSON.stringify(value, (_key, item: unknown) => item && typeof item === "object" && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([x], [y]) => x.localeCompare(y))) : item);
  return canonical(a) === canonical(b);
}
/** Bridges the existing save owner; owns neither autosave nor note persistence. */
export interface NoteTextSessionOptions {
  readonly controller: NoteSaveCoordinator;
  readonly gateway: NotesGateway;
  readonly getSnapshot: () => NoteDocumentSnapshot | undefined;
  readonly onError: (message: string) => void;
  readonly onChanged: (revision: number) => void;
}
export interface NoteTextSessionGateway { register(options: NoteTextSessionOptions): () => void }
export function registerNoteTextSession(options: NoteTextSessionOptions): () => void {
  const runtime = getAppRuntime();
  if (runtime.mode === "preview" && !runtime.nativeAgentPreview) return runtime.gateways.noteTextSessionGateway?.register(options) ?? (() => {});
  if (!isDesktopRuntime() && !isNativeAgentPreview()) return () => {};
  const sessionId = crypto.randomUUID(), pageId = options.getSnapshot()?.page.id;
  if (!pageId) return () => {};
  let disposed = false;
  const stops: UnlistenFn[] = [];
  const leases = new Map<string, { readonly lease: NoteExternalSaveLease; readonly baseline: NoteDocumentSnapshot; readonly editorDocument: NoteDocumentV1 }>();
  let recovery: { readonly requestId: string; readonly lease: NoteExternalSaveLease; readonly baseline: NoteDocumentSnapshot;
    readonly editorDocument: NoteDocumentV1; readonly dispatched: boolean } | undefined;
  let releasing = false;
  let receiptTail = Promise.resolve();
  const receiveInOrder = (operation: () => Promise<void>): Promise<void> => {
    const work = receiptTail.then(async () => {
      if (disposed) return;
      try { await operation(); }
      catch (reason) { if (!disposed) options.onError(noteReviewErrorMessage(reason)); }
    });
    receiptTail = work;
    return work;
  };
  const expired = new Set<string>();
  const release = async ({ requestId, pages }: Release): Promise<void> => {
    expired.add(requestId);
    const pending = leases.get(requestId); if (!pending) return;
    leases.delete(requestId);
    releasing = true; let dispatched = false;
    try {
      const receipt = pages.find(page => page.pageId === pageId);
      if (!receipt) { pending.lease.cancelBeforeDispatch(); return; }
      if (receipt.revision === pending.baseline.page.revision && same(decodeNoteDocument(receipt.document), pending.baseline.document)) {
        pending.lease.cancelBeforeDispatch(); return;
      }
      if (!pending.lease.markDispatched()) throw new Error("笔记会话已变化，已保留当前输入。"); dispatched = true;
      const snapshot = await options.gateway.loadDocument(pageId);
      if (disposed || snapshot.page.revision !== receipt.revision || !same(snapshot.document, decodeNoteDocument(receipt.document)))
        throw new Error("正文在修改后再次发生变化，请核对原操作结果。");
      await options.controller.synchronizeExternal?.(snapshot, receipt.review?.baseline);
      if (disposed) return;
      const install = options.controller.synchronizeExternal ? () => undefined : prepareReceiptInstall(pageId, pending.editorDocument, receipt, snapshot.document);
      if (!pending.lease.commitGroup(snapshot, receipt.revision, install)) throw new Error("修改已保存，但编辑器安装失败；请保留输入并重新加载核对。");
      if (receipt.review) await synchronizeNoteReview(pageId);
      recovery = undefined; options.onChanged(receipt.revision);
    } catch (reason) { recovery = { requestId, ...pending, dispatched }; pending.lease.requireRecovery(); options.onError(noteReviewErrorMessage(reason)); }
    finally { releasing = false; holdTextEditor(pageId, requestId, false); }
  };
  const applyReceipt = async (pages: readonly AppliedNotePage[]): Promise<void> => {
    if (disposed || releasing) return;
    const receipt = pages.find(page => page.pageId === pageId); if (!receipt) return;
    const active = leases.entries().next().value;
    if (active) { await release({ requestId: active[0], pages }); return; }
    const baseline = recovery?.baseline ?? options.getSnapshot();
    if (!baseline || (!recovery && receipt.revision <= baseline.page.revision)) return;
    // A receipt without its original fence may only install over the still-
    // formal projection or confirm the exact result already visible. Do not
    // adopt an unrelated later candidate/input as an inferred admission.
    const editorDocument = recovery?.editorDocument ?? textInstallPort(pageId).read();
    if (!recovery && receipt.review?.origin !== "manual" && !sameNoteProjection(editorDocument, baseline.document)
      && !sameNoteProjection(editorDocument, decodeNoteDocument(receipt.document))) {
      options.onError("原操作已有结果，当前输入已保留；请恢复原操作或核对保存版本，不能覆盖后来输入。"); return;
    }
    const lease = recovery?.lease ?? options.controller.beginExternalChange(baseline.page.revision, baseline.document);
    if (!lease) { options.onError("原操作已有结果，当前输入已保留；请保存或核对冲突后恢复编辑器。"); return; }
    const requestId = recovery?.requestId ?? crypto.randomUUID();
    // A typing receipt adopts S only. Its existing lease still checks CAS,
    // but locking contenteditable would interrupt newer typing/composition.
    const installsDocument = receipt.review?.origin !== "manual";
    if (installsDocument) holdTextEditor(pageId, requestId, true);
    try {
      if (!recovery?.dispatched && !lease.markDispatched()) throw new Error("笔记会话已变化，已保留当前输入。");
      const server = await options.gateway.loadDocument(pageId);
      if (disposed || server.page.revision !== receipt.revision || !same(server.document, decodeNoteDocument(receipt.document))) throw new Error("原操作后又出现了修改，请保留输入并核对数据库版本。");
      await options.controller.synchronizeExternal?.(server, receipt.review?.baseline);
      if (disposed) return;
      if (!lease.commitGroup(server, receipt.revision, options.controller.synchronizeExternal ? () => undefined : prepareReceiptInstall(pageId, editorDocument, receipt, server.document))) throw new Error("编辑器恢复未完成，已保留输入。");
      if (installsDocument && receipt.review) await synchronizeNoteReview(pageId);
      recovery = undefined; options.onChanged(receipt.revision);
    } catch (reason) { recovery = { requestId, lease, baseline, editorDocument, dispatched: true }; lease.requireRecovery(); options.onError(noteReviewErrorMessage(reason)); }
    finally { if (installsDocument) holdTextEditor(pageId, requestId, false); }
  };
  const stopReceipts = registerTextReceiptListener(pages => receiveInOrder(() => applyReceipt(pages)));
  void (async () => {
    const flushStop = await listen<Flush>("research-note-text-flush", event => {
      const { requestId, sessionId: target } = event.payload;
      if (disposed || target !== sessionId) return;
      void (async () => {
        let error: string | null = null;
        try {
          checkTextSnapshotReady(pageId);
          holdTextEditor(pageId, requestId, true);
          await flushNoteReview(pageId);
          await options.controller.flush();
          const baseline = options.getSnapshot();
          if (disposed || expired.has(requestId) || !baseline || options.controller.hasUnsavedChanges()) throw new Error("笔记尚未保存或操作已过期；已保留输入并停止读取。");
          checkTextSnapshotReady(pageId);
          const editorDocument = textInstallPort(pageId).read();
          const lease = options.controller.beginExternalChange(baseline.page.revision, baseline.document);
          if (!lease) throw new Error("笔记保存会话正忙，请稍后重试。");
          leases.set(requestId, { lease, baseline, editorDocument }); holdTextEditor(pageId, requestId, true);
        } catch (reason) { error = noteReviewErrorMessage(reason); holdTextEditor(pageId, requestId, false); }
        await invoke("agent_ack_note_text_editor", { requestId, sessionId, error }).catch(() => {
          const pending = leases.get(requestId); pending?.lease.cancelBeforeDispatch(); leases.delete(requestId); holdTextEditor(pageId, requestId, false);
        });
      })();
    });
    stops.push(flushStop);
    const releaseStop = await listen<Release>("research-note-text-release", event => { void receiveInOrder(() => release(event.payload)); }); stops.push(releaseStop);
    const receiptStop = await listen<Receipt>("research-note-text-receipt", event => { void receiveInOrder(() => applyReceipt(event.payload.pages)); }); stops.push(receiptStop);
    if (!disposed) await invoke("agent_register_note_text_editor", { sessionId, pageId });
    else stops.splice(0).forEach(stop => stop());
  })().catch(reason => { if (!disposed) options.onError(`笔记工具会话注册失败：${String(reason)}`); });
  return () => {
    disposed = true; stops.splice(0).forEach(stop => stop());
    stopReceipts();
    leases.forEach(({ lease }, request) => { lease.cancelBeforeDispatch(); holdTextEditor(pageId, request, false); }); leases.clear();
    void invoke("agent_register_note_text_editor", { sessionId, pageId: null }).catch(() => {});
  };
}

function prepareReceiptInstall(pageId: string, before: NoteDocumentV1, receipt: AppliedNotePage, after: NoteDocumentV1): () => undefined {
  // Typing is already in the editor, possibly followed by newer input. The
  // acknowledgement advances S only. Review/Agent delivery installs D and B
  // in one native editor transaction instead of two separate undo items.
  const review = receipt.review;
  if (review?.origin === "manual") return () => undefined;
  return textInstallPort(pageId).prepareStructure(before, after, review ? {
    baseline: review.baseline, origin: review.origin, addToHistory: review.addToHistory,
    ...(review.requestId ? { requestId: review.requestId } : {}),
  } : undefined);
}
