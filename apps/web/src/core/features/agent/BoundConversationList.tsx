import { getAppStorage } from "../../runtime/appStorage";
import { flushStorage } from "../../runtime/storagePersistence";
import { ControlHint } from "../../components/ControlHint";
import {useAgentSteeringOwners} from "./AgentSteeringProvider";
import {useAgentQueueOwners} from "./AgentQueueProvider";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { Archive, ArchiveRestore, ArrowLeft, BrushCleaning, ChevronDown, MessageCircle, Pin, RefreshCw, Search, SquarePen, Trash2 } from "lucide-react";
import { AgentActivitySpinner, InternalConversationIcon } from "./AgentActivity";
import { useConversationActivity } from "./useConversationActivity";
import type { AgentCatalogPage, AgentConversationBinding, AgentConversationSummary, AgentSearchHit } from "@research-assistant/core/agent";
import type { AgentArchiveGateway, AgentCatalogGateway, AgentConversationExportGateway, AgentMetadataGateway, AgentSearchGateway } from "@research-assistant/data/agent";
import { agentArchiveGateway, agentCatalogGateway, agentMetadataGateway, agentSearchGateway } from "../../services/agentGateway";
import { notify } from "../../services/notifications";
import { ConversationMenu } from "./ConversationMenu";
import { ConversationSearchForm } from "./ConversationSearchForm";
import {ConversationListToolbar} from "./ConversationDrawerHeader";
import { ConversationDeleteDialog, type ConversationDeleteConfirmationPreferences } from "./ConversationDeleteDialog";
import { agentDeletionGateway, agentDeletionBatchGateway } from "../../services/agentGateway";
import type { AgentDeletionBatchGateway, AgentDeletionGateway } from "@research-assistant/data/agent";
import { useAgentDraftStore } from "./AgentDraftProvider";
import { useConversationCatalogSync } from "./useConversationCatalogSync";
import { ConversationArchiveClearDialog } from "./ConversationArchiveClearDialog";
import "./ConversationList.css";
import { readBoundCreation, reserveBoundConversation } from "./boundConversationCreationPending";
import { conversationCreationMessage } from "./ConversationCreationRecovery";
import { useConversationArchives } from "./useConversationArchives";
import { useConversationDeletionRecords } from "./useConversationDeletionRecords";
import { ConversationDeletionRecoveries } from "./ConversationDeletionRecoveries";
import { sameDeletionHost } from "./conversationDeletionPending";
import { conversationDeletionInProgress, type ConversationDeletionTarget } from "./conversationDeletionController";
import type { ConversationOperationLock } from "./conversationOperationLock";
import { useConversationListWindow } from "./useConversationListWindow";

function readPinnedConversationIds(storageKey: string): readonly string[] {
  try {
    const storage = typeof window === "undefined" ? undefined : getAppStorage();
    if (!storage || typeof storage.getItem !== "function") return [];
    const value = JSON.parse(storage.getItem(storageKey) ?? "[]");
    return Array.isArray(value) ? value.filter((id): id is string => typeof id === "string") : [];
  } catch {
    // Desktop webviews can deny localStorage access while the document is
    // mounted. Pinning is a convenience preference and must never break the
    // conversation list when that happens.
    return [];
  }
}

function writePinnedConversationIds(storageKey: string, ids: readonly string[]): void {
  try {
    const storage = typeof window === "undefined" ? undefined : getAppStorage();
    if (storage && typeof storage.setItem === "function") storage.setItem(storageKey, JSON.stringify(ids));
  } catch {
    // Keep the in-memory preference even when persistence is unavailable.
  }
}

