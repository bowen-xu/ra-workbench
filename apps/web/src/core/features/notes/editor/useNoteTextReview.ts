import { useEffect, useLayoutEffect, useRef, useState, type MutableRefObject } from "react";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { getAppRuntime } from "../../../runtime/appRuntime";
import { EDITOR_AUTOSAVE_IDLE_MS } from "../../../services/editorAutosave";
import { getAppStorage } from "../../../runtime/appStorage";
import { notifyTextPatchesChanged, subscribeTextPatches, textPatchGateway, type TextPatchReview, type TextPatchPageReview, type TextPatchResolution } from "../../agent/textPatchGateway";
import type { NoteEditorInstance } from "./noteEditorSchema";
import type { NoteEditorInstallPort } from "./noteEditorInstallPort";
import { readNoteReviewHistory } from "./noteReviewHistory";
import { registerNoteReviewSavePort, notifyNoteReviewSaveState, type NoteReviewSaveState } from "./noteTextEditorPorts";
import { sameNoteProjection } from "./noteReviewProjection";
import type { NoteEditorDiffChunk } from "./noteDiffDecorations";
import { getEditorScrollContainer } from "../../../components/block-editor/editorSurface";
import { persistReviewSnapshots, sameReviewDocument } from "@research-assistant/core/document-review";
import { resolveNoteReviewDocument } from "./noteReviewAdapter";
import { loadNoteRecoveryDraft, clearNoteRecoveryDraft } from "../workspace/noteRecovery";
import { isNoteReviewConflict, noteReviewErrorMessage } from "./noteReviewRecovery";

interface WorkingReview { readonly conversationId: string; readonly review: TextPatchReview; readonly page: TextPatchPageReview }
/** The page's existing editor/save host owns this projection. Candidate writes
 * use the native review owner; formal saves still use its existing group lease.
 */
