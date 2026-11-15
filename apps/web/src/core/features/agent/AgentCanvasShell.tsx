import { createContext, useCallback, useContext, useId, useLayoutEffect, useEffect, useRef, useState, type ReactNode } from "react";
import {useWorkspaceState,workspaceBoolean} from "../../runtime/workspaceState";
import { createPortal } from "react-dom";
import { List, Minus, SquarePen } from "lucide-react";
import { ControlHint } from "../../components/ControlHint";
import { TEXT_DOCUMENT_REVEALED } from "../../components/text-editor/textDocumentEvents";
import { WritingAgentPanel } from "./presentation/WritingAgentPanel";
import { ShortcutPreferencesProvider, useShortcutPreferences } from "../settings/ShortcutPreferences";
import { formatShortcut } from "../settings/keyboardShortcuts";
import { useAgentCanvasShortcuts } from "./useAgentCanvasShortcuts";
import { AgentContextProvider } from "./context/AgentContextProvider";
import { IndependentAgentWorkspace } from "./context/IndependentAgentWorkspace";
import "./AgentCanvasShell.css";
import type { AgentContextDirectory } from "./context/contextDirectoryGateway";
import type { AgentCanvasMessage, AgentCanvasMessageOptions } from "./AgentCanvasMessage";
import {ConversationDrawerHeader} from "./ConversationDrawerHeader";
import { reconcileEvidenceDrafts } from "./evidenceImageGateway";
import { AgentDraftReferenceCountProvider, AgentReferenceCountBadge } from "./AgentDraftReferenceCount";
import { OPEN_AGENT_CONVERSATION, type OpenAgentConversation } from "./openAgentConversation";
import { AgentActivityProvider, AgentActivityIcon, agentLauncherDescription } from "./AgentActivity";
import { OverlayScopeContext, useOverlayPanelScope, type OverlayScope } from "../../components/overlay/overlayScope";
import { AgentChatPinProvider, useAgentChatPin } from "./AgentChatPin";
import { AgentCanvasNavigationErrorProvider } from "./AgentCanvasNavigationError";
import { AgentCanvasNavigationGuardProvider } from "./AgentCanvasNavigationGuard";

type ExternalSubmission = (text: string, resources?: AgentCanvasMessage["resources"], options?: AgentCanvasMessageOptions) => Promise<void>;
const HostSubmissionContext = createContext<React.RefObject<ExternalSubmission | null> | null>(null);
export function useAgentCanvasExternalSubmission(handler: ExternalSubmission): void {
  const submission = useContext(HostSubmissionContext);
  const current = useRef(handler); current.current = handler;
  useLayoutEffect(() => {if (!submission) return; const run: ExternalSubmission = (...args) => current.current(...args); submission.current = run; return () => {if (submission.current === run) submission.current = null;};}, [submission]);
}

interface HostControl {
  readonly id: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onNewConversation: () => void;
  readonly sendMessage: ExternalSubmission;
  readonly referenceCount?: number;
  readonly workspaceId?: string;
  readonly selectedId?: string | null;
  readonly onSelect?: (id: string, archived: boolean) => void;
  readonly onArchive?: (id: string, archived: boolean) => void;
  readonly onDeleted?: (id: string) => void;
}
interface CanvasContext {
  readonly hostVisible: boolean;
  readonly ownsList: boolean;
  readonly newConversation: () => void;
  readonly listHeader: ReactNode;
  readonly target: HTMLElement | null;
  readonly overlayScope: OverlayScope | undefined;
  readonly listOpen: boolean;
  readonly listShortcut: string;
  readonly setListOpen: (open: boolean) => void;
  readonly register: (host: HostControl) => () => void;
  readonly updateOpen: (id: string, open: boolean) => void;
  readonly updateSelection: (id: string, selectedId: string | null | undefined) => void;
  readonly updateReferenceCount: (id: string, count: number) => void;
  readonly toggleRef: React.RefObject<HTMLButtonElement | null>;
}
const Context = createContext<CanvasContext | null>(null);
/** Retained hosts can suspend view-only subscriptions while another host is visible. */
export function useAgentCanvasHostVisible(): boolean { return useContext(Context)?.hostVisible ?? true; }
const MessageContext = createContext<ExternalSubmission | null>(null);
const unavailableMessage: ExternalSubmission = () => Promise.reject(new Error("聊天窗不可用，请打开应用后重试。"));
export function useAgentCanvasMessage(): ExternalSubmission {
  return useContext(MessageContext) ?? unavailableMessage;
}

