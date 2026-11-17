import { WorkbenchLayout } from "../core/public/workbench";
import { AgentConversationAdaptersProvider } from "../core/public/agentConversationAdapters";
import { ReadingNotePresentationProvider } from "../core/public/notePresentation";
import {CoreNoteWorkspace, CoreNoteWorkspaceHost} from "../core/public/noteDocuments";
import type {WorkbenchComposition} from "./workbenchComposition";
import type {RuntimePluginWorkbench} from "./runtime/plugins/runtimePluginWorkbench";
import type {RegisteredPluginView} from "../core/runtime/plugins/viewRegistry";
import {RuntimePluginViewBoundary} from "./runtime/plugins/RuntimePluginViewBoundary";
import {useRuntimeWritingRole, type RuntimeWritingRole} from "./runtime/plugins/runtimeWritingRole";
import {libraryWorkspaceContribution, sourceDomainProvider} from "./runtime/plugins/sourceWorkspaceContribution";
import {workbenchNavigationItems} from "./navigation/workbenchNavigation";
import type {PluginWorkspaceHostProps, PluginWritingWorkspaceHostProps} from "@research-assistant/workbench-sdk/activation";

import { ControlHint } from "../core/public/workbench";
import { getAppRuntime } from "../core/runtime/appRuntime";
import { storageBackend } from "../core/runtime/storagePersistence";
import {readWorkspaceState, useWorkspaceState, workspaceBoolean} from "../core/runtime/workspaceState";
import {loadNoteTabs} from "../core/features/notes/workspace/noteTabSession";
import { WorkspaceTabsProvider, WorkspaceTabBar } from "../core/public/workbench";
import { WorkspacePresentationProvider, type WorkspacePresentation } from "../core/public/workbench";
import { useNoteDocumentTabHost } from "../core/features/notes/workspace/useNoteDocumentTabHost";
import { useTextDocuments } from "../core/components/text-editor/useTextDocuments";
import { SettingsWorkspace } from "../core/features/settings/SettingsWorkspace";
import {createPluginManagementGateway} from "../core/services/pluginManagementGateway";
import {coreCapabilities} from "./coreCapabilities";
import { notify } from "../core/public/feedback";
import { ReadingNoteFloatingPanel } from "./notes/ReadingNoteFloatingPanel";
import { AppWindowToolbar } from "../core/public/workbench";
import { useWorkspaceNavigation, type WorkspaceView, type WorkspaceLocation } from "../core/useWorkspaceNavigation";
import { NotesPreviewControl } from "../core/components/NotesPreviewControl";
import {NoteRecoveryPreviewControl} from "../core/components/NoteRecoveryPreviewControl";
import { PreviewResetControl } from "../core/components/PreviewResetControl";
import { AgentCanvasShell } from "../core/features/agent/AgentCanvasShell";

