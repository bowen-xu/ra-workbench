import { notify } from "../../../services/notifications";
import type { AgentCanvasMessage } from "../AgentCanvasMessage";
import { useAgentContextCandidates } from "./AgentContextProvider";
import { selectAgentContext, EMPTY_AGENT_CONTEXT, agentContextKey } from "@research-assistant/core/agent";
import { Suspense, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import { agentContextGateway, type AgentContextGateway } from "../../../services/agentContextGateway";
import { agentArchiveGateway, agentCatalogGateway, agentMetadataGateway, agentSearchGateway, agentDeletionGateway, agentConversationExportGateway } from "../../../services/agentGateway";
import { getAppStorage } from "../../../runtime/appStorage";
import { WritingAgentPanel } from "../presentation/WritingAgentPanel";
import { BoundConversationList } from "../BoundConversationList";
import { IndependentAgentSession } from "./IndependentAgentSession";
import { contextDirectoryGateway, type AgentContextDirectory } from "./contextDirectoryGateway";
import { contextStorageKey, writeConversationContext } from "./contextStorage";
import { CONVERSATION_TITLE_CHANGED_EVENT } from "../ConversationHeaderTitle";
import { recommendationDraftKey, recommendationSubmissionKey } from "./contextRecommendationDraft";
import { useWritingConversationSession } from "./AgentConversationAdapters";
/** One catalog and identity for every conversation; execution context selects adapters. */
export function IndependentAgentWorkspace({ messageRequest, draftRequest = 0, open, onOpenChange, headerControls, drawer, gateway = agentContextGateway, workspaceActive = false, navigationPending = false, navigationRevision = 0, externalId, onSelectionStarted, onSelectContext, onArchiveContext, onDeleteContext }: {
  readonly messageRequest?: AgentCanvasMessage | null;
  readonly draftRequest?: number;
  readonly open: boolean; readonly onOpenChange: (value: boolean) => void;
  readonly headerControls: ReactNode; readonly drawer: (content: ReactNode) => ReactNode;
  readonly gateway?: AgentContextGateway; readonly workspaceActive?: boolean; readonly navigationPending?: boolean; readonly externalId?: string | null;
  readonly navigationRevision?: number;
  readonly onSelectionStarted?: () => void;
  readonly onSelectContext?: (id: string, directory: AgentContextDirectory | null, archived: boolean) => void;
  readonly onArchiveContext?: (id: string, archived: boolean) => void;
  readonly onDeleteContext?: (id: string) => void;
}): React.JSX.Element {
  const WritingConversationSession = useWritingConversationSession();

  const candidates=useAgentContextCandidates();
  const storage = getAppStorage(), key = "research-assistant:independent-selected";
  const [handledDraftRequest, setHandledDraftRequest] = useState(0);
  const [id, setId] = useState<string | null>(null), [selected, setSelected] = useState<AgentConversationSummary | null>(null);
  // Resolve the execution adapter without opening a resource page.
  const [selectedDirectory, setSelectedDirectory] = useState<{id:string; directory:AgentContextDirectory|null} | null>(null);
  useEffect(() => {
    if (messageRequest && selectedDirectory?.id === id && selectedDirectory.directory && !WritingConversationSession) {
      messageRequest.reject(Object.assign(new Error("当前工作台尚未接入写作对话。"), {code:"agent_execution_adapter_unavailable"}));
    }
  }, [messageRequest, selectedDirectory, id, WritingConversationSession]);
  const pendingKey="research-assistant:conversation-pending";
  const [pending,setPending]=useState(false);
  const [restoring,setRestoring]=useState(true);
  const [restoreId, setRestoreId] = useState<string | null>(null);
  const persistence=useRef<{id:string;promise:Promise<void>}|null>(null);
  const [busy, setBusy] = useState(false), [creatingBusy, setCreatingBusy] = useState(false), [sessionBusy, setSessionBusy] = useState(false), [refresh, setRefresh] = useState(0), [error, setError] = useState<string | undefined>(undefined);
  const alive = useRef(true), generation = useRef(0), creating = useRef(false), selectionInFlight = useRef(false);
  const previousNavigationRevision = useRef(navigationRevision);
  useLayoutEffect(() => {
    if (previousNavigationRevision.current === navigationRevision) return;
    previousNavigationRevision.current = navigationRevision;
    // A newer canvas navigation owns the UI even if old metadata reads return
    // later. This invalidates presentation only; it does not stop Agent work.
    generation.current += 1; selectionInFlight.current = false; setBusy(false);
  }, [navigationRevision]);
  const navigation = useRef({onSelectContext, onOpenChange}); navigation.current = {onSelectContext, onOpenChange};
  const retrySelection = useRef<{id:string; archived:boolean; reveal:boolean} | null>(null);
  useEffect(() => {
    if (!error || busy) return;
    const failedGeneration = generation.current;
    const retry = retrySelection.current;
    notify.error(error, {action: {label: "重新核对", onClick: () => {
      if (!alive.current || generation.current !== failedGeneration) return;
      if (retry) void select(retry.id,retry.archived,retry.reveal);
      else if (restoreId) void select(restoreId,false,false); else void create();
    }}});
  }, [error,busy,restoreId]);
  useEffect(() => {
    const refreshTitles = (): void => {setRefresh(value => value + 1);};
    window.addEventListener(CONVERSATION_TITLE_CHANGED_EVENT, refreshTitles);
    return () => window.removeEventListener(CONVERSATION_TITLE_CHANGED_EVENT, refreshTitles);
  }, []);
  async function select(next: string, archived = false, reveal = true): Promise<void> {
    if (creating.current || sessionBusy) return;
    // Both reads may finish after another click or host change. Only the latest
    // generation may publish the selection, error, or busy state.
    const request = ++generation.current;
    selectionInFlight.current = true;
    retrySelection.current = null;
    setBusy(true); setError(undefined);
    let metadataConfirmed = false;
    try {
      const metadata = await agentMetadataGateway.read({id:next});
      metadataConfirmed = true;
      if (!alive.current || generation.current !== request) return;
      const directory = await contextDirectoryGateway.read(next);
      if (!alive.current || generation.current !== request) return;
      storage.setItem(key,next); setId(next); setSelected(metadata);setSelectedDirectory({id:next,directory});setPending(false);
      if(reveal){navigation.current.onSelectContext?.(next,directory,metadata.archived ?? archived);navigation.current.onOpenChange(true);}
    } catch (reason) {
      if (!alive.current || generation.current !== request) return;
      if (!reveal && !metadataConfirmed && reason === "notFound") {
        // An obsolete selection is not an existing conversation. Keep its drafts
        // and receipts, clear only this pointer, then prepare a new local draft.
        if (storage.getItem(key) === next) storage.removeItem(key);
        setId(null); setSelected(null); setPending(false); setRestoreId(null);
      } else {
        retrySelection.current = {id:next, archived, reveal};
        setError("无法核对对话及上下文，请重试。");
      }
    }
    finally { if (alive.current && generation.current === request) {selectionInFlight.current = false; setBusy(false);} }
  }
  // Selection belongs to the storage identity, not the active manuscript's
  // execution gateway. Gateway replacement must not cancel a user selection.
  useEffect(() => { alive.current=true; const saved=storage.getItem(key); if(saved===storage.getItem(pendingKey)&&saved){setId(saved);setPending(true);setRestoring(false);}else if(saved) {setRestoreId(saved);void select(saved,false,false).finally(()=>{if(alive.current)setRestoring(false);});}else setRestoring(false); return()=>{alive.current=false;generation.current+=1;}; }, [storage]);
  useEffect(() => {if (navigationPending) {generation.current += 1; selectionInFlight.current = false; setBusy(false);}}, [navigationPending]);
  // Do not reselect the previous id while an explicit selection is in flight:
  // finishing an earlier host navigation could otherwise supersede the click.
  useEffect(()=>{if(open&&id&&!pending&&!selectionInFlight.current&&!workspaceActive&&!navigationPending&&!messageRequest)void select(id);},[open,id,pending,workspaceActive,navigationPending,messageRequest]);
  useEffect(()=>{if(!workspaceActive&&!navigationPending&&(open || messageRequest)&&!id&&!busy&&!restoring&&!error)void create(messageRequest?.options?.reveal !== false);},[open,messageRequest,id,busy,restoring,error,workspaceActive,navigationPending]);
  useEffect(() => {
    if (draftRequest <= 0) return;
    if (sessionBusy) { messageRequest?.reject(new Error("聊天窗正在处理请求，请稍后重试。")); setHandledDraftRequest(draftRequest); return; }
    void create(messageRequest?.options?.reveal !== false).then(() => { if (alive.current) setHandledDraftRequest(draftRequest); });
  }, [draftRequest]);
  useEffect(()=>{ if(externalId && externalId!==id) {const request=++generation.current;selectionInFlight.current=false;setBusy(false);setRestoreId(null);setError(undefined);setId(externalId);setPending(false);setSelected(null);storage.setItem(key,externalId);void agentMetadataGateway.read({id:externalId}).then(row=>{if(alive.current&&generation.current===request)setSelected(row);}).catch(()=>undefined);} },[externalId]);
  async function refreshSelected(): Promise<void> {
    if (!id) return;
    const current=id;
    try {const row=await agentMetadataGateway.read({id:current});if(alive.current && current===storage.getItem(key))setSelected(row);}catch{/* Deleted metadata is cleared by the mutation callback. */}
  }
  async function create(reveal = true): Promise<void> {
    if(creating.current||sessionBusy)return;
    creating.current=true;selectionInFlight.current=false;retrySelection.current=null;setCreatingBusy(true);setBusy(true);setError(undefined);
    try {const existing=storage.getItem(pendingKey),next=existing??crypto.randomUUID(),candidate=candidates[0];if(!existing){writeConversationContext(next,candidate?selectAgentContext(EMPTY_AGENT_CONTEXT,candidate,"persistent"):EMPTY_AGENT_CONTEXT);if(candidate)storage.setItem(`research-assistant:context-auto:${next}`,agentContextKey(candidate.resource));storage.setItem(pendingKey,next);}storage.setItem(key,next);generation.current+=1;setId(next);setSelected(null);setSelectedDirectory(null);setPending(true);if(reveal){onSelectContext?.(next,null,false);onOpenChange(true);}}
    catch(reason){if(alive.current)setError(reason instanceof Error?reason.message:String(reason));}
    finally{creating.current=false;if(alive.current){setCreatingBusy(false);setBusy(false);}}
  }
  async function ensureCreated():Promise<void>{
    // A context change may await this twice through the same render's callback,
    // with a directory move in between. The cleared marker is authoritative.
    if(!id||!pending||storage.getItem(pendingKey)!==id)return;
    if(persistence.current?.id===id)return persistence.current.promise;
    const current=id;
    const promise=(async()=>{await gateway.sessions.create(current);const row=await agentMetadataGateway.read({id:current});if(storage.getItem(pendingKey)===current)storage.removeItem(pendingKey);if(alive.current&&storage.getItem(key)===current){setPending(false);setSelected(row);setSelectedDirectory({id:current,directory:null});setRefresh(v=>v+1);}})();
    persistence.current={id:current,promise};
    try{await promise;}finally{if(persistence.current?.promise===promise)persistence.current=null;}
  }
  function archiveChanged(next:string, archived:boolean):void {
    if(next===id)setSelected(row=>row?{...row,archived}:row);
    onArchiveContext?.(next,archived);
  }
  function deleted(next:string):void {
    storage.removeItem(recommendationDraftKey(next));storage.removeItem(recommendationSubmissionKey(next));
    storage.removeItem(contextStorageKey(next));storage.removeItem(`research-assistant:independent-draft:${next}`);storage.removeItem(`research-assistant:independent-pending:${next}`);storage.removeItem(`research-assistant:independent-input:${next}`);storage.removeItem(`research-assistant:context-auto:${next}`);
    if(storage.getItem(key)===next)storage.removeItem(key);
    if(next===id){generation.current+=1;setId(null);setSelected(null);onSelectContext?.(next,null,false);}
    onDeleteContext?.(next);
  }
  useEffect(() => { if (messageRequest && error && !busy) messageRequest.reject(new Error(error)); }, [messageRequest, error, busy]);
  // Metadata/directory reads set busy but must not lock the list: another click
  // can supersede them. Creation and session-owned operations retain their guard.
  const list=<div inert={creatingBusy||sessionBusy}><BoundConversationList binding={null} selectedId={id} pendingConversationId={pending?id:null} onCreate={()=>void create()} onSelect={(next,archived)=>{onSelectionStarted?.();void select(next,archived);}} onArchiveChange={archiveChanged} onDeleted={deleted} onRefreshSelected={()=>void refreshSelected()} refreshToken={refresh} catalog={agentCatalogGateway} metadata={agentMetadataGateway} archives={agentArchiveGateway} search={agentSearchGateway} deletions={agentDeletionGateway} exports={agentConversationExportGateway}/></div>;
  const independentReady = id !== null && (pending || selectedDirectory?.id === id);
  return <>{!workspaceActive&&(id&&independentReady ? selectedDirectory?.id === id && selectedDirectory.directory
    ? WritingConversationSession ? <Suspense fallback={<WritingAgentPanel independentContext showLauncher={false} open={open} onOpenChange={onOpenChange} headerControls={headerControls} historyReady={false} inputDisabled isSubmitting submittingText="正在读取对话……"/>}><WritingConversationSession messageRequest={messageRequest ?? null} key={id} id={id} directory={selectedDirectory.directory} title={selected?.title??"对话"} archived={selected?.archived??false} open={open} onOpenChange={onOpenChange} headerControls={headerControls} onRenamed={row=>{setSelected(row);setRefresh(value=>value+1);}}/></Suspense> : <WritingAgentPanel independentContext showLauncher={false} open={open} onOpenChange={onOpenChange} headerControls={headerControls} conversationTitle={selected?.title??"对话"} historyReady inputDisabled disabledReason="当前工作台尚未接入写作对话"/>
    : <IndependentAgentSession messageRequest={handledDraftRequest === draftRequest ? messageRequest ?? null : null} key={id} pending={pending} ensureCreated={ensureCreated} id={id} onRenamed={row=>{if(alive.current && storage.getItem(key)===row.id)setSelected(row);if(alive.current)setRefresh(value=>value+1);}} onRefreshTitle={()=>void refreshSelected()} title={selected?.title??"新对话"} archived={selected?.archived??false} gateway={gateway} open={open} onOpenChange={onOpenChange} headerControls={headerControls} onBusyChange={setSessionBusy} onDirectoryChanged={directory=>{setSelectedDirectory({id,directory});onSelectContext?.(id,directory,false);}} onStarted={()=>{setRefresh(v=>v+1);void refreshSelected();}}/>:
    <WritingAgentPanel independentContext showLauncher={false} open={open} onOpenChange={onOpenChange} headerControls={headerControls} historyReady={!!error} inputDisabled isSubmitting={busy || restoring} submittingText="正在核对对话状态……" disabledReason="正在核对对话……"/>)}{drawer(list)}</>;
}