/** Application-owned presentation; resource hosts retain their runtime and data. */
export function AgentCanvasShell(props: {
  readonly independent?: boolean;
  readonly independentGateway?: import("../../services/agentContextGateway").AgentContextGateway;
  readonly initialMode?: "writing" | "independent";
  readonly defaultOpen?: boolean;
  readonly children: ReactNode;
  readonly onChooseContext?: () => void;
  readonly navigationRevision?: number;
  readonly onNavigationIntent?: () => void;
  readonly topInset?: number;
  readonly placement?: "viewport" | "contained";
  readonly reconcileEvidence?: () => Promise<void>;
}): React.JSX.Element {
  return <AgentContextProvider><ShortcutPreferencesProvider><AgentChatPinProvider><AgentCanvasPresentation {...props} /></AgentChatPinProvider></ShortcutPreferencesProvider></AgentContextProvider>;
}
function AgentCanvasPresentation({ children, independent = false, independentGateway, initialMode, defaultOpen = false, onChooseContext, navigationRevision = 0, onNavigationIntent, topInset = 0, placement = "viewport", reconcileEvidence = reconcileEvidenceDrafts }: {
  readonly independent?: boolean;
  readonly independentGateway?: import("../../services/agentContextGateway").AgentContextGateway;
  readonly initialMode?: "writing" | "independent";
  readonly defaultOpen?: boolean;
  readonly children: ReactNode;
  readonly onChooseContext?: () => void;
  readonly navigationRevision?: number;
  readonly onNavigationIntent?: () => void;
  readonly topInset?: number;
  readonly placement?: "viewport" | "contained";
  readonly reconcileEvidence?: () => Promise<void>;
}): React.JSX.Element {
  const [target, setTarget] = useState<HTMLDivElement | null>(null);
  const [pinned] = useAgentChatPin();
  const overlayScope = useOverlayPanelScope(target);
  useEffect(()=>{void reconcileEvidence().catch(()=>{console.warn("图片草稿保留状态核对失败，已有快照继续保留。");});},[reconcileEvidence]);
  const [host, setHost] = useState<HostControl | null>(null);
  const [active, setActive] = useState(false);
  const [independentReferenceCount, setIndependentReferenceCount] = useState(0);
  const [emptyOpen, setEmptyOpen] = useWorkspaceState("chat-open",defaultOpen,workspaceBoolean,independent);
  const [mode, setMode] = useState<"independent" | "writing">(independent ? initialMode ?? "independent" : "writing");
  const modeRef = useRef(mode); modeRef.current = mode;
  const [requestedIndependent, setRequestedIndependent] = useState<string|null>(null);
  // A navigation owns its async results only until a newer user intent. The
  // guard controls presentation, not the underlying context or file save.
  const navigationEpoch = useRef(0);
  const captureNavigation = useCallback(() => {const epoch = navigationEpoch.current; return () => navigationEpoch.current === epoch;}, []);
  const previousNavigationRevision = useRef(navigationRevision);
  useLayoutEffect(() => {
    if (previousNavigationRevision.current === navigationRevision) return;
    previousNavigationRevision.current = navigationRevision;
    navigationEpoch.current += 1;
    setRequestedIndependent(null);
  }, [navigationRevision]);
  useEffect(()=>{const handle=(event:Event)=>{const detail=(event as CustomEvent<{id:string;directory:AgentContextDirectory|null}>).detail;
    // A retained writing host can finish saving after the user has switched
    // chats. Its directory remains saved, but must not navigate the new chat.
    if(modeRef.current!=="writing" || !host?.selectedId || host.selectedId!==detail.id)return;
    selectContext(detail.id,detail.directory,false);
  };window.addEventListener("research-assistant:agent-directory-changed",handle);return()=>window.removeEventListener("research-assistant:agent-directory-changed",handle);});
  function selectContext(id:string,_directory:AgentContextDirectory|null,_archived:boolean):void {
    navigationEpoch.current++;
    if(modeRef.current === "writing") host?.onOpenChange(false);
    setRequestedIndependent(id); setMode("independent"); setEmptyOpen(true);
  }
  useEffect(() => {
    const handle=(event:Event) => {
      const request=(event as CustomEvent<OpenAgentConversation>).detail;
      onNavigationIntent?.();
      event.preventDefault(); selectContext(request.id,null,false); request.resolve();
    };
    window.addEventListener(OPEN_AGENT_CONVERSATION,handle);
    return () => window.removeEventListener(OPEN_AGENT_CONVERSATION,handle);
  });
  const hostVisible = mode === "writing";
  const emptyOpenRef = useRef(emptyOpen); emptyOpenRef.current = emptyOpen;
  const [listOpen, setListOpen] = useWorkspaceState("chat-list-open",false,workspaceBoolean,independent);
  const [draftRequest, setDraftRequest] = useState(0);
  const [messageRequest, setMessageRequest] = useState<AgentCanvasMessage | null>(null);
  const pendingMessage = useRef<AgentCanvasMessage | null>(null);
  useEffect(() => () => pendingMessage.current?.reject(new Error("聊天窗已关闭，请重试。")), []);
  const launcher = useRef<HTMLButtonElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const open = hostVisible ? host?.open ?? emptyOpen : emptyOpen;
  const referenceCount = hostVisible ? host?.referenceCount ?? 0 : independentReferenceCount;
  const { bindings } = useShortcutPreferences();
  function setOpen(next: boolean): void { if (hostVisible && host) host.onOpenChange(next); else setEmptyOpen(next); }
  useEffect(() => {
    const revealDocument = () => { if (!pinned) setOpen(false); };
    window.addEventListener(TEXT_DOCUMENT_REVEALED, revealDocument);
    return () => window.removeEventListener(TEXT_DOCUMENT_REVEALED, revealDocument);
  });
  useAgentCanvasShortcuts(bindings, () => setOpen(!open), () => {
    if (!open) { setOpen(true); setListOpen(true); }
    else if (listOpen) closeList();
    else setListOpen(true);
  });
  const previousOpen = useRef(open);
  useLayoutEffect(() => {
    if (previousOpen.current && !open) {
      setListOpen(false);
      const focused = document.activeElement;
      if (!focused || focused === document.body || target?.contains(focused)) launcher.current?.focus();
    }
    previousOpen.current = open;
  }, [open, target]);
  const register = useCallback((next: HostControl) => {
      const transfer = (!independent || modeRef.current === "writing") && emptyOpenRef.current;
      setHost(transfer ? {...next,open:true} : next);
      if (!independent) setListOpen(false);
      if (transfer) { next.onOpenChange(true); setEmptyOpen(false); }
      return () => { setHost(current => current?.id === next.id ? null : current); if (!independent || modeRef.current === "writing") setListOpen(false); };
  }, [independent]);
  const updateOpen = useCallback((id: string, next: boolean) => {
    setHost(current => current?.id === id && current.open !== next ? { ...current, open: next } : current);
    if (next) setMode("writing");
  }, []);
  const updateSelection=useCallback((id:string,selectedId:string|null|undefined)=>{
    setHost(current=>current?.id===id&&current.selectedId!==selectedId?{...current,...(selectedId===undefined?{}:{selectedId})}:current);
  },[]);
  const updateReferenceCount = useCallback((id:string,count:number) => {
    setHost(current => current?.id === id && current.referenceCount !== count ? {...current,referenceCount:count} : current);
  }, []);
  function closeList(): void { setListOpen(false); toggleRef.current?.focus(); }
  function selectionStarted(): void {
    // Invalidate the previous owner at click time, before this selection's
    // metadata and directory reads can wait or resolve out of order.
    onNavigationIntent?.();
    navigationEpoch.current += 1;
  }
  function newConversation(): void {
    onNavigationIntent?.();
    if (!independent) { host?.onNewConversation(); return; }
    navigationEpoch.current += 1;
    setRequestedIndependent(null);
    setMode("independent"); setEmptyOpen(true); setDraftRequest(value => value + 1);
  }
  function sendMessage(text: string, resources?: AgentCanvasMessage["resources"], options?: AgentCanvasMessageOptions): Promise<void> {
    if (pendingMessage.current) return Promise.reject(new Error("聊天窗正在处理请求，请稍后重试。"));
    if (hostVisible && host && !options?.evidence?.length) {host.onOpenChange(true); return options ? host.sendMessage(text, resources, options) : host.sendMessage(text, resources);}
    if (!independent) return Promise.reject(new Error("当前对话暂时不可用，请重试。"));
    if (options?.reveal !== false) setEmptyOpen(true);
    else if (hostVisible) setEmptyOpen(open);
    if (options?.evidence?.length) {
      setMode("independent");
      // A writing-bound conversation cannot be prepared by the independent
      // gateway. Keep its draft/runtime and capture into a fresh pending chat.
      if (hostVisible) setDraftRequest(value => value + 1);
    }
    return new Promise<void>((resolve, reject) => {
      const finish = (settle: () => void): void => {
        if (pendingMessage.current !== request) return;
        pendingMessage.current = null; setMessageRequest(null); settle();
      };
      const request: AgentCanvasMessage = { text, ...(resources ? {resources} : {}), ...(options ? {options} : {}), resolve: () => finish(resolve), reject: reason => finish(() => reject(reason)) };
      pendingMessage.current = request;
      setMessageRequest(request);
    });
  }
  const latestMessage = useRef(sendMessage); latestMessage.current = sendMessage;
  const submitMessage = useCallback<ExternalSubmission>((...args) => latestMessage.current(...args), []);
  const context = { target, overlayScope, hostVisible, ownsList:independent, newConversation, listHeader:null, listOpen, listShortcut: formatShortcut(bindings.toggleConversations), setListOpen, toggleRef, register, updateOpen,updateSelection, updateReferenceCount };
  const chooseContext = <div className="agent-canvas-context-empty">
    <p>打开稿件以选择对话上下文。</p>
    {onChooseContext && <button type="button" onClick={onChooseContext}>选择稿件</button>}
  </div>;
  return <AgentCanvasNavigationGuardProvider capture={captureNavigation}><AgentCanvasNavigationErrorProvider error={null}><MessageContext.Provider value={submitMessage}><Context.Provider value={context}><AgentActivityProvider onChange={setActive}>
    <div className="agent-canvas-shell" data-placement={placement}>
    {children}
    <OverlayScopeContext value={overlayScope}><div className="agent-canvas-layer" ref={setTarget} style={{ top: topInset }}>
      {!open && <ControlHint content={active ? "正在处理消息，打开对话" : "打开对话"} shortcut={formatShortcut(bindings.toggleChat)}><button className="writing-agent-trigger" aria-label="打开对话" aria-expanded="false" aria-busy={active}
        aria-description={agentLauncherDescription(active, referenceCount)}
        onClick={() => setOpen(true)} ref={launcher} type="button"><AgentActivityIcon active={active}/><AgentReferenceCountBadge count={referenceCount}/></button></ControlHint>}
      {independent && <AgentDraftReferenceCountProvider onChange={setIndependentReferenceCount}><IndependentAgentWorkspace {...(independentGateway ? {gateway: independentGateway} : {})} messageRequest={messageRequest} draftRequest={draftRequest} open={open} workspaceActive={hostVisible} navigationRevision={navigationRevision} externalId={hostVisible ? host?.selectedId??null : requestedIndependent} onSelectionStarted={selectionStarted} onSelectContext={selectContext} onArchiveContext={(id,archived)=>host?.onArchive?.(id,archived)} onDeleteContext={id=>host?.onDeleted?.(id)} onOpenChange={setEmptyOpen} headerControls={<ConversationListToggle />} drawer={content => <ConversationDrawer open={open && listOpen} onClose={closeList}>{content}</ConversationDrawer>} /></AgentDraftReferenceCountProvider>}
      {hostVisible && !host && <>
        <WritingAgentPanel open={emptyOpen} onOpenChange={setEmptyOpen} showLauncher={false} headerControls={<ConversationListToggle />}
          historyReady inputDisabled disabledReason="请先选择稿件上下文" historyHeader={chooseContext} />
        <ConversationDrawer open={emptyOpen && listOpen} onClose={closeList}>{chooseContext}</ConversationDrawer>
      </>}
    </div></OverlayScopeContext>
    </div>
  </AgentActivityProvider></Context.Provider></MessageContext.Provider></AgentCanvasNavigationErrorProvider></AgentCanvasNavigationGuardProvider>;
}

