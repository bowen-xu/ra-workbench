import {useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState} from "react";
import {readingNoteDisplayTitle, type NoteLink, type NotePageId} from "@research-assistant/core/notes";
import type {NoteRevisionSummary} from "@research-assistant/data/notes";
import {useNotesClient, useNotesVersion} from "../NotesClient";
import {useHostedNoteDocumentSession as useNoteDocumentSession, useHostedNoteWorkspaceTabs as useNoteWorkspaceTabs, useCoreNoteWorkspaceHost} from "./noteWorkspaceHostContext";
import {CoreNoteWorkspaceHost, OwnedCoreNoteWorkspaceHost} from "./CoreNoteWorkspaceHost";
import {useNoteDocumentMetadata} from "./useNoteDocumentMetadata";
import {useNoteCatalogTargets} from "./useNoteCatalogTargets";
import {useNoteAcceptance} from "../agent/useNoteAcceptance";
import {useResearchObjectTargets} from "../links/useResearchObjectTargets";
import {useNoteHistoryScope} from "../editor/useNoteHistoryScope";
import {useAgentContextCandidate} from "../../agent/context/AgentContextProvider";
import {noteContextCandidate} from "./noteContextCandidate";
import {notify} from "../../../services/notifications";
import {NOTE_METADATA_CHANGED, NOTE_PAGES_DELETED, type NoteMetadataChanged} from "../noteResourceEvents";
import {holdTextEditor, synchronizeNoteReview} from "../editor/noteTextEditorPorts";
import {clearNoteRecoveryDraft} from "./noteRecovery";
import {NoteDocumentView} from "./NoteDocumentView";
import {NoteDocumentPending} from "./NoteDocumentPending";
import {RetainedNoteEditors} from "./RetainedNoteEditors";
import {useNotesShortcuts} from "./useNotesShortcuts";
import {applyNotePageStatus} from "../noteMetadataActions";
import {useNoteLifecycleNotice} from "./useNoteLifecycleNotice";
import {requestReadingNotePanelTransfer} from "../reading-notes/readingNotePanelTransfer";
import "./CoreNoteWorkspace.css";

import type {CoreNoteWorkspaceProps} from "@research-assistant/workbench-sdk/note-presentation";
export type {CoreNoteWorkspaceProps} from "@research-assistant/workbench-sdk/note-presentation";

/** The production single-document owner, used by floating notes and by the
 * workbench without a note manager. It has no catalog, filter or bulk UI. */
export function CoreNoteWorkspace(props: CoreNoteWorkspaceProps) {
  const embedded = props.embedded !== false;
  const Host = embedded ? OwnedCoreNoteWorkspaceHost : CoreNoteWorkspaceHost;
  return <Host {...props} embedded={embedded}><HostedCoreNoteWorkspaceContent {...props} embedded={embedded}/></Host>;
}

function HostedCoreNoteWorkspaceContent(props: CoreNoteWorkspaceProps) {
  const shared = useCoreNoteWorkspaceHost();
  return <CoreNoteWorkspaceContent {...shared} {...props}/>;
}

