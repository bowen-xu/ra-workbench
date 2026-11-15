import { useCallback, useEffect, useRef, useState } from "react";
import type { NoteDocumentV1, NoteLink, NotePageId } from "@research-assistant/core/notes";
import { NotesGatewayError, type NoteDocumentSnapshot } from "@research-assistant/data/notes";
import type { NotesClient } from "../NotesClient";
import type { NoteSaveStatus } from "../editor/noteSaveController";
import { AppNoteHbsSession } from "./appNoteHbsSession";
import { registerNoteTextSession } from "../editor/noteTextEditorSession";
import { clearNoteRecoveryDraft, loadNoteRecoveryDraft } from "./noteRecovery";
import { retryNoteReview, useNoteReviewSaveState, getNoteReviewSaveState, noteEditorDocument, isTextEditorSaveBlocked } from "../editor/noteTextEditorPorts";
import { noteReviewErrorMessage } from "../editor/noteReviewRecovery";
import { sameNoteProjection } from "../editor/noteReviewProjection";
import type { RetainedNoteSession } from "./retainedNoteSession";
import type { AppHbsDocumentGateway } from "../../../services/hbsDocumentGateway";
import { noteOpenFailure, type NoteOpenFailure } from "./noteOpenFailure";

interface SuspendedNoteSessions {
  readonly owners: readonly RetainedNoteSession[];
  readonly activeId: NotePageId | undefined;
  preservation: Promise<void>;
}
interface NoteLoadRequest { readonly id: NotePageId; readonly version: number; title?: string }

/** Owns the page document, its save coordinator, and explicit editor sessions.
 * Ordinary and source-linked notes share this save/recovery owner. */