function ConversationListToggle(): React.JSX.Element | null {
  const canvas = useContext(Context);
  if (!canvas) return null;
  return <>
    <ControlHint content="新建会话"><button type="button" className="agent-canvas-new-conversation" aria-label="新建会话" onClick={canvas.newConversation}><SquarePen aria-hidden="true" size={18}/></button></ControlHint>
    <ControlHint content={canvas.listOpen ? "隐藏对话列表" : "显示对话列表"} shortcut={canvas.listShortcut}><button className="agent-canvas-list-toggle"
    aria-label="对话列表" aria-expanded={canvas.listOpen} onClick={() => canvas.setListOpen(!canvas.listOpen)} ref={canvas.toggleRef} type="button">
    <List aria-hidden="true" size={18} />
  </button></ControlHint>
  <span className="agent-canvas-header-divider" aria-hidden="true"/>
  </>;
}

function ConversationDrawer({ open, onClose, children }: { readonly open: boolean; readonly onClose: () => void; readonly children: ReactNode }): React.JSX.Element | null {
  const { bindings } = useShortcutPreferences();
  const [visited, setVisited] = useState(open);
  useLayoutEffect(() => { if (open) setVisited(true); }, [open]);
  if (!visited && !open) return null;
  return <aside className="agent-canvas-conversations" aria-label="对话列表面板" hidden={!open}>
    <ConversationDrawerHeader closeButton={<ControlHint content="隐藏对话列表" shortcut={formatShortcut(bindings.toggleConversations)}><button aria-label="隐藏对话列表" type="button" onClick={onClose}><Minus aria-hidden="true" size={18} /></button></ControlHint>}>
      {children}
    </ConversationDrawerHeader>
  </aside>;
}