function CoreNoteWorkspaceContent({initialPageId, initialSnapshot, openRequestKey, readingSource, embedded = true,
  compactHeader = false, active = true, sourceNavigation, pinnedPageId, onHome, onNavigatePage, onClosePage, ref}: CoreNoteWorkspaceProps) {
  const client = useNotesClient(), version = useNotesVersion();
  const host = useCoreNoteWorkspaceHost();
  const [outgoing, setOutgoing] = useState<readonly NoteLink[]>([]), [incoming, setIncoming] = useState<readonly NoteLink[]>([]);
  const [revisions, setRevisions] = useState<readonly NoteRevisionSummary[]>([]), [showHistory, setShowHistory] = useState(false);
  const [busy, setBusy] = useState(false), [diffTarget, setDiffTarget] = useState<HTMLDivElement | null>(null);
  const report = useCallback((reason: unknown) => notify.error(reason instanceof Error ? reason.message : String(reason)), []);
  const message = useCallback((value: string) => notify.info(value), []);
  const session = useNoteDocumentSession({onError: report, onNotice: message,
    onLoaded: (_id, links, backlinks) => {setOutgoing(links); setIncoming(backlinks); setShowHistory(false);}});
  const {snapshot, editorSessionId, saveControllerRef, saveStatus, saveMessage} = session;
  const metadata = useNoteDocumentMetadata({client, version, active, snapshots: session.retainedSessions.map(value => value.snapshot),
    current: snapshot, source: readingSource, observe: session.observePages, onError: report});
  const source = readingSource?.pageId === snapshot?.page.id ? readingSource : metadata.readingNotes.find(value => value.pageId === snapshot?.page.id);
  const acceptance = useNoteAcceptance({snapshot, editorSessionId, saves: saveControllerRef, notes: client.gateway});
  const targets = useNoteCatalogTargets(client, version), researchTargets = useResearchObjectTargets(report, active && !!snapshot);
  const content = useRef<HTMLElement>(null), workspace = useRef<HTMLElement>(null), search = useRef<HTMLInputElement>(null);
  useNoteHistoryScope(client.history, workspace, active);
  useNoteLifecycleNotice(snapshot?.page, active);
  useAgentContextCandidate(snapshot && !client.creation.isDraft(snapshot.page.id) ? noteContextCandidate(snapshot.page, source, outgoing) : null, active, embedded ? "floating" : "tab");
  const tabs = useNoteWorkspaceTabs({page: snapshot?.page, enabled: !embedded, active, dirty: saveStatus !== "saved",
    ...(pinnedPageId ? {pinnedPageId} : {}), ...(initialPageId ? {documentPageId: initialPageId} : {}),
    ...(onHome ? {onHome} : {}), ...(onClosePage ? {onClose: onClosePage} : {}),
    open: async id => {if (await session.ensureDurable() && await requestReadingNotePanelTransfer(id)) {await session.loadPage(id); onNavigatePage?.(id);}},
    getPageId: session.getSelectedPageId, prepare: id => session.ensureDurable(false, id),
    dispose: session.disposeSession, clear: () => session.deactivate()});
  const retained = tabs.retainedIds.join("|");
  useEffect(() => session.retain(embedded ? [] : tabs.retainedIds), [retained, embedded, snapshot?.page.id, session.retain]);
  useImperativeHandle(ref === host?.ref ? undefined : ref, () => ({prepareClose: (saved = false) => session.ensureAllDurable(saved)}), [session.ensureAllDurable]);
  const request = useRef(0);
  useEffect(() => {
    if (!initialPageId) return;
    let alive = true; const sequence = ++request.current;
    void session.ensureDurable().then(async ready => {
      if (!ready || !alive || sequence !== request.current) return;
      if (!embedded && !await requestReadingNotePanelTransfer(initialPageId)) throw new Error("请先保存或解决阅读笔记的恢复问题，再在右侧打开。");
      if (alive && sequence === request.current) await session.loadPage(initialPageId, true, initialSnapshot);
    }).catch(report);
    return () => {alive = false;};
  }, [initialPageId, initialSnapshot, openRequestKey, embedded, session.ensureDurable, session.loadPage, report]);
  useEffect(() => {
    if (!snapshot) return;
    let alive = true;
    void Promise.all([client.links.listOutgoing(snapshot.page.id), client.links.listBacklinks("note_page", snapshot.page.id)])
      .then(([links, backlinks]) => {if (alive) {setOutgoing(links); setIncoming(backlinks); session.rememberLinks(snapshot.page.id, links, backlinks);}}).catch(report);
    return () => {alive = false;};
  }, [client, version, snapshot?.page.id, session.rememberLinks, report]);
  useEffect(() => {
    const changed = (event: Event) => {
      const receipt = (event as CustomEvent<NoteMetadataChanged>).detail;
      session.observePages(receipt.pages);
      for (const page of receipt.pages) {
        if (!receipt.bodyChangedIds.includes(page.id)) session.updateMetadata(page);
        else if (session.getSnapshot()?.page.id === page.id) void session.refreshAfterStructure(page.id).catch(report);
      }
      const page = receipt.pages.find(value => value.id === session.getSelectedPageId());
      if (page?.status === "trashed") dismiss(page.id);
    };
    const deleted = (event: Event) => {
      const id = session.getSelectedPageId();
      if (id && (event as CustomEvent<readonly string[]>).detail.includes(id)) dismiss(id);
    };
    function dismiss(id: NotePageId) {
      session.disposeSession(id);
      window.dispatchEvent(new CustomEvent(source?.pageId === id ? "research-assistant:reading-note-deleted" : "research-assistant:literature-note-deleted", {detail: source?.pageId === id ? source.id : id}));
    }
    window.addEventListener(NOTE_METADATA_CHANGED, changed); window.addEventListener(NOTE_PAGES_DELETED, deleted);
    return () => {window.removeEventListener(NOTE_METADATA_CHANGED, changed); window.removeEventListener(NOTE_PAGES_DELETED, deleted);};
  }, [source, session.observePages, session.updateMetadata, session.refreshAfterStructure, session.disposeSession, session.getSelectedPageId, session.getSnapshot, report]);
  useEffect(() => {
    if (!embedded || !snapshot) return;
    const frame = requestAnimationFrame(() => content.current?.querySelector<HTMLElement>(".bn-editor[contenteditable='true'], .bn-editor [contenteditable='true']")?.focus({preventScroll: true}));
    return () => cancelAnimationFrame(frame);
  }, [embedded, snapshot?.page.id, editorSessionId]);
  async function update(changes: {title?: string; isFavorite?: boolean; projectIds?: readonly string[]; tagIds?: readonly string[]}) {
    if (!snapshot || !await session.ensureDurable(true)) return;
    const current = await client.gateway.loadDocument(snapshot.page.id);
    const page = await client.mutate(gateway => gateway.updatePage({pageId: current.page.id, expectedRevision: current.page.revision, ...changes}));
    session.updateMetadata(page); await synchronizeNoteReview(page.id);
  }
  async function status(value: "active" | "archived" | "trashed") {
    if (!snapshot || !await session.ensureDurable(true)) return;
    const page = await applyNotePageStatus(client, snapshot.page.id, value);
    session.updateMetadata(page);
  }
  async function restoreRevision(id: NoteRevisionSummary["id"]) {
    if (!snapshot || !await session.ensureDurable(true)) return;
    const operation = crypto.randomUUID(); holdTextEditor(snapshot.page.id, operation, true); setBusy(true);
    try {
      const current = await client.gateway.loadDocument(snapshot.page.id);
      const restored = await client.mutate(gateway => gateway.restoreRevision(current.page.id, id, current.page.revision));
      await session.loadPage(current.page.id, false, restored); clearNoteRecoveryDraft(current.page.id); setShowHistory(false);
    } catch (reason) {
      if (session.getSnapshot()?.page.id === snapshot.page.id) saveControllerRef.current?.restore(session.getSnapshot()!.document, "conflict");
      throw reason;
    } finally {holdTextEditor(snapshot.page.id, operation, false); setBusy(false);}
  }
  async function deletePage() {
    if (!snapshot || !window.confirm(`确定永久删除“${snapshot.page.title}”吗？\n\n正文、历史和项目引用将无法恢复；引用它的其他笔记会保留为断链。子页面会移动到顶层。`) || !await session.ensureDurable(true)) return;
    const current = await client.gateway.loadDocument(snapshot.page.id);
    const result = await client.mutate(gateway => gateway.permanentlyDeletePage({id: current.page.id, expectedRevision: current.page.revision}));
    session.disposeSession(current.page.id); window.dispatchEvent(new CustomEvent(NOTE_PAGES_DELETED, {detail: [current.page.id]}));
    if (result.cleanupWarnings.length) message(result.cleanupWarnings.join("；"));
  }
  async function duplicate() {if (snapshot && await session.ensureDurable(true)) {const copy = await client.mutate(gateway => gateway.duplicatePage(snapshot.page.id)); openPage(copy.page.id);}}
  async function createPage() {
    if (!await session.ensureDurable(true)) return;
    const created = await client.creation.create(); openPage(created.page.id);
  }
  function openPage(id: NotePageId) {window.dispatchEvent(new CustomEvent("research-assistant:open-note-page", {detail: id}));}
  useNotesShortcuts({searchInputRef: search, scopeRef: workspace,
    onSave: () => void saveControllerRef.current?.flush(), onCreatePage: () => void createPage().catch(report)});
  const pageTargets = useMemo(() => metadata.pages.map(page => ({id: page.id, title: metadata.readingNotes.find(note => note.pageId === page.id) ? readingNoteDisplayTitle(metadata.readingNotes.find(note => note.pageId === page.id)!, page.title) : page.title})), [metadata.pages, metadata.readingNotes]);
  const readingTargets = useMemo(() => metadata.readingNotes.map(note => ({id: note.id, label: note.source.paperTitle, quote: note.source.quoteExact})), [metadata.readingNotes]);
  const pending = session.loadState.phase === "loading" || session.loadState.phase === "failed" ? session.loadState : undefined;
  return <section ref={workspace} className={`notes-workspace${embedded ? " embedded" : ""}`} data-presentation="document">
    <main className="notes-content" ref={content} aria-busy={session.loadState.phase === "loading"}>
      {pending && <NoteDocumentPending pageId={pending.pageId} title={pending.title} failure={pending.phase === "failed" ? pending.failure : undefined}
        onRetry={() => void session.loadPage(pending.pageId).catch(report)} />}
      {!pending && snapshot && <NoteDocumentView snapshot={snapshot} editorSessionId={editorSessionId} retainEditor compactHeader={compactHeader}
        lifecycleBusy={busy} sourceNavigation={sourceNavigation} pages={metadata.pages} readingNotes={metadata.readingNotes} researchTargets={researchTargets}
        queryNoteTargets={targets} onReadPageMetadata={ids => client.gateway.listPageMetadata(ids)} outgoingLinks={outgoing} backlinks={incoming}
        onDiffControlsTarget={setDiffTarget} onInstallPort={acceptance.registerEditor} saveStatus={acceptance.state.phase === "recovery" ? "conflict" : saveStatus} saveMessage={saveMessage}
        {...(source ? {readingSource: source, onReadingColorChange: (color: string) => void (async () => {
          if (!await session.ensureDurable(true)) return;
          const current = await client.gateway.getReadingNote(source.id);
          const next = await client.mutate(gateway => gateway.updateReadingNote({id: current.id, expectedRevision: current.revision, color}));
          session.setSnapshot(value => value ? {...value, page: {...value.page, revision: next.revision}} : value); saveControllerRef.current?.updateRevision(next.revision);
        })().catch(report)} : {})}
        onDocumentChange={session.onDocumentChange} onAssetStatus={message} onUpdatePage={changes => void update(changes).catch(report)}
        onOpenPage={openPage} onOpenReading={id => window.dispatchEvent(new CustomEvent("research-assistant:open-reading-note", {detail: id}))}
        onArchive={() => void status("archived").catch(report)} onTrash={() => void status("trashed").catch(report)} onRestore={() => void status("active").catch(report)}
        onPermanentlyDelete={() => void deletePage().catch(report)} onDuplicate={() => void duplicate().catch(report)} showHistory={showHistory} revisions={revisions}
        onOpenHistory={() => void client.gateway.listRevisions(snapshot.page.id).then(value => {setRevisions(value); setShowHistory(true);}).catch(report)}
        onCloseHistory={() => setShowHistory(false)} onLoadRevision={id => client.gateway.loadRevision(snapshot.page.id, id)} onRestoreRevision={restoreRevision}
        onToggleRevisionPinned={revision => void client.gateway.setRevisionPinned(snapshot.page.id, revision.id, !revision.isPinned)
          .then(value => setRevisions(values => values.map(row => row.id === value.id ? value : row))).catch(report)}
        onRetrySave={() => void (acceptance.state.phase === "recovery" ? acceptance.retry() : session.saveExplicitly()).catch(report)} />}
      <RetainedNoteEditors sessions={session.retainedSessions} isPageDraft={id => client.creation.isDraft(id)} hasPageTitle={!compactHeader}
        readingPageIds={new Set([...metadata.readingNotes.flatMap(note => note.pageId ? [note.pageId] : []), ...(readingSource?.pageId ? [readingSource.pageId] : [])])} activeId={active && !pending ? snapshot?.page.id : undefined}
        onInstallPort={acceptance.registerEditor} diffControlsTarget={diffTarget} conflictLocked={saveStatus === "conflict"} sourceNavigation={sourceNavigation}
        onChange={session.onRetainedDocumentChange} onAssetStatus={message} pageTargets={pageTargets} readingTargets={readingTargets} queryNoteTargets={targets} researchTargets={researchTargets}/>
      {!snapshot && !initialPageId && <div className="note-empty-state"><p>可通过已有文档标签或资源链接打开笔记。笔记管理插件未装配时，正文、资产与历史仍由工作台保留。</p></div>}
    </main>
  </section>;
}