export function useNoteDocumentSession(options: {
  readonly client: NotesClient;
  readonly onError: (message: string) => void;
  readonly onNotice: (message: string) => void;
  readonly onLoaded: (pageId: NotePageId, outgoing: readonly NoteLink[], incoming: readonly NoteLink[]) => void;
  readonly hbsGateway?: AppHbsDocumentGateway;
}) {
  const live = useRef(options); live.current = options;
  const [snapshot, setSnapshot] = useState<NoteDocumentSnapshot>();
  const [loadState, setLoadState] = useState<{ readonly phase: "idle" }
    | { readonly phase: "loading" | "ready"; readonly pageId: NotePageId; readonly title?: string }
    | { readonly phase: "failed"; readonly pageId: NotePageId; readonly title?: string; readonly failure: NoteOpenFailure }>({ phase: "idle" });
  const loadStateRef = useRef(loadState); loadStateRef.current = loadState;
  const snapshotRef = useRef(snapshot); snapshotRef.current = snapshot;
  const [saveStatus, setSaveStatus] = useState<NoteSaveStatus>("saved");
  const [saveMessage, setSaveMessage] = useState<string>();
  const reviewSave = useNoteReviewSaveState(snapshot?.page.id);
  const [editorSessionId, setEditorSessionId] = useState(0);
  const saveControllerRef = useRef<AppNoteHbsSession | undefined>(undefined);
  const textSessionRef = useRef<(() => void) | undefined>(undefined);
  const sessions = useRef(new Map<NotePageId, RetainedNoteSession>());
  const [, updateSessions] = useState(0);
  const nextEditorSession = useRef(0);
  const activeSession = snapshot && sessions.current.get(snapshot.page.id);
  if (activeSession) Object.assign(activeSession, {snapshot, editorSessionId, saveStatus, saveMessage});
  const generation = useRef(0);
  const loadRequest = useRef<NoteLoadRequest | undefined>(undefined);
  const pendingLoad = useRef<{request: NoteLoadRequest; promise: Promise<void>} | undefined>(undefined);
  const alive = useRef(true);
  const recoveryTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const suspended = useRef<SuspendedNoteSessions | undefined>(undefined);
  const resumeSessions = useRef<() => Promise<void>>(async () => {});
  const disposeSession = useCallback((id = snapshotRef.current?.page.id, cancelLoad = true) => {
    if (!id) return;
    // Closing this target cancels its pending open even before an owner exists.
    // Releasing any other editor is unrelated to the current navigation.
    if (cancelLoad && loadRequest.current?.id === id) {
      generation.current++; loadRequest.current = undefined; pendingLoad.current = undefined;
      const previous = snapshotRef.current;
      setLoadState(previous && previous.page.id !== id ? {phase: "ready", pageId: previous.page.id} : {phase: "idle"});
    }
    const session = sessions.current.get(id);
    if (!session) return;
    sessions.current.delete(id);
    clearTimeout(session.recoveryTimer);
    session.unregisterText?.(); session.controller.dispose();
    if (saveControllerRef.current === session.controller) {
      recoveryTimer.current = undefined;
      textSessionRef.current = undefined; saveControllerRef.current = undefined;
    }
    if (alive.current) updateSessions(value => value + 1);
  }, []);
  useEffect(() => { alive.current = true; void resumeSessions.current(); return () => {
    alive.current = false; generation.current++; loadRequest.current = undefined; pendingLoad.current = undefined;
    const owners = [...sessions.current.values()];
    if (!owners.length) return;
    for (const session of owners) { session.unregisterText?.(); session.unregisterText = undefined; }
    const preservation = Promise.all(owners.map(session => session.controller.ensurePreserved())).then(() => undefined);
    suspended.current = {owners, activeId:snapshotRef.current?.page.id, preservation};
    // Preservation owns immutable checkpoints after disposal. Stop timers and
    // subscriptions immediately so hidden/unmounted owners cannot keep writing.
    for (const session of owners) disposeSession(session.snapshot.page.id);
    // A retained React tree may restart its effects after this cleanup. Keep
    // every page's identity until its real S′ receipt permits reconstruction.
    void preservation.catch(() => {});
  }; }, [disposeSession]);
  const installSaveController = useCallback(async (loaded: NoteDocumentSnapshot, recovery?: ReturnType<typeof loadNoteRecoveryDraft>, allowRecovery = true, activate = true) => {
    if (!alive.current) throw new Error("笔记会话已关闭。");
    const installation = generation.current;
    let publishedRevision = loaded.page.revision;
    let recoveryCleared = !recovery;
    const current = () => alive.current && sessions.current.get(loaded.page.id)?.controller === controller;
    const active = () => current() && saveControllerRef.current === controller;
    let session: RetainedNoteSession;
    const draftGateway = live.current.client.creation?.gateway(loaded.page.id,page => controller.adoptCreatedMetadata(page));
    const gateway = draftGateway ?? live.current.hbsGateway;
    const controller: AppNoteHbsSession = await AppNoteHbsSession.open({ loaded, ...(recovery ? { recovery } : {}), allowRecovery,
      ...(gateway ? { gateway } : {}),
      onSnapshot: next => {
        if (!current()) return;
        session.snapshot = next;
        registerText();
        session.invalidated = session.observedRevision > next.page.revision;
        if (active()) { snapshotRef.current = next; setSnapshot(next); }
        else updateSessions(value => value + 1);
        if (next.page.revision > publishedRevision && !live.current.client.creation?.isDraft(next.page.id)) {
          publishedRevision = next.page.revision;
          live.current.client.notifyChanged({kind: "document", pageId: next.page.id, revision: next.page.revision});
        }
      },
      onDurable: next => {
        if (!current() || recoveryCleared) return;
        const actual = noteEditorDocument(next.page.id);
        if (getNoteReviewSaveState(next.page.id).status !== "saved" || actual && !sameNoteProjection(actual, next.document)) return;
        clearTimeout(session.recoveryTimer); session.recoveryTimer = undefined;
        if (active()) recoveryTimer.current = undefined;
        try { clearNoteRecoveryDraft(next.page.id); recoveryCleared = true; }
        catch (reason: unknown) { live.current.onError(`正文已保存，但清理恢复草稿失败：${noteReviewErrorMessage(reason)}`); }
      },
      onRelatedChange:()=>live.current.client.notifyChanged(),
      onStatus: (status, message) => { if (current()) {
        session.saveStatus = status; session.saveMessage = message;
        if (active()) { setSaveStatus(status); setSaveMessage(message); }
        else updateSessions(value => value + 1);
      } },
    });
    if (!alive.current || generation.current !== installation) { controller.dispose(); return undefined; }
    disposeSession(loaded.page.id, false);
    const state = controller.session.getSnapshot();
    session = {snapshot: { ...loaded, page: { ...loaded.page, revision: state.observed.version }, document: state.content },
      editorSessionId: ++nextEditorSession.current, saveStatus: "saved", saveMessage: undefined,
      outgoing: [], incoming: [], controller, hbs: controller.session, unregisterText: undefined, recoveryTimer: undefined,
      invalidated: false, observedRevision: loaded.page.revision};
    sessions.current.set(loaded.page.id, session);
    if (activate) {
      // Selection already shows the target/loading surface. Only the durable
      // owner and its actual document are handed over together here.
      saveControllerRef.current = controller;
      snapshotRef.current = session.snapshot; setSnapshot(session.snapshot);
      setEditorSessionId(session.editorSessionId);
      setSaveStatus(session.saveStatus); setSaveMessage(session.saveMessage);
    }
    controller.start();
    function registerText():void {
      if(session.unregisterText || live.current.client.creation?.isDraft(loaded.page.id))return;
      session.unregisterText = registerNoteTextSession({ controller, gateway: live.current.client.gateway, getSnapshot: () => active() ? snapshotRef.current : session.snapshot,
      onError: message => live.current.onError(message), onChanged: revision => {
        live.current.client.notifyChanged({kind: "document", pageId: loaded.page.id, revision});
        void Promise.all([live.current.client.links.listOutgoing(loaded.page.id), live.current.client.links.listBacklinks("note_page", loaded.page.id)])
          .then(([outgoing, incoming]) => { if (current()) {
            session.outgoing = outgoing; session.incoming = incoming;
            if (active()) live.current.onLoaded(loaded.page.id, outgoing, incoming);
          } }).catch(reason => live.current.onError(String(reason)));
      } });
      if(active())textSessionRef.current = session.unregisterText;
    }
    registerText();
    return controller;
  }, [disposeSession]);
  const ensureDurable = useCallback(async (requireSaved = false, pageId = snapshotRef.current?.page.id): Promise<boolean> => {
    const session = pageId && sessions.current.get(pageId);
    if (!session) return true;
    const controller = session.controller;
    const owned = () => alive.current && sessions.current.get(session.snapshot.page.id) === session;
    if (pageId && isTextEditorSaveBlocked(pageId)) { live.current.onError("agent正在写入，请等待写入完成后关闭。"); return false; }
    try {
      await controller.flush();
      if (!owned()) return false;
      if (requireSaved && getNoteReviewSaveState(session.snapshot.page.id).status !== "saved") {
        live.current.onError("正文或页面关联修改尚未确认，请先核对保存状态。"); return false;
      }
      if (!controller.hasUnsavedChanges()) return true;
      if (requireSaved) { live.current.onError("正文尚未正式保存，请先处理保存或冲突问题。"); return false; }
      await controller.ensurePreserved();
      if (session.snapshot.page.status === "active") live.current.onNotice("最新正文及未确认候选已持久保全；重开后会重建待审与冲突，保存不代表 Keep。");
      return owned();
    } catch (reason) { live.current.onError(`保全失败，已取消关闭：${noteReviewErrorMessage(reason)}`); return false; }
  }, []);
  // A host may hand off its just-loaded document, but recovery and save ownership
  // must still be initialized here before the editor becomes available.
  const performLoad = useCallback(async (request: NoteLoadRequest, allowRecovery = true, preloaded?: NoteDocumentSnapshot, reload = false): Promise<void> => {
    const {id, version} = request;
    const valid = () => alive.current && generation.current === version && loadRequest.current === request;
    if (!valid()) return;
    const cached = sessions.current.get(id);
    const loading = () => setLoadState({phase: "loading", pageId: id, ...(request.title ? {title: request.title} : {})});
    if (!cached || cached.invalidated || preloaded || reload || suspended.current) loading();
    let missingDocument = false;
    const readDocument = async () => {
      try {
        const stored = await live.current.client.gateway.loadDocument(id);
        await live.current.client.creation?.observeStored(stored);
        return stored;
      }
      catch (reason) {
        missingDocument = reason instanceof NotesGatewayError && reason.code === "not_found";
        // Only older unpublished recovery records can lack a canonical page.
        const legacy = missingDocument ? live.current.client.creation?.snapshot(id) : undefined;
        if (legacy) { missingDocument = false; return legacy; }
        throw reason;
      }
    };
    try {
      if (suspended.current) await suspended.current.preservation;
      if (!valid()) return;
      if (cached?.invalidated && cached.controller.synchronizeExternal && allowRecovery && !preloaded && !reload) {
        await cached.controller.ensurePreserved?.();
        const latest = await readDocument();
        if (!valid()) return;
        await cached.controller.synchronizeExternal(latest);
        if (!valid()) return;
        cached.invalidated = cached.observedRevision > cached.snapshot.page.revision;
      }
      if (cached?.invalidated && !await ensureDurable(false, id)) {
        if (valid()) setLoadState({phase: "failed", pageId: id, failure: {
          kind: "failed", code: "note_preservation_required", message: "当前笔记尚未完成保全，请先核对保存状态，再重新加载。",
        }});
        return;
      }
      if (!valid()) return;
      if (cached && !cached.invalidated && allowRecovery && !preloaded && !reload) {
        snapshotRef.current = cached.snapshot; setSnapshot(cached.snapshot);
        saveControllerRef.current = cached.controller; textSessionRef.current = cached.unregisterText;
        recoveryTimer.current = cached.recoveryTimer;
        setSaveStatus(cached.saveStatus); setSaveMessage(cached.saveMessage);
        setEditorSessionId(cached.editorSessionId);
        setLoadState({phase: "ready", pageId: id});
        live.current.onLoaded(id, cached.outgoing, cached.incoming);
        return;
      }
      const result = await Promise.all([
        preloaded ?? readDocument(), live.current.client.creation?.isDraft(id) ? [] : live.current.client.links.listOutgoing(id),
        live.current.client.creation?.isDraft(id) ? [] : live.current.client.links.listBacklinks("note_page", id),
      ]);
      const [loaded, outgoing, incoming] = result;
      if (!valid()) return;
      if (loaded.page.id !== id || loaded.document.pageId !== id) {
        throw new Error("加载的笔记身份不匹配。");
      }
      request.title = loaded.page.title; loading();
      let recovery: ReturnType<typeof loadNoteRecoveryDraft>;
      if (allowRecovery) {
        try { recovery = loadNoteRecoveryDraft(id); }
        catch (reason: unknown) { live.current.onError(reason instanceof Error ? reason.message : String(reason)); }
      }
      const different = recovery !== undefined && !sameNoteProjection(recovery.document, loaded.document);
      const hasConflict = recovery !== undefined && (recovery.review?.request !== undefined || different && (recovery.review !== undefined || recovery.baseRevision !== loaded.page.revision));
      // A restored review owns D/B. Its formal save coordinator must start from
      // acknowledged S so an original request receipt can advance that owner.
      const controller = await installSaveController(loaded, recovery, allowRecovery);
      if (!valid() || !controller || saveControllerRef.current !== controller) return;
      const installed = sessions.current.get(id)!;
      Object.assign(installed, {outgoing, incoming});
      setLoadState({ phase: "ready", pageId: id });
      live.current.onLoaded(id, outgoing, incoming);
      if (recovery) {
        live.current.onNotice(hasConflict ? "已恢复上次暂存内容；数据库版本已变化，请在正文内核对冲突" : "已自动恢复上次暂存内容");
      }
    } catch (reason) {
      if (!valid()) return;
      setLoadState({phase: "failed", pageId: id, failure: noteOpenFailure(reason, missingDocument), ...(request.title ? {title: request.title} : {})});
      throw reason;
    }
  }, [disposeSession, installSaveController, ensureDurable]);
  const loadPage = useCallback((id: NotePageId, allowRecovery = true, preloaded?: NoteDocumentSnapshot, reload = false, title?: string): Promise<void> => {
    if (allowRecovery && !preloaded && !reload && pendingLoad.current?.request === loadRequest.current && loadRequest.current?.id === id) return pendingLoad.current!.promise;
    const request: NoteLoadRequest = {id, version: ++generation.current, ...(title ? {title} : {})};
    loadRequest.current = request;
    const promise = performLoad(request, allowRecovery, preloaded, reload);
    pendingLoad.current = {request, promise};
    const clear = () => {
      if (pendingLoad.current?.request === request) pendingLoad.current = undefined;
      if (loadRequest.current === request) loadRequest.current = undefined;
    };
    void promise.then(clear, clear);
    return promise;
  }, [performLoad]);
  resumeSessions.current = async () => {
    const pending = suspended.current;
    if (!pending) return;
    try {
      await pending.preservation;
      if (!alive.current || suspended.current !== pending) return;
      for (const previous of pending.owners) {
        const id = previous.snapshot.page.id;
        if (id === pending.activeId || sessions.current.has(id)) continue;
        const loaded = await live.current.client.gateway.loadDocument(id);
        const controller = await installSaveController(loaded, loadNoteRecoveryDraft(id), true, false);
        if (!controller || !alive.current || suspended.current !== pending) return;
        const restored = sessions.current.get(id)!;
        restored.outgoing = previous.outgoing; restored.incoming = previous.incoming;
      }
      if (pending.activeId) await loadPage(pending.activeId, true, undefined, true);
      if (suspended.current === pending) suspended.current = undefined;
      if (alive.current) updateSessions(value => value + 1);
    } catch (reason) {
      if (!alive.current) return;
      const message = `MODEL_SESSION_RECOVERY_FAILED：${noteReviewErrorMessage(reason)}；原编辑内容仍保留，重试将先核对保全结果。`;
      setSaveStatus("failed"); setSaveMessage(message); live.current.onError(message);
    }
  };
  const onRetainedDocumentChange = useCallback((document: NoteDocumentV1, editorId?: number) => {
    if (!alive.current) return;
    const session = sessions.current.get(document.pageId);
    if (!session || (editorId === undefined ? snapshotRef.current?.page.id !== document.pageId : editorId !== session.editorSessionId)) return;
    session.controller.enqueue(document);
  }, []);
  const onDocumentChange = useCallback((document: NoteDocumentV1) => onRetainedDocumentChange(document), [onRetainedDocumentChange]);
  const getSnapshot = useCallback(() => snapshotRef.current, []);
  const getSelectedPageId = useCallback(() => loadRequest.current?.id
    ?? (loadStateRef.current.phase === "loading" || loadStateRef.current.phase === "failed" ? loadStateRef.current.pageId : snapshotRef.current?.page.id), []);
  const refreshAfterStructure = useCallback(async (id: NotePageId): Promise<void> => {
    const owner = saveControllerRef.current;
    const initial = snapshotRef.current;
    if (!owner || initial?.page.id !== id) return;
    const [server, outgoing, incoming] = await Promise.all([
      live.current.client.gateway.loadDocument(id), live.current.client.links.listOutgoing(id),
      live.current.client.links.listBacklinks("note_page", id),
    ]);
    if (!alive.current || saveControllerRef.current !== owner) return;
    const current = snapshotRef.current;
    if (!current || current.page.id !== id || server.page.id !== id || server.document.pageId !== id) return;
    await owner.synchronizeExternal(server);
    if (!alive.current || saveControllerRef.current !== owner) return;
    const retained = sessions.current.get(id);
    if (retained) Object.assign(retained, {outgoing, incoming});
    live.current.onLoaded(id, outgoing, incoming);
  }, []);

  // HBS and the existing metadata transaction share the save indicator. A body
  // ACK cannot hide an unresolved metadata request or its original identity.
  const priority = { saved: 0, dirty: 1, saving: 2, conflict: 3, failed: 4 };
  const showReview = priority[reviewSave.status] >= priority[saveStatus];
  const retrySave = async () => { const pageId = snapshotRef.current?.page.id;
    if (suspended.current) {
      suspended.current.preservation = Promise.all(suspended.current.owners.map(owner => owner.controller.ensurePreserved())).then(() => undefined);
      await resumeSessions.current(); return;
    }
    if (pageId) await retryNoteReview(pageId); };
  const saveExplicitly = async (): Promise<"saved" | "preserved"> => {
    const id=snapshotRef.current?.page.id;
    if(!id)throw new Error("MODEL_OWNER_UNAVAILABLE：笔记保存拥有者尚未就绪。");
    await retrySave();
    const owner=sessions.current.get(id)?.controller;
    if(!owner)throw new Error("MODEL_OWNER_UNAVAILABLE：笔记保存拥有者尚未就绪。");
    await owner.ensurePreserved();
    const result=getNoteReviewSaveState(id);
    if(result.status==="failed")throw new Error(result.message??"MODEL_SAVE_FAILED：当前输入尚未确认保存。");
    return result.status==="saved" ? "saved" : "preserved";
  };
  const deactivate = useCallback(() => {
    generation.current++; loadRequest.current = undefined; pendingLoad.current = undefined;
    snapshotRef.current = undefined; setSnapshot(undefined);
    setLoadState({phase: "idle"});
    saveControllerRef.current = undefined; textSessionRef.current = undefined; recoveryTimer.current = undefined;
  }, []);
  const ensureAllDurable = useCallback(async (requireSaved = false): Promise<boolean> => {
    for (const id of sessions.current.keys()) if (!await ensureDurable(requireSaved, id)) return false;
    return true;
  }, [ensureDurable]);
  const retain = useCallback((ids: readonly string[]) => {
    for (const id of sessions.current.keys()) if (id !== snapshotRef.current?.page.id && id !== loadRequest.current?.id && !ids.includes(id)) disposeSession(id);
  }, [disposeSession]);
  const updateMetadata = useCallback((page: NoteDocumentSnapshot["page"]) => {
    const session = sessions.current.get(page.id);
    if (!session || page.revision < session.snapshot.page.revision) return;
    session.snapshot = {...session.snapshot, page}; session.controller.updateMetadata(page);
    session.invalidated = session.observedRevision > page.revision;
    if (snapshotRef.current?.page.id === page.id) { snapshotRef.current = session.snapshot; setSnapshot(session.snapshot); }
    else updateSessions(value => value + 1);
  }, []);
  const observePages = useCallback((pages: readonly NoteDocumentSnapshot["page"][]) => {
    for (const page of pages) {
      const session = sessions.current.get(page.id);
      if (session) {
        session.observedRevision = Math.max(session.observedRevision, page.revision);
        session.invalidated = session.observedRevision > session.snapshot.page.revision;
      }
    }
  }, []);
  const rememberLinks = useCallback((id: NotePageId, outgoing: readonly NoteLink[], incoming: readonly NoteLink[]) => {
    const session = sessions.current.get(id);
    if (session) {session.outgoing = outgoing; session.incoming = incoming;}
  }, []);
  return { snapshot, setSnapshot, loadState, saveStatus: showReview ? reviewSave.status : saveStatus,
    saveMessage: showReview ? reviewSave.message : saveMessage, retrySave, saveExplicitly,
    editorSessionId, setEditorSessionId, saveControllerRef, installSaveController, disposeSession,
    ensureDurable, loadPage, onDocumentChange, getSnapshot, getSelectedPageId, refreshAfterStructure, deactivate,
    retainedSessions: [...sessions.current.values()], retain, onRetainedDocumentChange, ensureAllDurable, updateMetadata, observePages, rememberLinks };
}