import { Suspense, createRef, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { isDesktopRuntime } from "../core/runtime/platformCore";
import { installDesktopZoom } from "../core/runtime/desktopZoom";
import { AppUpdateControl } from "../core/components/AppUpdateControl";
import { noteBlockId, notePageId, type NoteLinkTarget, type ReadingNoteSource } from "@research-assistant/core/notes";

import { getNoteDocumentGateway } from "../core/public/documents";
import { NotesGatewayError } from "@research-assistant/data/notes";
import { AppToaster } from "../core/components/AppToaster";
import { allowsAppTextSelection } from "../core/uiSelectionPolicy";
import { installDesktopLifecycle } from "./desktopLifecycle";
import { getReaderCloseCoordinator } from "../core/public/workbench";
import { AiProviderSettings } from "../core/features/settings/AiProviderSettings";
interface AiSourceInspectionRequest {readonly source: ReadingNoteSource; readonly resolve: () => void; readonly reject: (reason: Error) => void;}
import { useSourceNavigationHost } from "./navigation/useSourceNavigationHost";
import {captureViewNavigationGuard} from "./navigation/viewNavigationGuard";
import type {WorkspaceClosePort as NoteWorkspacePort} from "../core/contracts/workbench";


const EMPTY_REGISTERED_VIEWS: readonly RegisteredPluginView[] = Object.freeze([]);
const emptyViews = () => EMPTY_REGISTERED_VIEWS;
const noViewSubscription = () => () => undefined;
const EMPTY_PLUGIN_STATUSES: ReturnType<RuntimePluginWorkbench["getSnapshot"]> = Object.freeze([]);
const emptyPluginStatuses = () => EMPTY_PLUGIN_STATUSES;
const initialLoadComplete = () => true;

function AppContent({composition, runtimePlugins, runtimeWriting, initialChatOpen = false, initialLocation}: {readonly composition: WorkbenchComposition; readonly runtimeWriting: RuntimeWritingRole; readonly runtimePlugins?: RuntimePluginWorkbench; readonly initialChatOpen?: boolean; readonly initialLocation?: WorkspaceLocation}): React.JSX.Element {
  const pluginManagement = useMemo(() => isDesktopRuntime() ? createPluginManagementGateway({
    coreCapabilities, readRuntime: runtimePlugins?.getSnapshot ?? emptyPluginStatuses,
    isInitialLoadComplete: runtimePlugins?.isInitialLoadComplete ?? initialLoadComplete,
    ...(runtimePlugins ? {subscribe: (listener: () => void) => {
      const runtime = runtimePlugins.subscribe(listener);
      const loaded = runtimePlugins.subscribeInitialLoad(listener);
      return () => {runtime(); loaded();};
    }} : {}),
  }) : undefined, [runtimePlugins]);
  const registeredViews = useSyncExternalStore(runtimePlugins?.views.subscribe ?? noViewSubscription,
    runtimePlugins?.views.getSnapshot ?? emptyViews);
  const libraryContribution = libraryWorkspaceContribution(registeredViews);
  const pluginStatuses = useSyncExternalStore(runtimePlugins?.subscribe ?? noViewSubscription,
    runtimePlugins?.getSnapshot ?? emptyPluginStatuses);
  const runtimeLoaded = useSyncExternalStore(runtimePlugins?.subscribeInitialLoad ?? noViewSubscription,
    runtimePlugins?.isInitialLoadComplete ?? initialLoadComplete);
  const presentationBlocked = (registered: RegisteredPluginView): boolean => pluginStatuses.some(status =>
    status.pluginId === registered.pluginId && (status.state === "deactivating" || status.state === "failed"));
  const [failedViews, setFailedViews] = useState<ReadonlySet<RegisteredPluginView>>(() => new Set());
  useEffect(() => {
    setFailedViews(previous => [...previous].every(registered => registeredViews.includes(registered))
      ? previous : new Set([...previous].filter(registered => registeredViews.includes(registered))));
  }, [registeredViews]);
  const NAV_ITEMS = workbenchNavigationItems(composition, registeredViews);
  const availableViews = [...registeredViews.map(view => view.id), "settings" as const];
  const registeredPorts = useRef(new Map<RegisteredPluginView, ReturnType<typeof createRef<NoteWorkspacePort>>>());
  for (const registered of registeredViews) if (!registeredPorts.current.has(registered)) registeredPorts.current.set(registered, createRef<NoteWorkspacePort>());
  useLayoutEffect(() => {
    for (const registered of registeredPorts.current.keys()) if (!registeredViews.includes(registered)) registeredPorts.current.delete(registered);
  }, [registeredViews]);
  useEffect(() => installDesktopZoom({onError: notify.error}), []);
  const libraryReady = pluginStatuses.some(status => status.pluginId === libraryContribution?.pluginId && status.state === "ready");
  const restore = libraryReady ? libraryContribution?.restore : undefined;
  const navigationPluginIds = [...new Set(pluginStatuses.map(status => status.pluginId))].sort();
  const fullProduct = ["literature_library", "note_manager", "writing_projects"].every(id => navigationPluginIds.includes(id));
  const navigation = useWorkspaceNavigation(initialLocation, restore, {availableViews,
    ...(runtimePlugins ? {deferredRestore: !runtimeLoaded} : {}),
    ...(fullProduct ? {} : {persistenceKey: `location:composition:${navigationPluginIds.join(",") || "core"}`})});
  const textNavigation = useRef<((owner: WorkspaceView, destination: string | undefined, ready: () => void, presentation?: WorkspacePresentation) => boolean) | undefined>(undefined);
  const textDocuments = useTextDocuments(navigation.view, (owner, ready) => textNavigation.current?.(owner, undefined, ready) ?? false);
  const [hasVisitedNotes, setHasVisitedNotes] = useState(() => navigation.view === "notes" || loadNoteTabs().length > 0);
  const [hasVisitedSettings, setHasVisitedSettings] = useState(navigation.view === "settings");
  const notesWorkspace = useRef<NoteWorkspacePort>(null);
  const historyOperation = useRef(false);
  const { view, workspaceView } = navigation;
  const currentLocation = useRef(navigation.location); currentLocation.current = navigation.location;
  const navigationRequest = useRef(0);
  const [isWritingFocused, setIsWritingFocused] = useState(false);
  const [isGlobalSidebarExpanded, setIsGlobalSidebarExpanded] = useWorkspaceState("sidebar",true,workspaceBoolean);
  const [windowTabHost, setWindowTabHost] = useState<HTMLDivElement | null>(null);
  const [isWritingSidebarExpanded, setIsWritingSidebarExpanded] = useWorkspaceState("writing-sidebar",false,workspaceBoolean);
  const [isWritingWorkbenchVisible, setIsWritingWorkbenchVisible] = useState(() => readWorkspaceState("writing-visible",false,workspaceBoolean));
  const usesWritingSidebar = workspaceView === "writing" && (isWritingWorkbenchVisible || navigation.location.presentation === "document");
  const isSidebarExpanded = usesWritingSidebar ? isWritingSidebarExpanded : isGlobalSidebarExpanded;
  const onWritingEditorVisibilityChange = useCallback((isWorkbenchVisible: boolean) => {
    setIsWritingWorkbenchVisible(isWorkbenchVisible);
  }, []);
  const writingOperation = useRef(false);
  const [writingBusy, setWritingBusy] = useState(false);
  const onWritingOperationBusyChange = useCallback((busy: boolean) => {
    writingOperation.current = busy; setWritingBusy(busy);
  }, []);
  const chooseView = useCallback((next: WorkspaceView, destination?: string, ready?: () => void, presentation: WorkspacePresentation = "workspace", cancelled?: () => void, signal?: AbortSignal): boolean => {
    if (signal?.aborted) { cancelled?.(); return false; }
    if (next !== "notes" && !availableViews.includes(next) && !runtimePlugins?.views.getSnapshot().some(view => view.id === next)) { notify.error(`plugin_unavailable：当前装配未提供 ${next} 工作区。原资源和恢复状态已保留。`); cancelled?.(); return false; }
    if (writingOperation.current && next !== "writing") {
      notify.error("稿件提案操作尚未结束，请完成后再切换工作区。"); return false;
    }
    const sequence = ++navigationRequest.current, origin = currentLocation.current;
    const targetIsCurrent = captureViewNavigationGuard({view: next, presentation,
      ...(runtimePlugins ? {registry: runtimePlugins.views} : {})});
    const proceed = () => {
      if (signal?.aborted || sequence !== navigationRequest.current || currentLocation.current !== origin || !targetIsCurrent()
        || (writingOperation.current && next !== "writing")) { cancelled?.(); return; }
      if (next === "notes") setHasVisitedNotes(true);
      if (next === "settings") setHasVisitedSettings(true);
      textDocuments.deactivate();
      textDocuments.forgetOwner(next);
      navigation.navigate(next, destination, presentation);
      ready?.();
    };
    if (origin.view !== next && (origin.view === "notes" || origin.view === "writing" || runtimePlugins?.views.getSnapshot().some(view => view.id === origin.view))) {
      void getReaderCloseCoordinator().prepareAll().then(saved => { if (saved) proceed(); else cancelled?.(); })
        .catch((reason: unknown) => { cancelled?.(); notify.error(reason instanceof Error ? reason.message : String(reason)); });
    } else proceed();
    return true;
  }, [navigation.navigate, textDocuments.deactivate, textDocuments.forgetOwner, composition, runtimePlugins, registeredViews]);
  textNavigation.current = chooseView;
  useLayoutEffect(() => runtimePlugins?.attachNavigation({
    open: (registered, signal) => new Promise<void>((resolve, reject) => {
      const cancelled = () => {
        signal?.removeEventListener("abort", cancelled);
        reject(signal?.reason ?? new Error(`plugin_navigation_cancelled: ${registered.pluginId} / ${registered.id}`));
      };
      const opened = () => { signal?.removeEventListener("abort", cancelled); resolve(); };
      signal?.addEventListener("abort", cancelled, {once: true});
      if (!chooseView(registered.id, undefined, opened, "workspace", cancelled, signal)) cancelled();
    }),
    prepareClose: async (registered, signal) => {
      const current = () => !signal?.aborted && runtimePlugins?.views.getSnapshot().includes(registered) === true;
      if (!current()) return false;
      if (registered.writing) {
        const binding = runtimeWriting.readBinding(registered);
        if (!binding || !await binding.prepareClose() || !current()) return false;
      }
      const port = registeredPorts.current.get(registered)?.current;
      if (port && !await port.prepareClose(true)) return false;
      if (!current()) return false;
      if ((registered.writing || registered === libraryWorkspaceContribution(runtimePlugins?.views.getSnapshot() ?? [])
        || currentLocation.current.view === registered.id) && !await getReaderCloseCoordinator().prepareAll()) return false;
      // Preparation never changes navigation or drops a ref. The registry
      // publishes final removal; existing navigation reconciliation handles it.
      return current();
    },
  }), [runtimePlugins, chooseView, runtimeWriting.readBinding]);
  const failedView = useCallback((registered: RegisteredPluginView) => {
    if (!runtimePlugins?.views.getSnapshot().includes(registered)) return;
    setFailedViews(previous => previous.has(registered) ? previous : new Set([...previous, registered]));
  }, [runtimePlugins]);
  const recoverView = useCallback(async (registered: RegisteredPluginView): Promise<void> => {
    const current = () => runtimePlugins?.views.getSnapshot().includes(registered) === true
      && runtimePlugins.getSnapshot().some(status => status.pluginId === registered.pluginId && status.state === "ready");
    const unavailable = () => new Error(`plugin_view_recovery_unavailable: ${registered.pluginId} / ${registered.id}`);
    if (!current()) throw unavailable();
    if (!await getReaderCloseCoordinator().prepareAll())
      throw new Error(`plugin_view_recovery_blocked: ${registered.pluginId} / ${registered.id}`);
    if (!current()) throw unavailable();
    await new Promise<void>((resolve, reject) => {
      const cancelled = () => reject(new Error(`plugin_navigation_cancelled: ${registered.pluginId} / ${registered.id}`));
      if (!chooseView(registered.id, undefined, resolve, "workspace", cancelled)) cancelled();
    });
    if (!current()) throw unavailable();
    setFailedViews(previous => new Set([...previous].filter(view => view !== registered)));
  }, [runtimePlugins, chooseView]);
  const [readingSource, setReadingSource] = useState<ReadingNoteSource>();
  const readingSourceSequence = useRef(0);
  const readingSourceOwner = useRef<object | undefined>(undefined);
  const pendingEvidenceNavigation = useRef<{
    readonly resolve: () => void; readonly reject: (reason: Error) => void;
    readonly token: number;
  } | undefined>(undefined);
  function showReadingSource(source: ReadingNoteSource): void {
    pendingEvidenceNavigation.current?.reject(new Error("已由新的来源定位请求取代"));
    pendingEvidenceNavigation.current = undefined;
    readingSourceOwner.current = sourceDomainProvider(composition, runtimePlugins?.views.getSnapshot() ?? [], runtimePlugins?.getSnapshot() ?? [], "literature_library");
    readingSourceSequence.current++; setReadingSource(source);
  }
  const librarySourceOwner = sourceDomainProvider(composition, registeredViews, pluginStatuses, "literature_library");
  useEffect(() => {
    const pending = pendingEvidenceNavigation.current;
    if (!readingSourceOwner.current || readingSourceOwner.current === librarySourceOwner) return;
    pendingEvidenceNavigation.current = undefined; readingSourceSequence.current++;
    pending?.reject(new Error("plugin_unavailable：文献管理插件已停止；来源与原资源已保留。"));
    readingSourceOwner.current = undefined;
    setReadingSource(undefined);
  }, [librarySourceOwner]);
  useEffect(() => () => {pendingEvidenceNavigation.current?.reject(new Error("来源导航界面已关闭")); readingSourceSequence.current++;}, []);
  const noteNavigationSequence = useRef(0);
  const [openNoteRequestKey, setOpenNoteRequestKey] = useState(0);
  const [pinnedNotePageId, setPinnedNotePageId] = useState<string>();
  const [openResearchTarget, setOpenResearchTarget] = useState<NoteLinkTarget>();
  const sourceNavigation = useSourceNavigationHost({composition, chooseView, research: setOpenResearchTarget, pdf: showReadingSource,
    domainProvider: domain => sourceDomainProvider(composition, registeredViews, pluginStatuses, domain,
      runtimeWriting.binding ? runtimeWriting.registration : undefined),
    ...(libraryContribution?.reading ? {reading: libraryContribution.reading} : {}),
    note: () => { setOpenNoteRequestKey(key => key + 1); } });
  useNoteDocumentTabHost({ chooseView, openPage: id => { setPinnedNotePageId(id); setOpenNoteRequestKey(key => key + 1); } });
  const [apiSettingsOpen, setApiSettingsOpen] = useState(false);
  const apiSettingsReturnFocus = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    let uninstall: (() => void) | undefined;
    let active = true;
    void installDesktopLifecycle({ onError: notify.error }).then((cleanup) => {
      if (active) uninstall = cleanup;
      else cleanup();
    }).catch((reason: unknown) => {
      if (active) notify.error(`初始化桌面关闭控制失败：${reason instanceof Error ? reason.message : String(reason)}`);
    });
    return () => {
      active = false;
      uninstall?.();
    };
  }, []);

  useEffect(() => {
    const suppressNativeContextMenu = (event: MouseEvent): void => event.preventDefault();
    const suppressChromeTextSelection = (event: Event): void => {
      if (!allowsAppTextSelection(event.target)) event.preventDefault();
    };
    document.addEventListener("contextmenu", suppressNativeContextMenu);
    document.addEventListener("selectstart", suppressChromeTextSelection);
    return () => {
      document.removeEventListener("contextmenu", suppressNativeContextMenu);
      document.removeEventListener("selectstart", suppressChromeTextSelection);
    };
  }, []);

  useEffect(() => {
    const inspectReadingSource = (event: Event): void => {
      const request = (event as CustomEvent<AiSourceInspectionRequest>).detail;
      event.preventDefault();
      const owner = sourceDomainProvider(composition, runtimePlugins?.views.getSnapshot() ?? [], runtimePlugins?.getSnapshot() ?? [], "literature_library");
      if (!owner) {request.reject(new Error("plugin_unavailable：文献管理插件未就绪；来源与原资源已保留。")); return;}
      if (!chooseView("library")) { request.reject(new Error("当前操作尚未结束，无法打开来源")); return; }
      showReadingSource(request.source);
      pendingEvidenceNavigation.current = {...request, token: readingSourceSequence.current};
    };
    const openReadingSource = (event: Event): void => {
      const source = (event as CustomEvent<ReadingNoteSource>).detail;
      if (!chooseView("library")) return;
      showReadingSource(source);
    };
    window.addEventListener("research-assistant:open-reading-source", openReadingSource);
    window.addEventListener("research-assistant:inspect-reading-source", inspectReadingSource);
    return () => {
      window.removeEventListener("research-assistant:open-reading-source", openReadingSource);
      window.removeEventListener("research-assistant:inspect-reading-source", inspectReadingSource);
    };
  }, [chooseView]);

  useEffect(() => {
    const openTarget = (event: Event): void => {
      const target = (event as CustomEvent<NoteLinkTarget>).detail;
      if (!(["paper", "manuscript"] as const).includes(target.kind as "paper" | "manuscript")) return;
      if (!chooseView(target.kind === "paper" ? "library" : "writing")) return;
      setOpenResearchTarget(target);
    };
    window.addEventListener("research-assistant:open-research-target", openTarget);
    return () => window.removeEventListener("research-assistant:open-research-target", openTarget);
  }, [chooseView]);

  useEffect(() => {
    const openNotes = (): void => {chooseView("notes");};
    window.addEventListener("research-assistant:open-notes", openNotes);
    return () => window.removeEventListener("research-assistant:open-notes", openNotes);
  }, [chooseView]);

  useEffect(() => {
    const openNote = (event: Event): void => {
      const detail = (event as CustomEvent<string | { readonly targetId: string; readonly workspace?: boolean; readonly pinned?: boolean; readonly onOpened?: () => void }>).detail;
      const id = typeof detail === "string" ? detail : detail.targetId;
      if (!id) return;
      const sequence = ++noteNavigationSequence.current;
      const openOrdinaryPage = (): void => {
        if (sequence !== noteNavigationSequence.current) return;
        chooseView("notes", id, () => {
          if (typeof detail !== "string") { if (detail.pinned) setPinnedNotePageId(id); detail.onOpened?.(); }
          setOpenNoteRequestKey(sequence);
        });
      };
      if (typeof detail !== "string" && detail.workspace) { openOrdinaryPage(); return; }
      void getNoteDocumentGateway().getReadingNote(noteBlockId(id)).then(source => {
        if (sequence !== noteNavigationSequence.current) return;
        // note_page references identify canonical pages; a colliding legacy
        // reading alias must never shadow an independent ordinary page.
        if (source.pageId === id) window.dispatchEvent(new CustomEvent("research-assistant:open-reading-note", { detail: source.id }));
        else openOrdinaryPage();
      }).catch((reason: unknown) => {
        if (sequence !== noteNavigationSequence.current) return;
        if (reason instanceof NotesGatewayError && reason.code === "not_found") {
          openOrdinaryPage();
        } else notify.error(reason instanceof Error ? reason.message : String(reason));
      });
    };
    window.addEventListener("research-assistant:open-note-page", openNote);
    return () => { window.removeEventListener("research-assistant:open-note-page", openNote); noteNavigationSequence.current++; };
  }, [chooseView]);

  async function navigateHistory(direction: "back" | "forward"): Promise<void> {
    const next = direction === "back" ? navigation.previous : navigation.next;
    if (historyOperation.current || next === undefined || (writingOperation.current && next !== "writing")) return;
    historyOperation.current = true;
    const sequence = ++navigationRequest.current, origin = currentLocation.current;
    try {
      if ((view === "notes" || view === "writing") && !await getReaderCloseCoordinator().prepareAll()) return;
      if (sequence !== navigationRequest.current || currentLocation.current !== origin || (writingOperation.current && next !== "writing")) return;
      if (next === "notes") setHasVisitedNotes(true);
      if (next === "settings") setHasVisitedSettings(true);
      textDocuments.deactivate();
      navigation[direction]();
    } finally { historyOperation.current = false; }
  }

  function closeApiSettings(): void {
    setApiSettingsOpen(false);
    window.setTimeout(() => apiSettingsReturnFocus.current?.focus(), 0);
  }

  function finishReadingSourceNavigation(opened: boolean, token: number): void {
    if (token !== readingSourceSequence.current) return;
    readingSourceSequence.current++;
    readingSourceOwner.current = undefined;
    setReadingSource(undefined);
    const pending = pendingEvidenceNavigation.current;
    pendingEvidenceNavigation.current = undefined;
    if (pending === undefined || pending.token !== token) return;
    if (opened) pending.resolve();
    else pending.reject(new Error("候选引用的附件版本已不可用；未记录来源核对"));
  }

  const readingSourceToken = readingSourceSequence.current;
  const finishReadingSource = (opened: boolean) => finishReadingSourceNavigation(opened, readingSourceToken);
  const libraryHost: PluginWorkspaceHostProps = {
    navigationTabId: navigation.location.view === "library" ? navigation.location.tabId : undefined,
    onNavigateTab: navigation.visitLibraryTab, onSessionsChanged: navigation.reconcileLibrary,
    tabHost: windowTabHost, project: undefined, openSource: readingSource, onSourceOpened: finishReadingSource,
    openItemId: openResearchTarget?.kind === "paper" ? openResearchTarget.id : undefined,
    onItemOpened: () => setOpenResearchTarget(undefined),
  };
  const writingHost: PluginWritingWorkspaceHostProps = {
    tabHost: windowTabHost, tabsActive: view === "writing" && textDocuments.activeId === undefined,
    onOperationBusyChange: onWritingOperationBusyChange, onEditorVisibilityChange: onWritingEditorVisibilityChange,
    onFocusModeChange: setIsWritingFocused, onManuscriptOpened: () => setOpenResearchTarget(undefined),
    openManuscriptId: openResearchTarget?.kind === "manuscript" ? openResearchTarget.id : undefined,
    sourceNavigation: sourceNavigation.writing,
  };
  const withCoreNoteHost = (content: ReactNode) => runtimePlugins ? <CoreNoteWorkspaceHost embedded={false}
    active={view === "notes" && textDocuments.activeId === undefined} ref={notesWorkspace}
    onHome={navigation.visitNotesHome} onNavigatePage={navigation.visitNotePage} onClosePage={navigation.reconcileNotes}
    {...(view === "notes" && navigation.notePageId ? {initialPageId: notePageId(navigation.notePageId)} : {})}
    {...(pinnedNotePageId ? {pinnedPageId: notePageId(pinnedNotePageId)} : {})}
    openRequestKey={openNoteRequestKey} sourceNavigation={sourceNavigation.note}>{content}</CoreNoteWorkspaceHost> : content;

  return (
    <>
      <AppToaster />
    <AgentCanvasShell independent defaultOpen={initialChatOpen} topInset={36} onChooseContext={() => chooseView("writing")}>
      <WorkbenchLayout sidebarExpanded={isSidebarExpanded} writingFocused={workspaceView === "writing" && isWritingFocused}
        toolbar={<AppWindowToolbar
        onTabHostChange={setWindowTabHost}
        tabHostActive
        tabs={<><WorkspaceTabBar owner={view} homeOwner={workspaceView} textActive={textDocuments.activeId !== undefined} navigate={chooseView} /></>}
        canGoBack={navigation.previous !== undefined && (!writingBusy || navigation.previous === "writing")}
        canGoForward={navigation.next !== undefined && (!writingBusy || navigation.next === "writing")}
        onBack={() => navigateHistory("back")} onForward={() => navigateHistory("forward")}
        sidebarExpanded={isSidebarExpanded} onToggleSidebar={() => {
          if (usesWritingSidebar) setIsWritingSidebarExpanded(expanded => !expanded);
          else setIsGlobalSidebarExpanded(expanded => !expanded);
        }}
      />}
        navigation={<>{NAV_ITEMS.map((item) => (
            <ControlHint content={item.label} key={item.id}><button
              className={workspaceView === item.id ? "nav-item active" : "nav-item"}
              aria-label={item.label}
              disabled={writingBusy && item.id !== "writing"}
              onClick={() => { chooseView(item.id, undefined, textDocuments.restoreOwner(item.id)); }}
              type="button"
            >
              <span aria-hidden="true">{item.icon}</span>
              {item.label}
            </button></ControlHint>
          ))}</>}
        status={<div className="sidebar-note">
            <span className="status-dot" />
            <div>
              <strong>{isDesktopRuntime() ? "本地桌面模式" : "浏览器预览模式"}</strong>
              <span>{isDesktopRuntime() ? "SQLite 数据库" : `${storageBackend(getAppRuntime().storage)} 适配器`}</span>
            </div>
          </div>}
        utilityActions={<AppUpdateControl/>}>
        {sourceNavigation.resourceDialog}
      <WorkspacePresentationProvider value={navigation.location.presentation ?? "workspace"}>{withCoreNoteHost(<main className={`main-area${view === "library" ? " library-main-area" : view === "writing" ? " writing-main-area" : view === "notes" ? " notes-main-area" : registeredViews.some(registered => registered.id === view) ? " registered-main-area" : " settings-main-area"}`}>
        {pluginStatuses.filter(status => (status.state === "deactivating" || status.state === "failed")
          && registeredViews.some(registered => registered.pluginId === status.pluginId && registered.id === view)).map(status =>
          <section className="inline-error" role={status.state === "failed" ? "alert" : "status"} key={status.pluginId}>
            <p>{status.state === "deactivating" ? "正在停用管理视图，Core 文档和后台工作继续保留。" : "此管理视图未能安全停用，原资源已保留。"}</p>
            {status.error && <p>{status.error.message}</p>}
          </section>)}
        {!registeredViews.some(registered => registered.id === "notes" && !presentationBlocked(registered) && !failedViews.has(registered)) && (view === "notes" || hasVisitedNotes) && (
          <div className="notes-workspace-host" hidden={view !== "notes" || textDocuments.activeId !== undefined}>
          <Suspense fallback={<section className="coming-section"><p>正在载入笔记工作区…</p></section>}>
            <CoreNoteWorkspace embedded={false} active={view === "notes" && textDocuments.activeId === undefined} ref={notesWorkspace} onHome={navigation.visitNotesHome} onNavigatePage={navigation.visitNotePage} onClosePage={navigation.reconcileNotes} {...(navigation.notePageId ? {initialPageId: notePageId(navigation.notePageId)} : {})} openRequestKey={openNoteRequestKey} sourceNavigation={sourceNavigation.note}/>
          </Suspense>
          </div>
        )}

        {registeredViews.map(registered => {
          const active = view === registered.id && textDocuments.activeId === undefined && !presentationBlocked(registered);
          const Scope = registered === runtimeWriting.registration ? runtimeWriting.binding?.Scope : undefined;
          const content = <RuntimePluginViewBoundary registration={registered}
            viewProps={{active, ...(registered === libraryContribution ? {workspace: libraryHost} : {}),
              ...(registered.writing ? {writingWorkspace: {...writingHost, tabsActive: active}} : {}),
              ref: registeredPorts.current.get(registered)!}} onFailure={failedView} onRecover={recoverView}/>;
          return <div className="registered-workspace-host" key={registered.key}
            data-view-failed={failedViews.has(registered) || undefined}
            hidden={view !== registered.id || textDocuments.activeId !== undefined || presentationBlocked(registered)} inert={presentationBlocked(registered)}>
            {registered.writing ? Scope ? <Scope>{content}</Scope> : <p role="status">正在准备写作文件拥有者…</p> : content}
          </div>;
        })}

        {textDocuments.surface}
        {(view === "settings" || hasVisitedSettings) && <div className="settings-workspace-host" hidden={view !== "settings" || textDocuments.activeId !== undefined}>
          <SettingsWorkspace {...(pluginManagement ? {pluginManagement} : {})} active={view === "settings" && textDocuments.activeId === undefined} browserCaptureAvailable={libraryReady} onOpenApiSettings={trigger => { apiSettingsReturnFocus.current = trigger; setApiSettingsOpen(true); }} />
        </div>}
      </main>)}</WorkspacePresentationProvider>
      </WorkbenchLayout>
      <ReadingNoteFloatingPanel />
      </AgentCanvasShell>
      <PreviewResetControl />
      <NotesPreviewControl />
      <NoteRecoveryPreviewControl />
      {apiSettingsOpen && (
        <Suspense fallback={<div className="api-settings-backdrop"><div className="api-settings-dialog api-settings-loading" role="status">正在读取 AI API 设置…</div></div>}>
          <AiProviderSettings onClose={closeApiSettings} />
        </Suspense>
      )}
    </>
  );
}

export function WorkbenchApp({composition, ...props}: {readonly composition: WorkbenchComposition; readonly runtimePlugins?: RuntimePluginWorkbench; readonly initialChatOpen?: boolean; readonly initialLocation?: WorkspaceLocation}): React.JSX.Element {
  const views = useSyncExternalStore(props.runtimePlugins?.views.subscribe ?? noViewSubscription, props.runtimePlugins?.views.getSnapshot ?? emptyViews);
  const runtimeWriting = useRuntimeWritingRole(props.runtimePlugins, views);
  const WritingSession = runtimeWriting.Session;
  const ReadingActions = libraryWorkspaceContribution(views)?.reading?.ReadingActions;
  return <AgentConversationAdaptersProvider {...(WritingSession ? {WritingSession} : {})}><ReadingNotePresentationProvider {...(ReadingActions ? {ResearchActions: ReadingActions} : {})}><WorkspaceTabsProvider><AppContent composition={composition} runtimeWriting={runtimeWriting} {...props}/>{runtimeWriting.host}</WorkspaceTabsProvider></ReadingNotePresentationProvider></AgentConversationAdaptersProvider>;
}