export function useNoteTextReview({ editor, document, port, getDocument, onFormalChange }: {
  readonly editor: NoteEditorInstance; readonly document: NoteDocumentV1;
  readonly port: MutableRefObject<NoteEditorInstallPort | undefined>;
  readonly getDocument: () => NoteDocumentV1;
  readonly onFormalChange: (document: NoteDocumentV1) => void;
}) {
  const [working, setWorking] = useState<WorkingReview>();
  const live = useRef<WorkingReview | undefined>(undefined);
  const [initialized, setInitialized] = useState(getAppRuntime().mode === "browser");
  const conflicted = useRef(false);
  const recovered = useRef(false);
  const [busy, setBusy] = useState(false), [error, setError] = useState<string>();
  const [refresh, setRefresh] = useState(0);
  const [request, setRequest] = useState<TextPatchResolution>();
  const pending = useRef<TextPatchResolution | undefined>(undefined);
  const admitted = useRef(new Map<string, NoteDocumentV1>());
  const editing = useRef<Promise<void>>(Promise.resolve());
  const operations = useRef<Promise<void>>(Promise.resolve());
  const editTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const changed = useRef<NoteDocumentV1 | undefined>(undefined);
  const saveState = useRef<NoteReviewSaveState>({ status: "saved" });
  const publishSaveState = (status: NoteReviewSaveState["status"], message?: string) => {
    if (saveState.current.status === status && saveState.current.message === message) return;
    saveState.current = { status, ...(saveState.current.pendingChanges ? { pendingChanges: saveState.current.pendingChanges } : {}), ...(message ? { message } : {}) }; notifyNoteReviewSaveState();
  };
  const changedSequence = useRef(0);
  const handledHistory = useRef<number | undefined>(undefined);
  const installationUndos = useRef(new Set<string>()), replayPending = useRef(false);
  const applying = useRef(false), resolvedOnce = useRef(false), mounted = useRef(true);
  const formal = useRef(document); formal.current = document;
  const viewport = useRef<{ element: HTMLElement; top: number; left: number } | undefined>(undefined);
  const focusOrigin = useRef<Element | null | undefined>(undefined);
  const retainViewport = () => {
    const element = editor.domElement && getEditorScrollContainer(editor.domElement);
    if (element) viewport.current = { element, top: element.scrollTop, left: element.scrollLeft };
  };
  // Return keyboard ownership after a deliberate review action, without
  // moving the reading viewport or stealing a field focused during the RPC.
  useLayoutEffect(() => {
    const saved = viewport.current; viewport.current = undefined;
    if (!busy && !request && focusOrigin.current !== undefined && editor.isEditable) {
      const dom = editor.domElement, active = dom?.ownerDocument.activeElement;
      if (dom && (active === focusOrigin.current || active === dom.ownerDocument.body || active && dom.contains(active)))
        editor.prosemirrorView.focus();
      focusOrigin.current = undefined;
    }
    if (saved?.element.isConnected) { saved.element.scrollTop = saved.top; saved.element.scrollLeft = saved.left; }
  });
  const options = useRef({ getDocument, onFormalChange }); options.current = { getDocument, onFormalChange };
  const storageKey = `note-inline-review:${document.pageId}`;
  const publish = (value: WorkingReview | undefined) => {
    live.current = value;
    const pendingChanges = value?.review.state === "pending" && value.review.baselineMatches
      ? (value.page.chunks?.length ?? 0) + (value.page.metadataPending ? 1 : 0) : 0;
    if ((saveState.current.pendingChanges ?? 0) !== pendingChanges) {
      saveState.current = { ...saveState.current, pendingChanges }; notifyNoteReviewSaveState();
    }
    if (mounted.current) setWorking(value);
  };

  function project(value: WorkingReview, origin: "agent" | "review" | "restore", requestId?: string, history = true, expected?: NoteDocumentV1): void {
    const installer = port.current, after = value.page.afterDocument, baseline = value.page.reviewDocument;
    if (!installer || !after || !baseline) throw new Error("笔记候选缺少完整的编辑器快照，请重新读取。");
    const actual = installer.read();
    if (expected && !sameNoteProjection(actual, expected) && !sameNoteProjection(actual, after))
      throw new Error("审阅期间出现了新的输入，已保留输入；请核对原操作结果。");
    const installed = readNoteReviewHistory(editor.prosemirrorState);
    if (sameNoteProjection(actual, after) && installed && sameNoteProjection(installed.baseline, baseline)) return;
    applying.current = true;
    try { installer.prepareStructure(actual, after, { baseline, origin, ...(requestId ? { requestId } : {}), addToHistory: history })(); }
    finally { applying.current = false; }
  }

  async function execute(original: TextPatchResolution, history: boolean): Promise<void> {
    retainViewport();
    changedSequence.current++;
    const expected = admitted.current.get(original.requestId) ?? port.current?.read();
    if (expected) admitted.current.set(original.requestId, expected);
    pending.current = original; setRequest(original); setError(undefined); setBusy(true);
    publishSaveState("saving");
    try {
      getAppStorage().setItem(storageKey, JSON.stringify({ request: original, expectedDocument: expected }));
      const receipt = await textPatchGateway.resolve(original);
      if (!mounted.current) return;
      // Read again after the RPC so scrolling while it was pending wins.
      retainViewport();
      if (receipt.requestId !== original.requestId || receipt.proposalId !== original.proposalId || receipt.action !== original.action)
        throw new Error("笔记审阅结果身份不匹配");
      const page = receipt.review?.pages.find(item => item.pageId === document.pageId);
      if (receipt.review && page) {
        const next = { conversationId: original.conversationId, review: receipt.review, page };
        if (original.action !== "edit") project(next, original.action === "restore" ? "restore" : "review", history ? original.requestId : undefined, history, expected);
        const replay = readNoteReviewHistory(editor.prosemirrorState);
        if (original.action === "edit" && replay?.installationId === original.requestId) {
          if (!page.reviewDocument || !sameNoteProjection(replay.baseline, page.reviewDocument))
            throw new Error("撤销安装后的审阅基线不一致，已保留输入，请核对原操作。");
          installationUndos.current.add(original.requestId);
        }
        // After restart D may already be formally saved while the editor was
        // initialized with B=D. Recover the receipt's independent B as well.
        // Normal typing acknowledgments are no-ops here; later input is never
        // replaced by an older snapshot.
        if (original.action === "edit" && page.afterDocument && port.current
          && sameNoteProjection(port.current.read(), page.afterDocument)) project(next, "restore", undefined, false, expected);
        publish(next);
      }
      getAppStorage().removeItem(storageKey); pending.current = undefined; admitted.current.delete(original.requestId); setRequest(undefined);
      publishSaveState(changed.current ? "dirty" : "saved");
      if (!changed.current) { recovered.current = false; clearNoteRecoveryDraft(document.pageId); }
      notifyTextPatchesChanged();
    } catch (reason) {
      if (!viewport.current) retainViewport();
      // A definitive preflight conflict is safe to release. Unknown results
      // retain the exact request and fence all following candidate mutations.
      const failed = reason && typeof reason === "object" && "status" in reason && reason.status === "failed";
      if (failed) {
        getAppStorage().removeItem(storageKey); pending.current = undefined; admitted.current.delete(original.requestId); setRequest(undefined);
      }
      if (failed && original.action === "edit") changed.current ??= options.current.getDocument();
      if (isNoteReviewConflict(reason) || failed && (original.action === "restore"
        || readNoteReviewHistory(editor.prosemirrorState)?.installationId === original.requestId)) conflicted.current = true;
      const message = noteReviewErrorMessage(reason);
      publishSaveState(conflicted.current ? "conflict" : "failed", `正文／审阅基线尚未确认保存：${message}`);
      if (mounted.current) setError(`${failed ? "审阅未应用，输入已保留" : "无法确认审阅结果，请恢复原操作"}：${message}`);
      throw reason;
    } finally { if (mounted.current) setBusy(false); }
  }

  async function flushEdits(immediate = true): Promise<void> {
    if (conflicted.current) throw new Error("笔记审阅版本存在冲突，请比较版本或保留当前内容为副本。");
    if (editTimer.current !== undefined) { clearTimeout(editTimer.current); editTimer.current = undefined; }
    if (changed.current) {
      const task = editing.current.catch(() => undefined).then(async () => {
        await persistReviewSnapshots({
          capture: () => !immediate && editTimer.current !== undefined ? undefined : changed.current,
          prepare: next => {
            const value = live.current;
            if (!next || !value || (value.review.state !== "pending"
              && !(value.review.state === "accept" && value.review.persistenceVersion === 2))) return null;
            if (pending.current) throw new Error("原审阅操作结果尚未确认，输入已保留。");
            changed.current = undefined;
            return { conversationId: value.conversationId, proposalId: value.review.proposalId,
              generation: value.review.generation, requestId: crypto.randomUUID(), pageId: document.pageId,
              action: "edit" as const, document: next };
          },
          write: write => execute(write, false),
        });
      });
      editing.current = task;
    }
    await editing.current;
    if (pending.current?.action === "edit") throw new Error("笔记候选尚未可靠保存，请恢复原操作。");
  }

  const resolve = (action: "keep" | "undo", chunkId?: string): void => {
    const active = editor.domElement?.ownerDocument.activeElement;
    const priorChunk = live.current?.page.chunks?.find(chunk => chunk.id === chunkId);
    const task = operations.current.then(async () => {
      await flushEdits();
      const value = live.current; if (!value || value.review.state !== "pending" || pending.current) return;
      if (!value.review.baselineMatches) throw new Error("正式笔记已有变化，已保留当前输入，请核对版本。");
      if (chunkId && !value.page.chunks?.some(chunk => chunk.id === chunkId) && priorChunk) {
        const identities = new Set([...priorChunk.beforeBlockIds, ...priorChunk.afterBlockIds]);
        const matching = value.page.chunks?.filter(chunk => [...chunk.beforeBlockIds, ...chunk.afterBlockIds].some(id => identities.has(id))) ?? [];
        if (matching.length !== 1) throw new Error("变更范围已改变，请重新选择当前 Diff。");
        chunkId = matching[0]!.id;
      }
      const page = value.page;
      if (!page.afterDocument || !page.reviewDocument) throw new Error("审阅快照不完整，请重新读取。");
      const expected = resolveNoteReviewDocument({ content: page.afterDocument, reviewBaseline: page.reviewDocument }, action,
        chunkId ? page.chunks?.find(chunk => chunk.id === chunkId) : undefined);
      focusOrigin.current = active;
      await execute({ conversationId: value.conversationId, proposalId: value.review.proposalId, generation: value.review.generation,
        requestId: crypto.randomUUID(), pageId: document.pageId, action, ...(chunkId ? { chunkId } : {}) }, true);
      const result = live.current?.page;
      if (!result?.afterDocument || !result.reviewDocument || !sameReviewDocument(expected,
        { content: result.afterDocument, reviewBaseline: result.reviewDocument }, sameNoteProjection))
        throw new Error("审阅回执与正文／基线变更不一致，请核对原操作。");
      resolvedOnce.current = true;
    });
    operations.current = task.catch(reason => { if (mounted.current) setError(noteReviewErrorMessage(reason)); });
  };

  const publishInput = (): boolean => {
    if (applying.current || port.current?.isInstalling) return true;
    const value = live.current;
    const history = readNoteReviewHistory(editor.prosemirrorState);
    if (history?.direction && (history.historyProofRequestId || history.installationId) && value) {
      const direction = history.direction, installationId = history.installationId;
      const historyRequestId = history.historyProofRequestId ?? installationId;
      if (!historyRequestId) return true;
      if (handledHistory.current === history.revision) return true;
      handledHistory.current = history.revision;
      replayPending.current = true;
      publishSaveState("saving");
      const active = editor.domElement?.ownerDocument.activeElement;
      const replayDocument = options.current.getDocument();
      const task = operations.current.then(async () => {
        await flushEdits();
        const current = live.current; if (!current || pending.current) return;
        focusOrigin.current = active;
        const identity = { conversationId: current.conversationId, proposalId: current.review.proposalId,
          generation: current.review.operationGeneration ?? current.review.generation, pageId: document.pageId };
        if (installationId && !installationUndos.current.has(installationId)) {
          if (direction !== "undo") throw new Error("原安装撤销尚未确认，请恢复原操作。");
          // The first undo records a normal candidate edit, including trusted
          // native before/after B/D. Redo reverses that receipt rather than
          // publishing restored Agent text as an already-kept manual save.
          await execute({ ...identity, requestId: installationId, action: "edit", document: replayDocument }, false);
        } else await execute({ ...identity, requestId: crypto.randomUUID(), action: "restore", historyRequestId,
          direction: installationId ? direction === "undo" ? "redo" : "undo" : direction }, false);
      });
      operations.current = task.catch(reason => { if (mounted.current) setError(noteReviewErrorMessage(reason)); }).finally(() => { replayPending.current = false; });
      return true;
    }
    if (conflicted.current) { changed.current = options.current.getDocument(); changedSequence.current++; publishSaveState("conflict", "当前输入已保留，请比较版本。"); return true; }
    if (!value || (value.review.state !== "pending" && !(value.review.state === "accept" && value.review.persistenceVersion === 2))) return false;
    const current = options.current.getDocument();
    if (conflicted.current || !value.review.baselineMatches) {
      changed.current = current; changedSequence.current++;
      publishSaveState("conflict", "笔记审阅版本存在冲突，当前输入已保留，请比较版本。");
      return true;
    }
    // Undo may return to the last acknowledged D while a different D is being
    // saved. Compare against the newest queued/dispatched snapshot, otherwise
    // that undo is dropped and the older response falsely finishes the flush.
    const latest = changed.current ?? (pending.current?.action === "edit" ? pending.current.document : value.page.afterDocument);
    if (latest && sameNoteProjection(current, latest)) return true;
    changed.current = current; changedSequence.current++;
    publishSaveState(pending.current ? "saving" : "dirty");
    if (editTimer.current !== undefined) clearTimeout(editTimer.current);
    editTimer.current = setTimeout(() => { editTimer.current = undefined; void flushEdits(false).catch(() => {}); }, EDITOR_AUTOSAVE_IDLE_MS);
    return true;
  };

  useEffect(() => subscribeTextPatches(() => setRefresh(value => value + 1)), []);
  useEffect(() => registerNoteReviewSavePort(document.pageId, { flush: flushEdits, retry: restorePending, getState: () => saveState.current,
    discardRecovery: () => {
      pending.current = undefined; changed.current = undefined; conflicted.current = false; recovered.current = false;
      admitted.current.clear(); editing.current = Promise.resolve(); operations.current = Promise.resolve();
      if (editTimer.current !== undefined) clearTimeout(editTimer.current); editTimer.current = undefined;
      getAppStorage().removeItem(storageKey); setRequest(undefined); publish(undefined); setError(undefined); publishSaveState("saved");
    },
    synchronize: async () => {
      changedSequence.current++;
      try { await refreshReview(() => mounted.current, true); }
      catch (reason) { conflicted.current = true; publishSaveState("conflict", noteReviewErrorMessage(reason)); setError(noteReviewErrorMessage(reason)); throw reason; }
      finally { if (mounted.current) setInitialized(true); }
    },
    recovery: () => ({ document: options.current.getDocument(), baseline: readNoteReviewHistory(editor.prosemirrorState)?.baseline ?? options.current.getDocument(),
      ...(live.current ? { proposalId: live.current.review.proposalId, conversationId: live.current.conversationId } : {}),
      ...(pending.current ? { request: pending.current, ...(admitted.current.get(pending.current.requestId) ? { expectedDocument: admitted.current.get(pending.current.requestId)! } : {}) } : {}) }),
  }), [document.pageId]);
  useEffect(() => {
    mounted.current = true;
    try {
      const recovery = loadNoteRecoveryDraft(document.pageId)?.review;
      if (recovery && port.current) {
        const actual = port.current.read();
        applying.current = true;
        try { port.current.prepareStructure(actual, actual, { baseline: recovery.baseline, origin: "restore", addToHistory: false })(); }
        finally { applying.current = false; }
        recovered.current = true;
        if (recovery.request) {
          pending.current = recovery.request; setRequest(recovery.request);
          if (recovery.expectedDocument) admitted.current.set(recovery.request.requestId, recovery.expectedDocument);
          if (recovery.request.action === "edit" && recovery.request.document && !sameNoteProjection(actual, recovery.request.document))
            changed.current = actual;
          setError("上次审阅结果尚未确认，请恢复原操作。");
          publishSaveState("failed", "上次正文／审阅基线的保存结果尚未确认，请恢复原操作。");
        }
      }
      const stored = getAppStorage().getItem(storageKey);
      if (stored) { const data = JSON.parse(stored) as TextPatchResolution | { request: TextPatchResolution; expectedDocument?: NoteDocumentV1 };
        const original = "request" in data ? data.request : data;
        if ("request" in data && data.expectedDocument) admitted.current.set(original.requestId, data.expectedDocument);
        if (original.pageId === document.pageId && typeof original.requestId === "string") {
        pending.current = original; setRequest(original); setError("上次审阅结果尚未确认，请恢复原操作。");
        publishSaveState("failed", "上次正文／审阅基线的保存结果尚未确认，请恢复原操作。");
      } }
    } catch { conflicted.current = true; publishSaveState("conflict", "审阅恢复记录无法读取，当前输入已保留。"); setError("审阅恢复记录无法读取，当前输入已保留。"); }
    return () => { mounted.current = false; if (editTimer.current !== undefined) clearTimeout(editTimer.current); void flushEdits().catch(() => {}); };
  }, [document.pageId]);
  async function refreshReview(current: () => boolean, delivery = false): Promise<void> {
    if (getAppRuntime().mode === "browser") return;
    const sequence = changedSequence.current, expected = port.current?.read();
    const conversations = await textPatchGateway.forPage(document.pageId);
    const reviews = await Promise.all(conversations.map(async conversationId => ({ conversationId, review: await textPatchGateway.review(conversationId) })));
    if (!current() || sequence !== changedSequence.current || changed.current || pending.current) return;
    const candidates = reviews.flatMap(value => { const page = value.review?.pages.find(item => item.pageId === document.pageId);
      return value.review && page?.afterDocument && page.reviewDocument ? [{ conversationId: value.conversationId, review: value.review, page }] : []; });
    const selected = candidates.find(value => value.review.state === "pending" && value.review.proposalId === live.current?.review.proposalId)
      ?? candidates.find(value => value.review.state === "pending") ?? candidates.find(value => value.review.state === "accept");
    if (selected && selected.review.proposalId === live.current?.review.proposalId && selected.review.generation < live.current.review.generation) return;
    // Completed historical reviews do not own the current editor. Only an
    // unresolved proposal can require conflict resolution.
    if (selected && !selected.review.baselineMatches && selected.review.state !== "pending") {
      publish(undefined); conflicted.current = false; setError(undefined); publishSaveState("saved");
      return;
    }
    if (selected && !selected.review.baselineMatches) {
      conflicted.current = true; publish(selected);
      publishSaveState("conflict", "已保存正文与审阅版本不一致，请比较版本或保留当前内容为副本。");
      setError("已保留最新正文，旧提案没有应用；请比较版本或保留当前内容为副本。"); return;
    }
    if (selected?.review.state === "pending") {
      if (recovered.current && !sameNoteProjection(options.current.getDocument(), selected.page.afterDocument!)) {
        conflicted.current = true; publish(selected); publishSaveState("conflict", "已恢复未保存输入及审阅基线，请比较版本。");
        setError("已恢复上次输入，旧候选没有覆盖正文；请比较版本或保留副本。"); return;
      }
      const previous = live.current, actual = options.current.getDocument();
      if (previous?.review.state !== "pending" && !sameNoteProjection(actual, formal.current)
        && !sameNoteProjection(actual, selected.page.afterDocument!)) throw new Error("笔记有尚未保存的输入，候选暂未安装。请先完成保存。");
      // A proposal becomes the input/save owner only after D/B installed successfully.
      project(selected, "agent", undefined, resolvedOnce.current, expected);
      conflicted.current = false; setError(undefined); publishSaveState("saved");
      publish(selected); resolvedOnce.current = true;
    } else if (live.current?.review.state === "pending") {
      applying.current = true;
      try { const installer = port.current; if (installer) installer.prepareStructure(installer.read(), formal.current)(); }
      finally { applying.current = false; }
      publish(selected); resolvedOnce.current = true;
    } else if (selected) { publish(selected); resolvedOnce.current = true; }
    else {
      if (delivery && readNoteReviewHistory(editor.prosemirrorState)?.origin === "agent") throw new Error("修改已保存，但审阅会话尚未就绪，请保留输入后恢复原操作。");
      resolvedOnce.current = true;
    }
  }
  useEffect(() => {
    let active = true;
    void refreshReview(() => active).catch(reason => {
      if (active) { conflicted.current = true; publishSaveState("conflict", noteReviewErrorMessage(reason)); setError(noteReviewErrorMessage(reason)); }
    })
      .finally(() => { if (active) setInitialized(true); });
    return () => { active = false; };
  }, [document.pageId, refresh, port]);

  const chunks: NoteEditorDiffChunk[] = !conflicted.current && working?.review.state === "pending" ? (working.page.chunks ?? []).map(chunk => ({ id: chunk.id,
    before: working.page.reviewDocument?.blocks.slice(chunk.beforeIndex, chunk.beforeIndex + chunk.beforeBlockIds.length) ?? [],
    after: working.page.afterDocument?.blocks.slice(chunk.afterIndex, chunk.afterIndex + chunk.afterBlockIds.length) ?? [], afterFrom: chunk.afterIndex })) : [];
  async function restorePending(): Promise<void> { const original = pending.current; if (original) {
    const history = readNoteReviewHistory(editor.prosemirrorState);
    const recordHistory = (original.action === "keep" || original.action === "undo") && history?.historyProofRequestId !== original.requestId;
    await execute(original, recordHistory); editing.current = Promise.resolve();
  } else if (saveState.current.status === "failed" && live.current) {
    const value = live.current, latest = await textPatchGateway.review(value.conversationId, value.review.proposalId);
    if (!latest?.baselineMatches || latest.generation !== value.review.generation) {
      conflicted.current = true; publishSaveState("conflict", "审阅版本已变化，请比较版本或保留副本。");
    }
  } await flushEdits(); }
  return { chunks, resolve, publishInput, busy: busy || !!request, error, request, restore: () => { void restorePending().catch(() => {}); },
    locked: conflicted.current || !initialized || !!request && (request.action !== "edit" || !!error || replayPending.current),
    // Ordinary saves serialize snapshots without owning the editor. Undo can
    // enqueue a newer D while the prior checkpoint is awaiting its receipt.
    canReplayHistory: () => !conflicted.current && (!pending.current || pending.current.action === "edit" && !error) && !replayPending.current,
    pendingBlockIds: working?.page.chunks?.flatMap(chunk => [...chunk.beforeBlockIds, ...chunk.afterBlockIds]) ?? [],
    trustedDocument: working?.page.afterDocument ?? document,
    metadataPending: !conflicted.current && working?.review.state === "pending" && working.page.metadataPending === true,
    metadataKind: working?.page.kind, disabled: !!request && (request.action !== "edit" || !!error || replayPending.current)
      || working?.review.baselineMatches === false };
}