interface Props {
  readonly binding: AgentConversationBinding | null;
  readonly onCreate?: () => void;
  readonly selectedId: string | null;
  /** Host-owned local draft identity; absent from the list until persisted on first send. */
  readonly pendingConversationId?: string | null;
  readonly onSelect: (id: string, archived?: boolean, hit?: AgentSearchHit, pending?: boolean) => void;
  readonly onArchiveChange?: (id: string, archived: boolean) => void;
  readonly onRefreshSelected?: () => void;
  readonly refreshToken?: number;
  readonly revealConversation?: AgentConversationSummary | null;
  readonly archives?: AgentArchiveGateway;
  readonly catalog?: AgentCatalogGateway;
  readonly metadata?: AgentMetadataGateway;
  readonly search?: AgentSearchGateway;
  readonly deletions?: AgentDeletionGateway;
  readonly batchDeletions?: AgentDeletionBatchGateway;
  readonly exports?: AgentConversationExportGateway;
  readonly deletionConfirmationPreferences?: ConversationDeleteConfirmationPreferences;
  readonly forceDeletionConfirmationPreferences?: ConversationDeleteConfirmationPreferences;
  readonly deletionRecoveryStorage?: Storage;
  readonly deletionRecoveryLock?: ConversationOperationLock;
  readonly onDeleted?: (id: string) => void;
}
/** Key this component by host identity so local pending drafts remain bound to their host. */
export function BoundConversationList({ binding, onCreate, selectedId, pendingConversationId, onSelect, onArchiveChange, onRefreshSelected, refreshToken = 0, revealConversation, archives = agentArchiveGateway, catalog = agentCatalogGateway, metadata, search = agentSearchGateway, deletions = agentDeletionGateway, batchDeletions = agentDeletionBatchGateway, exports, deletionConfirmationPreferences, forceDeletionConfirmationPreferences, deletionRecoveryStorage, deletionRecoveryLock, onDeleted }: Props): React.JSX.Element {
  const drafts = useAgentDraftStore();
  const steeringOwners=useAgentSteeringOwners();
  const queueOwners=useAgentQueueOwners();
  const [deleting, setDeleting] = useState<(ConversationDeletionTarget & {readonly recoveryOnly?: boolean; readonly recoveryBinding?: AgentConversationBinding | null}) | null>(null);
  const [clearing, setClearing] = useState<{readonly binding: AgentConversationBinding | null; readonly internal: boolean} | null>(null);
  const clearTrigger = useRef<HTMLButtonElement>(null);
  const restoreClearFocus = useRef(false);
  const section = useRef<HTMLElement>(null);
  const running = useConversationActivity(catalog, binding, section);
  const internalRunning = running.some(row => row.internal);
  const restoreDeleteFocus = useRef<string | null>(null);
  const [archived, setArchived] = useState(false);
  const archiveView = useRef(false);
  const [internal, setInternal] = useState(false);
  const internalView = useRef(false);
  const internalFilter = useRef<HTMLButtonElement>(null);
  const currentFilter = useRef<HTMLButtonElement>(null);
  const archivedFilter = useRef<HTMLButtonElement>(null);
  const searchTrigger = useRef<HTMLButtonElement>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const deletionRecovery = useConversationDeletionRecords(deletionRecoveryStorage, deletionRecoveryLock);
  const batchRequests = new Set(deletionRecovery.batches.flatMap(batch => batch.requests.map(request => request.requestId)));
  const deletionRecords = deletionRecovery.records.filter(record => !batchRequests.has(record.intent.requestId)
    && (binding === null ? record.internal === internal : sameDeletionHost(record, binding, internal)));
  useEffect(() => {
    if (clearing || !restoreClearFocus.current) return;
    restoreClearFocus.current = false;
    const restore = () => (clearTrigger.current && !clearTrigger.current.disabled ? clearTrigger.current
      : archiveView.current ? currentFilter.current : archivedFilter.current)?.focus();
    restore();
    // Native modal removal can release document inertness after React effects.
    // Finish restoration on the next frame without taking focus from later input.
    const frame = requestAnimationFrame(() => {
      if (section.current?.isConnected && document.activeElement === document.body) restore();
    });
    return () => cancelAnimationFrame(frame);
  }, [clearing]);
  function closeClear(): void { restoreClearFocus.current = true; setClearing(null); synchronize.current(); }
  function confirmedDeletion(id: string): void {
    drafts.forgetConversation(id);
    steeringOwners?.writing.forgetConversation(id); steeringOwners?.notes.forgetConversation(id); steeringOwners?.conversations.forgetConversation(id); steeringOwners?.handoffs?.forgetConversation(id); steeringOwners?.pendingOrder.forgetConversation(id);
    queueOwners?.writing.forgetConversation(id); queueOwners?.notes.forgetConversation(id);
    queueOwners?.papers.forgetConversation(id); queueOwners?.conversations.forgetConversation(id);
  }
  async function flushDeletionCleanup(): Promise<void> {
    await drafts.flush();
    await flushStorage(deletionRecoveryStorage ?? getAppStorage());
  }
  function cleared(ids: readonly string[]): void {
    if (selectedLookup.current && ids.includes(selectedLookup.current.id)) selectedLookup.current = null;
    loadId.current++;
    loadingRequest.current = false;
    setPage(previous => previous ? { ...previous, conversations: previous.conversations.filter(item => !ids.includes(item.id)) } : previous);
    ids.forEach(id => onDeleted?.(id));
    synchronize.current();
  }
  useEffect(() => {
    if (deleting || !restoreDeleteFocus.current) return;
    const id = restoreDeleteFocus.current; restoreDeleteFocus.current = null;
    const row = [...(section.current?.querySelectorAll<HTMLElement>("[data-conversation-id]") ?? [])].find((node) => node.dataset.conversationId === id);
    (row?.querySelector<HTMLButtonElement>('[aria-label="永久删除对话"]')
      ?? row?.querySelector<HTMLButtonElement>("button")
      ?? (archiveView.current ? currentFilter : archivedFilter).current)?.focus();
  }, [deleting]);
  function closeDelete(): void {
    restoreDeleteFocus.current = deleting?.id ?? null; setDeleting(null);
    synchronize.current();
  }
  function deleted(id: string): void {
    if (selectedLookup.current?.id === id) selectedLookup.current = null;
    onDeleted?.(id);
    loadId.current += 1;
    loadingRequest.current = false;
    setPage((previous) => previous ? { ...previous, conversations: previous.conversations.filter((item) => item.id !== id) } : previous);
    closeDelete();
  }
  const [page, setPage] = useState<AgentCatalogPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const selection = useRef({ selectedId, pendingConversationId });
  selection.current = { selectedId, pendingConversationId };
  const pendingIdRef = useRef<string | null>(null);
  const generation = useRef(0);
  const loadId = useRef(0);
  const loadedPages = useRef(1), loadingRequest = useRef(false), syncPending = useRef(false);
  const synchronize = useRef<() => void>(() => {});
  const termRef = useRef("");
  const [searchDraft, setSearchDraft] = useState("");
  const [term, setTerm] = useState("");
  // A selected row outside the loaded pages has its own lookup lifetime.
  // Reuse its in-flight read across refreshes and pagination, or a slower read
  // can be repeatedly invalidated and never appear in a busy list.
  const selectedLookup = useRef<{id: string; view: boolean; internal: boolean; metadata: AgentMetadataGateway; value?: AgentConversationSummary; pending: boolean} | null>(null);
  const retainedLookup = selectedLookup.current;
  if (retainedLookup && (retainedLookup.id !== selectedId || selectedId === pendingConversationId
    || retainedLookup.view !== archived || retainedLookup.internal !== internal || term)) selectedLookup.current = null;
  const [hits, setHits] = useState<Readonly<Record<string, AgentSearchHit>>>({});
  const [contextConversation, setContextConversation] = useState<AgentConversationSummary | null>(null);
  const [contextPosition, setContextPosition] = useState({ x: 0, y: 0 });
  const contextTrigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (contextConversation) contextTrigger.current?.click();
  }, [contextConversation]);
  const pinStorageKey = binding ? `research-assistant:agent-pins:${binding.kind}:${binding.resourceId}` : "research-assistant:agent-pins:conversations";
  const [pinnedIds, setPinnedIds] = useState<readonly string[]>([]);
  useEffect(() => {
    // Read this after mount. WKWebView may expose a localStorage property that
    // throws during render when access has not been granted yet.
    setPinnedIds(readPinnedConversationIds(pinStorageKey));
  }, [pinStorageKey]);
  function persistPins(next: readonly string[]): void {
    setPinnedIds(next);
    writePinnedConversationIds(pinStorageKey, next);
  }
  function togglePin(id: string): void {
    persistPins(pinnedIds.includes(id) ? pinnedIds.filter((value) => value !== id) : [id, ...pinnedIds]);
  }
  async function load(more = false, view = archiveView.current, background = false): Promise<void> {
    if (background && loadingRequest.current) { syncPending.current = true; return; }
    if (!more && !background) requested.current = undefined;
    loadingRequest.current = true;
    if (!more) onRefreshSelected?.();
    const owner = generation.current;
    const request = ++loadId.current;
    if (!background) setLoading(true);
    setFailed(false);
    try {
      const query = termRef.current;
      const readPage = async (before: AgentCatalogPage["next"]) => {
        const scope = { archived: view, limit: 50, before, binding, ...(internalView.current ? { internal: true } : {}) };
        const matches = query ? await search.search({ term: query, scope }) : null;
        const result: AgentCatalogPage = matches ? { schemaVersion: 1, conversations: matches.hits.map(hit => hit.conversation), next: matches.next } : await catalog.list(scope);
        return { result, hits: Object.fromEntries((matches?.hits ?? []).map(hit => [hit.conversation.id, hit])) };
      };
      const first = await readPage(more ? page?.next ?? null : null);
      let result = first.result, newHits = first.hits, pages = 1;
      // Replay only the already loaded depth. Background synchronization keeps
      // pagination, search and filters rather than collapsing to the first page.
      while (background && pages < loadedPages.current && result.next) {
        const next = await readPage(result.next);
        const ids = new Set(result.conversations.map(row => row.id));
        result = { ...next.result, conversations: [...result.conversations, ...next.result.conversations.filter(row => !ids.has(row.id))] };
        newHits = { ...newHits, ...next.hits }; pages++;
        if (owner !== generation.current || request !== loadId.current) return;
      }
      if (owner !== generation.current || request !== loadId.current) return;
      loadedPages.current = more ? loadedPages.current + 1 : pages;
      setHits((previous) => more ? { ...previous, ...newHits } : newHits);
      setPage((previous) => more && previous ? { ...result, conversations: [...previous.conversations, ...result.conversations.filter((item) => !previous.conversations.some((old) => old.id === item.id))] } : result);
      // Publish the authoritative page without waiting for the selected row:
      // its metadata can be slow or unavailable even when the catalog is ready.
      // A supplement is valid only for the same selection and view, and a
      // catalog refresh must not invalidate an otherwise current lookup.
      const selected = selection.current.selectedId;
      const selectedInternal = internalView.current;
      if (!termRef.current && selected && selected !== selection.current.pendingConversationId
        && !result.conversations.some(item => item.id === selected) && metadata) {
        let lookup = selectedLookup.current;
        if (!lookup || lookup.id !== selected || lookup.view !== view || lookup.internal !== selectedInternal || lookup.metadata !== metadata) {
          lookup = {id: selected, view, internal: selectedInternal, metadata, pending: false};
          selectedLookup.current = lookup;
        }
        const current = lookup;
        const isCurrent = (): boolean => owner === generation.current && selectedLookup.current === current
          && selection.current.selectedId === selected && selection.current.pendingConversationId !== selected
          && !termRef.current && archiveView.current === view && internalView.current === selectedInternal;
        const publish = (row: AgentConversationSummary, previousValue?: AgentConversationSummary): void => {
          if (!isCurrent() || row.id !== selected || row.archived !== view || Boolean(row.internal) !== selectedInternal) return;
          setPage(previous => {
            if (!previous) return previous;
            const existing = previous.conversations.find(item => item.id === selected);
            // An authoritative page or local mutation wins over an older lookup.
            if (existing && existing !== previousValue) return previous;
            return { ...previous, conversations: existing ? previous.conversations.map(item => item === existing ? row : item) : [row, ...previous.conversations] };
          });
        };
        // Retain a supplemented row while refreshing, then recheck it so that
        // external archive or deletion does not leave it visible indefinitely.
        if (current.value) publish(current.value);
        if (!current.pending) {
          current.pending = true;
          const previousValue = current.value;
          void Promise.resolve().then(() => metadata.read({ id: selected, ...(binding ? {binding} : {}) })).then(row => {
            if (!isCurrent()) return;
            current.pending = false;
            if (row.id !== selected || row.archived !== view || Boolean(row.internal) !== selectedInternal) {
              delete current.value;
              if (previousValue) setPage(previous => previous ? {...previous, conversations: previous.conversations.filter(item => item !== previousValue)} : previous);
              return;
            }
            current.value = row; publish(row, previousValue);
          }).catch(() => {
            // The selected identity may be unavailable; keep the loaded page.
            if (isCurrent()) {
              current.pending = false; selectedLookup.current = null;
              if (previousValue) setPage(previous => previous ? {...previous, conversations: previous.conversations.filter(item => item !== previousValue)} : previous);
            }
          });
        }
      }
    } catch {
      if (owner !== generation.current || request !== loadId.current) return;
      setFailed(true);
      notify.error("无法加载对话列表，请重试。");
    } finally {
      if (owner === generation.current && request === loadId.current) {
        loadingRequest.current = false; setLoading(false);
        if (syncPending.current) { syncPending.current = false; queueMicrotask(() => { if (owner === generation.current) synchronize.current(); }); }
      }
    }
  }
  synchronize.current = () => { void load(false, archiveView.current, true); };
  useConversationCatalogSync(() => synchronize.current());
  useEffect(() => {
    void load();
    return () => { generation.current += 1; };
    // The keyed host owns this component. The gateway and binding cannot change in place.
  }, []);
  useEffect(() => { if (refreshToken > 0) synchronize.current(); }, [refreshToken]);
  useEffect(() => {
    if (!revealConversation) return;
    termRef.current = ""; setTerm(""); setSearchDraft(""); setHits({});
    archiveView.current = revealConversation.archived; setArchived(revealConversation.archived); setPage(null);
    internalView.current = Boolean(revealConversation.internal); setInternal(internalView.current);
    void load(false, revealConversation.archived);
    // An explicit open, not background metadata, owns this filter reset. No focus or selection effects.
  }, [revealConversation]);
  function searchFor(value: string): void {
    termRef.current = value; setTerm(value); setPage(null); setHits({});
    void load();
  }
  function renamed(conversation: AgentConversationSummary): void {
    if (selectedLookup.current?.id === conversation.id) selectedLookup.current = null;
    loadId.current += 1;
    loadingRequest.current = false;
    setLoading(false);
    setPage((previous) => previous ? { ...previous, conversations: previous.conversations.map((item) => item.id === conversation.id ? conversation : item) } : previous);
    if (conversation.id === selectedId) onRefreshSelected?.();
    if (termRef.current) synchronize.current();
  }
  function changeView(view: boolean): void {
    archiveView.current = view; setArchived(view); setPage(null);
    void load(false, view);
    requestAnimationFrame(() => (view ? currentFilter : internalFilter).current?.focus());
  }
  function changeCatalog(value: boolean): void {
    internalView.current = value; setInternal(value);
    archiveView.current = false; setArchived(false);
    termRef.current = ""; setTerm(""); setSearchDraft(""); setHits({}); setPage(null);
    setContextConversation(null); loadedPages.current = 1;
    void load(false, false);
    requestAnimationFrame(() => internalFilter.current?.focus());
  }
  function closeSearch(): void {
    setSearchOpen(false); setSearchDraft("");
    if (termRef.current) searchFor("");
    searchTrigger.current?.focus();
  }
  function create(): void {
    if (archiveView.current || internalView.current) return;
    if (onCreate) { onCreate(); return; }
    // A new conversation is a local draft until its first non-empty message is
    // sent. Repeated clicks keep selecting the same draft and never create
    // empty durable rows in the catalog.
    let id: string;
    try {
      if (binding && selectedId && pendingConversationId !== selectedId && readBoundCreation(binding)?.intent.id === selectedId) {
        notify.error("当前创建恢复记录尚未清理，请先处理后再新建对话。"); return;
      }
      id = binding ? reserveBoundConversation(binding) : pendingIdRef.current ?? pendingId ?? globalThis.crypto.randomUUID();
    }
    catch (cause: unknown) { notify.error(conversationCreationMessage(cause)); return; }
    pendingIdRef.current = id;
    setPendingId(id);
    onSelect(id, false, undefined, true);
  }
  useEffect(() => {
    // A host-owned pending identity survives list remounts. An omitted/null
    // value means this list owns the draft locally; only an explicit different
    // pending ID (including null after creation) replaces it.
    if (pendingConversationId !== undefined && pendingIdRef.current !== pendingConversationId) {
      pendingIdRef.current = pendingConversationId;
      setPendingId(pendingConversationId);
    }
  }, [pendingConversationId, pendingId]);
  const archiveOperations = useConversationArchives(binding, { archives, metadata: metadata ?? agentMetadataGateway }, (result) => {
      if (selectedLookup.current?.id === result.id) selectedLookup.current = null;
      onArchiveChange?.(result.id, result.archived);
      setContextConversation(previous => previous?.id === result.id ? null : previous);
      loadId.current++; loadingRequest.current = false; setLoading(false);
      const active = document.activeElement;
      if (active === document.body || active?.closest('[data-conversation-id]')?.getAttribute('data-conversation-id') === result.id) {
        (archiveView.current ? currentFilter : archivedFilter).current?.focus();
      }
      setPage((previous) => previous ? { ...previous, conversations: previous.conversations
        .map((item) => item.id === result.id ? result : item).filter((item) => item.archived === archiveView.current) } : previous);
      void load(false, archiveView.current, true);
  }, (cause) => { if (cause === "conflict") synchronize.current(); });
  const deletionPending = (id: string) => deletionRecovery.records.some(record => record.intent.conversationId === id)
    || deletionRecovery.batches.some(batch => batch.requests.some(request => request.conversationId === id));
  const archivePending = (id: string) => deletionRecovery.storage !== null && !deletionRecovery.legacyReady || archiveOperations.pending.has(id.toLowerCase()) || deletionPending(id);
  const operationHint = (id: string, label: string) => deletionPending(id) ? "原删除请求尚待核对，请从原入口继续。" : label;
  const changeArchive = (conversation: AgentConversationSummary) => archiveOperations.change(conversation);
  const conversations = useMemo(() => (page?.conversations ?? []).slice().sort((left, right) => Number(pinnedIds.includes(right.id)) - Number(pinnedIds.includes(left.id))), [page?.conversations, pinnedIds]);
  const rowWindow = useConversationListWindow(conversations, section, contextConversation?.id ?? deleting?.id);
  const sentinel = useRef<HTMLButtonElement>(null), requested = useRef<string | undefined>(undefined);
  useEffect(() => {
    const cursor = page?.next;
    if (!cursor || loading || failed || !sentinel.current || typeof IntersectionObserver === "undefined") return;
    const identity = JSON.stringify([archiveView.current, internalView.current, termRef.current, cursor]);
    const observer = new IntersectionObserver(entries => {
      if (!entries.some(entry => entry.isIntersecting) || requested.current === identity || loadingRequest.current) return;
      requested.current = identity; void load(true);
    }, {root: section.current, rootMargin: "160px"});
    observer.observe(sentinel.current);
    return () => observer.disconnect();
  }, [page, loading, failed]);
  const contextTarget = contextConversation ?? conversations[0] ?? null;
  return <section ref={section} aria-label="对话列表" className="writing-conversation-list" aria-busy={loading}>
    <ConversationListToolbar>
      {archived && <ControlHint content="返回当前对话"><button type="button" ref={currentFilter} aria-label="返回当前对话" onClick={() => changeView(false)}><ArrowLeft aria-hidden="true" size={18}/></button></ControlHint>}
      {!archived && <ControlHint content={internal ? "返回普通对话" : internalRunning ? "内部对话正在运行" : "内部对话"}><button type="button" ref={internalFilter} aria-label={internal ? "返回普通对话" : "内部对话"} aria-busy={internalRunning} aria-description={internalRunning ? "内部对话正在运行" : undefined} aria-pressed={internal} onClick={() => changeCatalog(!internal)}>{internal ? <ArrowLeft aria-hidden="true" size={18}/> : <InternalConversationIcon active={internalRunning}/>}</button></ControlHint>}
      <span className="conversation-list-toolbar-spacer"/>
      {!archived && <ControlHint content="查看归档对话"><button type="button" ref={archivedFilter} aria-label="查看归档对话" onClick={() => changeView(true)}><Archive aria-hidden="true" size={18}/></button></ControlHint>}
      {archived && <ControlHint content="清空归档"><button type="button" ref={clearTrigger} aria-label="清空归档" disabled={!deletionRecovery.legacyReady || loading || Boolean(deleting) || Boolean(clearing) || archiveOperations.pending.size > 0 || deletionRecords.length > 0 || (!term && !failed && conversations.length === 0)} onClick={() => setClearing({binding, internal})}><BrushCleaning aria-hidden="true" size={18}/></button></ControlHint>}
      <ControlHint content="搜索对话"><button type="button" ref={searchTrigger} aria-label="搜索对话" aria-expanded={searchOpen} onClick={() => searchOpen ? closeSearch() : setSearchOpen(true)}><Search aria-hidden="true" size={18}/></button></ControlHint>
    </ConversationListToolbar>
    <ConversationDeletionRecoveries records={deletionRecords.filter(record => !deletionRecovery.storage || !conversationDeletionInProgress(record.intent.conversationId, deletions, deletionRecovery.storage))} issue={deletionRecovery.issue} activeId={deleting?.id ?? null} showOrigin={binding === null} onOpen={record => setDeleting({
      id: record.intent.conversationId, title: record.intent.expectedTitle, archived: true,
      durableCount: record.intent.expectedDurableCount, internal: record.internal, recoveryOnly: true, recoveryBinding: record.intent.binding,
    })} />
    {searchOpen && <ConversationSearchForm autoFocus showSubmit={false} onClose={closeSearch} value={searchDraft} onChange={setSearchDraft} onSearch={searchFor} label={binding ? {manuscript: "搜索当前稿件的对话", paper: "搜索当前文献的对话", note: "搜索当前笔记的对话", readingNote: "搜索当前阅读笔记的对话"}[binding.kind] : "搜索对话"} />}
    {internal && <p className="conversation-catalog-label">内部对话</p>}
    {!archived && !internal && <button type="button" aria-label="新对话" onClick={create}><SquarePen aria-hidden="true" size={18} /><span>新对话</span></button>}
    <div ref={rowWindow.body}>{rowWindow.rows.map(({item: conversation, gap}) => <Fragment key={conversation.id}>{gap > 0 && <div aria-hidden="true" style={{height: gap}} />}<div className={`conversation-list-row${selectedId === conversation.id ? " active" : ""}`} data-conversation-id={conversation.id}
      onContextMenu={(event) => { event.preventDefault(); setContextPosition({ x: event.clientX, y: event.clientY }); setContextConversation(conversation); }}>
      <button type="button" aria-current={selectedId === conversation.id ? "true" : undefined} className={selectedId === conversation.id ? "active" : ""} onClick={() => hits[conversation.id] ? onSelect(conversation.id, conversation.archived, hits[conversation.id]) : onSelect(conversation.id, conversation.archived)}><MessageCircle aria-hidden="true" size={17} /><span><strong>{conversation.title}</strong>{hits[conversation.id]?.itemId && <small className="conversation-search-excerpt">{hits[conversation.id]?.excerpt}</small>}</span></button>
      {running.some(row => row.id === conversation.id) && <span className="conversation-row-activity" role="status" aria-label="对话正在运行"><AgentActivitySpinner size={16}/></span>}
      <div className="conversation-row-actions">
        {archived ? <><ControlHint content={operationHint(conversation.id, "恢复对话")}><button type="button" aria-label="恢复对话" disabled={archivePending(conversation.id)} onClick={() => void changeArchive(conversation)}><ArchiveRestore aria-hidden="true" size={15} /></button></ControlHint><ControlHint content={operationHint(conversation.id, "永久删除")}><button type="button" aria-label="永久删除对话" disabled={archivePending(conversation.id)} onClick={() => setDeleting(conversation)}><Trash2 aria-hidden="true" size={15} /></button></ControlHint></> : <><ControlHint content={pinnedIds.includes(conversation.id) ? "取消置顶" : "置顶"}><button type="button" aria-label={pinnedIds.includes(conversation.id) ? "取消置顶对话" : "置顶对话"} aria-pressed={pinnedIds.includes(conversation.id)}  onClick={() => togglePin(conversation.id)}><Pin aria-hidden="true" fill={pinnedIds.includes(conversation.id) ? "currentColor" : "none"} size={15} /></button></ControlHint>
          <ControlHint content={operationHint(conversation.id, "归档")}><button type="button" aria-label="归档对话" disabled={archivePending(conversation.id)} onClick={() => void changeArchive(conversation)}><Archive aria-hidden="true" size={15} /></button></ControlHint></>}
      </div>
    </div></Fragment>)}{rowWindow.bottom > 0 && <div aria-hidden="true" style={{height: rowWindow.bottom}} />}</div>
    {loading && <p role="status">正在加载对话…</p>}
    {!loading && !failed && conversations.length === 0 && (term || archived) && <p>{term ? `未找到包含“${term}”的${archived ? "已归档" : "当前"}对话。` : "没有已归档的对话。"}</p>}
    {!loading && !failed && internal && !term && !archived && conversations.length === 0 && <p>暂无内部对话。</p>}
    {failed && <button type="button" onClick={() => void load()}><RefreshCw aria-hidden="true" size={17} /><span>重新加载对话</span></button>}
    {page?.next && !failed && <button ref={sentinel} type="button" disabled={loading} onClick={() => void load(true)}><ChevronDown aria-hidden="true" size={17} /><span>加载更多</span></button>}
    {contextTarget && <div className="conversation-context-menu-anchor" style={{ left: contextPosition.x, top: contextPosition.y, visibility: contextConversation ? "visible" : "hidden" }}>
      <ConversationMenu conversationId={contextTarget.id} title={contextTarget.title} contextMenuTrigger triggerRef={contextTrigger}
        {...(exports ? { gateway: exports } : {})}
        catalog={catalog} {...(metadata ? { metadata } : {})} onRenamed={renamed} onConflict={() => synchronize.current()} archived={contextTarget.archived} archivePending={archivePending(contextTarget.id)}
        onArchive={() => changeArchive(contextTarget)} onDelete={() => setDeleting(contextTarget)} deleting={deleting?.id === contextTarget.id} />
    </div>}
    {deleting && <ConversationDeleteDialog key={deleting.id} conversation={deleting} binding={deleting.recoveryOnly ? deleting.recoveryBinding ?? null : binding} gateway={deletions}
      viewIdentity={JSON.stringify([binding, internal])}
      {...(deleting.recoveryOnly ? {recoveryOnly: true} : {})}
      {...(deletionRecoveryStorage ? {recoveryStorage: deletionRecoveryStorage} : {})}
      {...(deletionRecoveryLock ? {recoveryLock: deletionRecoveryLock} : {})}
      {...(forceDeletionConfirmationPreferences ? { forceConfirmationPreferences: forceDeletionConfirmationPreferences } : {})} {...(deletionConfirmationPreferences ? { confirmationPreferences: deletionConfirmationPreferences } : {})} onClose={closeDelete} onDeleted={deleted} onConfirmed={confirmedDeletion} />}
    {clearing && <ConversationArchiveClearDialog binding={clearing.binding} internal={clearing.internal} catalog={catalog} deletions={batchDeletions}
      viewIdentity={JSON.stringify([binding, internal])}
      onClose={closeClear} onDeleted={cleared} onConfirmed={confirmedDeletion} flushCleanup={flushDeletionCleanup} />}
  </section>;
}