/** Portals do not detach the runtime from its file sessions or injected gateways. */
export function AgentCanvasHost({ open, onOpenChange, onNewConversation, conversations, children, workspaceId, selectedId, onSelect, onArchive, onDeleted, enabled = true }: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onNewConversation: () => void;
  readonly conversations: ReactNode;
  readonly workspaceId?: string;
  readonly selectedId?: string | null;
  readonly onSelect?: (id:string,archived:boolean)=>void;
  readonly onArchive?: (id:string,archived:boolean)=>void;
  readonly onDeleted?: (id:string)=>void;
  readonly enabled?: boolean;
  readonly children: (controls: { readonly headerControls: ReactNode; readonly showLauncher: boolean }) => ReactNode;
}): React.JSX.Element {
  const canvas = useContext(Context);
  const submission = useRef<ExternalSubmission | null>(null);
  const id = useId();
  const latestReferenceCount = useRef(0);
  const callback = useRef(onOpenChange); callback.current = onOpenChange;
  const actions=useRef({onNewConversation,onSelect,onArchive,onDeleted});actions.current={onNewConversation,onSelect,onArchive,onDeleted};
  const register = canvas?.register, updateOpen = canvas?.updateOpen,updateSelection=canvas?.updateSelection, updateReferenceCount=canvas?.updateReferenceCount;
  const publishReferenceCount = useCallback((count:number) => {latestReferenceCount.current=count; updateReferenceCount?.(id,count);}, [id,updateReferenceCount]);
  // Only the active resource host registers; hidden hosts keep their children
  // mounted so their file sessions and acknowledgement subscribers survive.
  useLayoutEffect(() => enabled ? register?.({ id, open, referenceCount:latestReferenceCount.current, onOpenChange: next => callback.current(next), onNewConversation:()=>actions.current.onNewConversation(), sendMessage:(...args)=>submission.current?.(...args) ?? Promise.reject(new Error("当前对话尚未就绪，请重试。")), ...(workspaceId ? {workspaceId} : {}), ...(selectedId !== undefined ? {selectedId} : {}), onSelect:(...args)=>actions.current.onSelect?.(...args),onArchive:(...args)=>actions.current.onArchive?.(...args),onDeleted:(...args)=>actions.current.onDeleted?.(...args) }) : undefined, [id, register, workspaceId, enabled]);
  useLayoutEffect(() => { if (enabled) updateOpen?.(id, open); }, [id, open, updateOpen, enabled]);
  useLayoutEffect(()=>{if (enabled) updateSelection?.(id,selectedId);},[id,selectedId,updateSelection,enabled]);
  if (canvas && !canvas.target) return <></>;
  const content = <div className="agent-canvas-host" hidden={!enabled || (canvas ? !canvas.hostVisible : false)} inert={!enabled}><AgentDraftReferenceCountProvider onChange={publishReferenceCount}><HostSubmissionContext.Provider value={submission}>{children({ headerControls: <ConversationListToggle />, showLauncher: !canvas })}</HostSubmissionContext.Provider></AgentDraftReferenceCountProvider>
    {canvas && !canvas.ownsList && <ConversationDrawer open={open && canvas.listOpen} onClose={() => { canvas.setListOpen(false); canvas.toggleRef.current?.focus(); }}>{canvas.listHeader}{conversations}</ConversationDrawer>}
  </div>;
  // React portals retain their source context. Bind resource-owned runtimes
  // to the destination scope too, so their menus cannot escape the chat.
  return <>{canvas?.target ? createPortal(<OverlayScopeContext value={canvas.overlayScope}>{content}</OverlayScopeContext>, canvas.target) : content}</>;
}
